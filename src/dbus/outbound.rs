use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::Result;
use zbus::zvariant::{OwnedValue, Value};
use zbus::Connection;

use super::connection::SystemConnectionCache;
use super::{
    extract_string, extract_u32, DBUS_PROPERTIES_INTERFACE, MM_DESTINATION, MM_MESSAGING_INTERFACE,
    MM_SMS_INTERFACE,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedSms {
    pub modem_sms_path: String,
}

#[derive(Debug)]
pub enum SendAttemptOutcome {
    Accepted,
    Rejected(anyhow::Error),
    NotAttempted(anyhow::Error),
    Unknown(anyhow::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModemSmsState {
    Stored,
    Sending,
    Sent,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmsSnapshot {
    pub state: ModemSmsState,
    pub phone_number: String,
    pub body: String,
}

pub trait SmsSender: Send + Sync {
    fn prepare<'a>(
        &'a self,
        modem_path: &'a str,
        tel_number: &'a str,
        sms_text: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<PreparedSms>> + Send + 'a>>;

    fn send_prepared<'a>(
        &'a self,
        modem_sms_path: &'a str,
    ) -> Pin<Box<dyn Future<Output = SendAttemptOutcome> + Send + 'a>>;

    fn sms_state<'a>(
        &'a self,
        modem_sms_path: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<ModemSmsState>> + Send + 'a>>;

    fn sms_snapshot<'a>(
        &'a self,
        _modem_path: Option<&'a str>,
        modem_sms_path: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<SmsSnapshot>> + Send + 'a>> {
        Box::pin(async move {
            Ok(SmsSnapshot {
                state: self.sms_state(modem_sms_path).await?,
                phone_number: String::new(),
                body: String::new(),
            })
        })
    }
}

#[derive(Clone, Default)]
pub struct SystemSmsSender {
    connection: SystemConnectionCache,
}

impl SystemSmsSender {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn connect() -> Result<Self> {
        Ok(Self {
            connection: SystemConnectionCache::connect().await?,
        })
    }
}

impl SmsSender for SystemSmsSender {
    fn prepare<'a>(
        &'a self,
        modem_path: &'a str,
        tel_number: &'a str,
        sms_text: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<PreparedSms>> + Send + 'a>> {
        Box::pin(async move {
            let connection = self.connection.get_or_connect().await?;
            let result = create_sms(&connection, modem_path, tel_number, sms_text).await;
            if result.is_err() {
                self.connection.discard_if_current(&connection).await;
            }
            result
        })
    }

    fn send_prepared<'a>(
        &'a self,
        modem_sms_path: &'a str,
    ) -> Pin<Box<dyn Future<Output = SendAttemptOutcome> + Send + 'a>> {
        Box::pin(async move {
            let connection = match self.connection.get_or_connect().await {
                Ok(connection) => connection,
                Err(error) => return SendAttemptOutcome::NotAttempted(error),
            };
            let outcome = send_prepared_sms(&connection, modem_sms_path).await;
            if !matches!(outcome, SendAttemptOutcome::Accepted) {
                self.connection.discard_if_current(&connection).await;
            }
            outcome
        })
    }

    fn sms_state<'a>(
        &'a self,
        modem_sms_path: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<ModemSmsState>> + Send + 'a>> {
        Box::pin(async move {
            let connection = self.connection.get_or_connect().await?;
            let result = get_sms_state(&connection, modem_sms_path).await;
            if result.is_err() {
                self.connection.discard_if_current(&connection).await;
            }
            result
        })
    }

    fn sms_snapshot<'a>(
        &'a self,
        modem_path: Option<&'a str>,
        modem_sms_path: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<SmsSnapshot>> + Send + 'a>> {
        Box::pin(async move {
            let connection = self.connection.get_or_connect().await?;
            let result = get_sms_snapshot(&connection, modem_path, modem_sms_path).await;
            if result.is_err() {
                self.connection.discard_if_current(&connection).await;
            }
            result
        })
    }
}

pub async fn create_sms(
    connection: &Connection,
    modem_path: &str,
    tel_number: &str,
    sms_text: &str,
) -> Result<PreparedSms> {
    let mut properties = HashMap::new();
    properties.insert("text", Value::from(sms_text));
    properties.insert("number", Value::from(tel_number));

    let create_args = (&properties,);
    let call = connection.call_method(
        Some(MM_DESTINATION),
        modem_path,
        Some(MM_MESSAGING_INTERFACE),
        "Create",
        &create_args,
    );
    let reply = tokio::time::timeout(Duration::from_secs(15), call)
        .await
        .map_err(|_| anyhow::anyhow!("dbus Create timeout"))??;

    let sms_path: zbus::zvariant::OwnedObjectPath = reply.body().deserialize()?;
    Ok(PreparedSms {
        modem_sms_path: sms_path.to_string(),
    })
}

pub async fn send_prepared_sms(
    connection: &Connection,
    modem_sms_path: &str,
) -> SendAttemptOutcome {
    let send_call = connection.call_method(
        Some(MM_DESTINATION),
        modem_sms_path,
        Some(MM_SMS_INTERFACE),
        "Send",
        &(),
    );
    match tokio::time::timeout(Duration::from_secs(30), send_call).await {
        Err(_) => SendAttemptOutcome::Unknown(anyhow::anyhow!("dbus Send timeout")),
        Ok(Ok(_)) => {
            println!("短信已发送");
            SendAttemptOutcome::Accepted
        }
        Ok(Err(error)) if is_explicit_send_rejection(&error) => {
            SendAttemptOutcome::Rejected(error.into())
        }
        Ok(Err(error)) => SendAttemptOutcome::Unknown(error.into()),
    }
}

fn is_explicit_send_rejection(error: &zbus::Error) -> bool {
    let zbus::Error::MethodError(name, _, _) = error else {
        return false;
    };
    is_explicit_send_rejection_name(name.as_str())
}

fn is_explicit_send_rejection_name(name: &str) -> bool {
    name.starts_with("org.freedesktop.ModemManager1.Error.")
        || matches!(
            name,
            "org.freedesktop.DBus.Error.AccessDenied"
                | "org.freedesktop.DBus.Error.InvalidArgs"
                | "org.freedesktop.DBus.Error.UnknownMethod"
                | "org.freedesktop.DBus.Error.UnknownObject"
                | "org.freedesktop.DBus.Error.UnknownInterface"
        )
}

pub async fn get_sms_state(connection: &Connection, modem_sms_path: &str) -> Result<ModemSmsState> {
    Ok(get_sms_snapshot(connection, None, modem_sms_path)
        .await?
        .state)
}

pub async fn get_sms_snapshot(
    connection: &Connection,
    modem_path: Option<&str>,
    modem_sms_path: &str,
) -> Result<SmsSnapshot> {
    if let Some(modem_path) = modem_path {
        let list_call = connection.call_method(
            Some(MM_DESTINATION),
            modem_path,
            Some(MM_MESSAGING_INTERFACE),
            "List",
            &(),
        );
        let list_reply = tokio::time::timeout(Duration::from_secs(5), list_call)
            .await
            .map_err(|_| anyhow::anyhow!("dbus SMS list timeout"))??;
        let paths: Vec<zbus::zvariant::OwnedObjectPath> = list_reply.body().deserialize()?;
        if !paths.iter().any(|path| path.as_str() == modem_sms_path) {
            return Ok(SmsSnapshot {
                state: ModemSmsState::Unknown,
                phone_number: String::new(),
                body: String::new(),
            });
        }
    }
    let call = connection.call_method(
        Some(MM_DESTINATION),
        modem_sms_path,
        Some(DBUS_PROPERTIES_INTERFACE),
        "GetAll",
        &(MM_SMS_INTERFACE,),
    );
    let reply = tokio::time::timeout(Duration::from_secs(5), call)
        .await
        .map_err(|_| anyhow::anyhow!("dbus SMS state timeout"))?;
    let reply = match reply {
        Ok(reply) => reply,
        Err(error) if is_missing_sms_object(&error) => {
            return Ok(SmsSnapshot {
                state: ModemSmsState::Unknown,
                phone_number: String::new(),
                body: String::new(),
            });
        }
        Err(error) => return Err(error.into()),
    };
    let properties: HashMap<String, OwnedValue> = reply.body().deserialize()?;
    Ok(SmsSnapshot {
        state: sms_state_from_raw(extract_u32(&properties, "State")),
        phone_number: extract_string(&properties, "Number"),
        body: extract_string(&properties, "Text"),
    })
}

fn is_missing_sms_object(error: &zbus::Error) -> bool {
    let zbus::Error::MethodError(name, _, _) = error else {
        return false;
    };
    is_missing_sms_object_error_name(name.as_str())
}

fn is_missing_sms_object_error_name(name: &str) -> bool {
    matches!(
        name,
        "org.freedesktop.DBus.Error.UnknownObject"
            | "org.freedesktop.ModemManager1.Error.Core.NotFound"
    )
}

fn sms_state_from_raw(state: u32) -> ModemSmsState {
    match state {
        1 => ModemSmsState::Stored,
        4 => ModemSmsState::Sending,
        5 => ModemSmsState::Sent,
        _ => ModemSmsState::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modem_sms_states_map_to_recovery_states() {
        assert_eq!(sms_state_from_raw(1), ModemSmsState::Stored);
        assert_eq!(sms_state_from_raw(4), ModemSmsState::Sending);
        assert_eq!(sms_state_from_raw(5), ModemSmsState::Sent);
        assert_eq!(sms_state_from_raw(0), ModemSmsState::Unknown);
        assert_eq!(sms_state_from_raw(6), ModemSmsState::Unknown);
    }

    #[test]
    fn missing_sms_object_errors_are_terminal_for_recovery() {
        assert!(is_missing_sms_object_error_name(
            "org.freedesktop.DBus.Error.UnknownObject"
        ));
        assert!(is_missing_sms_object_error_name(
            "org.freedesktop.ModemManager1.Error.Core.NotFound"
        ));
        assert!(!is_missing_sms_object_error_name(
            "org.freedesktop.DBus.Error.NoReply"
        ));
    }

    #[test]
    fn no_reply_is_unknown_but_modem_rejection_is_explicit() {
        assert!(!is_explicit_send_rejection_name(
            "org.freedesktop.DBus.Error.NoReply"
        ));
        assert!(!is_explicit_send_rejection_name(
            "org.freedesktop.DBus.Error.Disconnected"
        ));
        assert!(is_explicit_send_rejection_name(
            "org.freedesktop.ModemManager1.Error.Core.WrongState"
        ));
        assert!(is_explicit_send_rejection_name(
            "org.freedesktop.DBus.Error.AccessDenied"
        ));
    }

    #[test]
    fn system_sms_sender_defers_connecting_until_a_send_is_requested() {
        let sender = SystemSmsSender::new();
        assert!(sender.connection.is_empty());
    }

    #[test]
    fn system_sms_sender_clones_share_the_connection_cache() {
        let sender = SystemSmsSender::new();
        let cloned = sender.clone();

        assert!(sender.connection.shares_state_with(&cloned.connection));
    }
}
