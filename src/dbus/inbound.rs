use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use futures_util::{future::BoxFuture, StreamExt};
use zbus::names::OwnedUniqueName;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};
use zbus::{Connection, Message, MessageStream};

use super::{
    extract_string, extract_u32, DBUS_INTERFACE, DBUS_PROPERTIES_INTERFACE, MM_DESTINATION,
    MM_MESSAGING_INTERFACE, MM_SMS_INTERFACE, OBJECT_MANAGER_INTERFACE,
};

const DBUS_METHOD_TIMEOUT: Duration = Duration::from_secs(10);
const DBUS_PROPERTIES_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InboundSmsProperties {
    pub(crate) phone_number: String,
    pub(crate) body: String,
    pub(crate) timestamp: String,
    pub(crate) storage: u32,
}

#[derive(Clone)]
pub(crate) struct InboundSms {
    path: String,
    reader: Arc<dyn SmsPropertiesReader>,
}

impl std::fmt::Debug for InboundSms {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InboundSms")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl InboundSms {
    pub(crate) fn path(&self) -> &str {
        &self.path
    }

    pub(crate) async fn properties(&self) -> Result<InboundSmsProperties> {
        self.reader.read(&self.path).await
    }
}

#[derive(Debug)]
pub(crate) enum InboundEvent {
    Added(InboundSms),
}

#[derive(Default)]
pub(crate) struct SystemInboundSource;

impl SystemInboundSource {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) async fn subscribe(&self, modem_path: &str) -> Result<InboundSubscription> {
        let connection = Arc::new(Connection::system().await?);

        let owner_rule = format!(
            "type='signal',interface='{DBUS_INTERFACE}',member='NameOwnerChanged',arg0='{MM_DESTINATION}'"
        );
        add_match_rule(&connection, &owner_rule).await?;
        // Activate the receiver before resolving the owner so a change between
        // AddMatch and GetNameOwner cannot be dropped by zbus.
        let stream = MessageStream::from(connection.as_ref());

        let owner_call = connection.call_method(
            Some(DBUS_INTERFACE),
            "/org/freedesktop/DBus",
            Some(DBUS_INTERFACE),
            "GetNameOwner",
            &(MM_DESTINATION,),
        );
        let owner_reply = tokio::time::timeout(DBUS_METHOD_TIMEOUT, owner_call).await??;
        let owner_name: String = owner_reply.body().deserialize()?;
        let owner = OwnedUniqueName::try_from(owner_name)?;

        let added_rule = format!(
            "type='signal',sender='{owner}',path='{modem_path}',interface='{MM_MESSAGING_INTERFACE}',member='Added'"
        );
        add_match_rule(&connection, &added_rule).await?;

        let removed_rule = format!(
            "type='signal',sender='{owner}',interface='{OBJECT_MANAGER_INTERFACE}',member='InterfacesRemoved'"
        );
        add_match_rule(&connection, &removed_rule).await?;

        let reader = Arc::new(ZbusSmsPropertiesReader {
            connection,
            owner: owner.clone(),
        });
        Ok(InboundSubscription::new(
            modem_path.to_string(),
            owner.to_string(),
            Box::new(ZbusSessionBackend { stream }),
            reader,
        ))
    }
}

pub(crate) struct InboundSubscription {
    modem_path: String,
    owner: String,
    backend: Box<dyn SessionBackend>,
    reader: Arc<dyn SmsPropertiesReader>,
    terminal_error: Option<&'static str>,
}

impl InboundSubscription {
    fn new(
        modem_path: String,
        owner: String,
        backend: Box<dyn SessionBackend>,
        reader: Arc<dyn SmsPropertiesReader>,
    ) -> Self {
        Self {
            modem_path,
            owner,
            backend,
            reader,
            terminal_error: None,
        }
    }

    pub(crate) async fn next(&mut self) -> Result<InboundEvent> {
        if let Some(error) = self.terminal_error {
            return Err(anyhow::anyhow!(error));
        }

        loop {
            let Some(signal) = self.backend.next_signal().await? else {
                self.terminal_error = Some("ModemManager signal stream ended");
                return Err(anyhow::anyhow!("ModemManager signal stream ended"));
            };
            match signal {
                ProtocolSignal::Added {
                    sender,
                    signal_path,
                    sms_path,
                    is_received,
                } if sender.as_deref() == Some(self.owner.as_str())
                    && signal_path.as_deref() == Some(self.modem_path.as_str())
                    && is_received =>
                {
                    return Ok(InboundEvent::Added(InboundSms {
                        path: sms_path,
                        reader: self.reader.clone(),
                    }));
                }
                ProtocolSignal::NameOwnerChanged {
                    name,
                    old_owner,
                    new_owner,
                } if modem_owner_changed(&name, &old_owner, &new_owner) => {
                    self.terminal_error = Some("ModemManager owner changed");
                    return Err(anyhow::anyhow!("ModemManager owner changed"));
                }
                ProtocolSignal::InterfacesRemoved {
                    sender,
                    removed_path,
                } if sender.as_deref() == Some(self.owner.as_str())
                    && removed_path == self.modem_path =>
                {
                    self.terminal_error = Some("monitored modem object removed");
                    return Err(anyhow::anyhow!("monitored modem object removed"));
                }
                _ => {}
            }
        }
    }
}

trait SessionBackend: Send {
    fn next_signal<'a>(&'a mut self) -> BoxFuture<'a, Result<Option<ProtocolSignal>>>;
}

trait SmsPropertiesReader: Send + Sync {
    fn read<'a>(&'a self, sms_path: &'a str) -> BoxFuture<'a, Result<InboundSmsProperties>>;
}

enum ProtocolSignal {
    Added {
        sender: Option<String>,
        signal_path: Option<String>,
        sms_path: String,
        is_received: bool,
    },
    NameOwnerChanged {
        name: String,
        old_owner: String,
        new_owner: String,
    },
    InterfacesRemoved {
        sender: Option<String>,
        removed_path: String,
    },
    Unrelated,
}

struct ZbusSessionBackend {
    stream: MessageStream,
}

impl SessionBackend for ZbusSessionBackend {
    fn next_signal<'a>(&'a mut self) -> BoxFuture<'a, Result<Option<ProtocolSignal>>> {
        Box::pin(async move {
            match self.stream.next().await {
                Some(Ok(message)) => Ok(Some(decode_message(&message))),
                Some(Err(error)) => Err(error.into()),
                None => Ok(None),
            }
        })
    }
}

struct ZbusSmsPropertiesReader {
    connection: Arc<Connection>,
    owner: OwnedUniqueName,
}

impl SmsPropertiesReader for ZbusSmsPropertiesReader {
    fn read<'a>(&'a self, sms_path: &'a str) -> BoxFuture<'a, Result<InboundSmsProperties>> {
        Box::pin(async move {
            let call = self.connection.call_method(
                Some(&self.owner),
                sms_path,
                Some(DBUS_PROPERTIES_INTERFACE),
                "GetAll",
                &(MM_SMS_INTERFACE,),
            );
            let reply = tokio::time::timeout(DBUS_PROPERTIES_TIMEOUT, call)
                .await
                .map_err(|_| anyhow::anyhow!("dbus getAll timeout"))??;
            let properties: HashMap<String, OwnedValue> = reply.body().deserialize()?;
            Ok(InboundSmsProperties {
                phone_number: extract_string(&properties, "Number"),
                body: extract_string(&properties, "Text"),
                timestamp: extract_string(&properties, "Timestamp"),
                storage: extract_u32(&properties, "Storage"),
            })
        })
    }
}

fn decode_message(message: &Message) -> ProtocolSignal {
    let header = message.header();
    let interface = header.interface().map(|value| value.as_str());
    let member = header.member().map(|value| value.as_str());

    if interface == Some(DBUS_INTERFACE) && member == Some("NameOwnerChanged") {
        return message
            .body()
            .deserialize::<(String, String, String)>()
            .map(
                |(name, old_owner, new_owner)| ProtocolSignal::NameOwnerChanged {
                    name,
                    old_owner,
                    new_owner,
                },
            )
            .unwrap_or(ProtocolSignal::Unrelated);
    }

    if interface == Some(OBJECT_MANAGER_INTERFACE) && member == Some("InterfacesRemoved") {
        return message
            .body()
            .deserialize::<(OwnedObjectPath, Vec<String>)>()
            .map(
                |(removed_path, _interfaces)| ProtocolSignal::InterfacesRemoved {
                    sender: header.sender().map(ToString::to_string),
                    removed_path: removed_path.to_string(),
                },
            )
            .unwrap_or(ProtocolSignal::Unrelated);
    }

    if interface == Some(MM_MESSAGING_INTERFACE) && member == Some("Added") {
        return message
            .body()
            .deserialize::<(OwnedObjectPath, bool)>()
            .map(|(sms_path, is_received)| ProtocolSignal::Added {
                sender: header.sender().map(ToString::to_string),
                signal_path: header.path().map(ToString::to_string),
                sms_path: sms_path.to_string(),
                is_received,
            })
            .unwrap_or(ProtocolSignal::Unrelated);
    }

    ProtocolSignal::Unrelated
}

async fn add_match_rule(connection: &Connection, rule: &str) -> Result<()> {
    let args = (rule,);
    let call = connection.call_method(
        Some(DBUS_INTERFACE),
        "/org/freedesktop/DBus",
        Some(DBUS_INTERFACE),
        "AddMatch",
        &args,
    );
    tokio::time::timeout(DBUS_METHOD_TIMEOUT, call).await??;
    Ok(())
}

fn modem_owner_changed(name: &str, old_owner: &str, new_owner: &str) -> bool {
    name == MM_DESTINATION && old_owner != new_owner
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, VecDeque};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use super::*;

    const MODEM_PATH: &str = "/org/freedesktop/ModemManager1/Modem/0";
    const SMS_PATH: &str = "/org/freedesktop/ModemManager1/SMS/1";
    const OWNER_A: &str = ":1.42";
    const OWNER_B: &str = ":1.43";

    enum ScriptAction {
        Signal(ProtocolSignal),
        Error(anyhow::Error),
        End,
    }

    struct ScriptedSessionBackend {
        actions: VecDeque<ScriptAction>,
        reads: Arc<AtomicUsize>,
    }

    impl SessionBackend for ScriptedSessionBackend {
        fn next_signal<'a>(&'a mut self) -> BoxFuture<'a, Result<Option<ProtocolSignal>>> {
            Box::pin(async move {
                self.reads.fetch_add(1, Ordering::SeqCst);
                match self.actions.pop_front() {
                    Some(ScriptAction::Signal(signal)) => Ok(Some(signal)),
                    Some(ScriptAction::Error(error)) => Err(error),
                    Some(ScriptAction::End) | None => Ok(None),
                }
            })
        }
    }

    struct ScriptedPropertiesReader {
        session_id: &'static str,
        responses: Mutex<HashMap<String, VecDeque<Result<InboundSmsProperties>>>>,
        calls: Arc<Mutex<Vec<(String, String)>>>,
    }

    impl ScriptedPropertiesReader {
        fn new(
            session_id: &'static str,
            responses: Vec<(&str, Result<InboundSmsProperties>)>,
        ) -> Self {
            let mut by_path: HashMap<String, VecDeque<Result<InboundSmsProperties>>> =
                HashMap::new();
            for (path, response) in responses {
                by_path
                    .entry(path.to_string())
                    .or_default()
                    .push_back(response);
            }
            Self {
                session_id,
                responses: Mutex::new(by_path),
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl SmsPropertiesReader for ScriptedPropertiesReader {
        fn read<'a>(&'a self, sms_path: &'a str) -> BoxFuture<'a, Result<InboundSmsProperties>> {
            Box::pin(async move {
                self.calls
                    .lock()
                    .unwrap()
                    .push((self.session_id.to_string(), sms_path.to_string()));
                self.responses
                    .lock()
                    .unwrap()
                    .get_mut(sms_path)
                    .and_then(VecDeque::pop_front)
                    .unwrap_or_else(|| Err(anyhow::anyhow!("missing scripted property response")))
            })
        }
    }

    fn properties(body: &str) -> InboundSmsProperties {
        InboundSmsProperties {
            phone_number: "+15550000000".to_string(),
            body: body.to_string(),
            timestamp: "2026-07-25T00:00:00Z".to_string(),
            storage: 2,
        }
    }

    fn added(sender: &str, signal_path: &str, sms_path: &str, is_received: bool) -> ProtocolSignal {
        ProtocolSignal::Added {
            sender: Some(sender.to_string()),
            signal_path: Some(signal_path.to_string()),
            sms_path: sms_path.to_string(),
            is_received,
        }
    }

    fn scripted_subscription(
        owner: &str,
        actions: Vec<ScriptAction>,
        reader: Arc<ScriptedPropertiesReader>,
    ) -> (InboundSubscription, Arc<AtomicUsize>) {
        let reads = Arc::new(AtomicUsize::new(0));
        (
            InboundSubscription::new(
                MODEM_PATH.to_string(),
                owner.to_string(),
                Box::new(ScriptedSessionBackend {
                    actions: actions.into(),
                    reads: reads.clone(),
                }),
                reader,
            ),
            reads,
        )
    }

    #[tokio::test]
    async fn received_added_returns_path_and_complete_properties() {
        let reader = Arc::new(ScriptedPropertiesReader::new(
            "A",
            vec![(SMS_PATH, Ok(properties("hello")))],
        ));
        let actions = vec![
            ScriptAction::Signal(added(OWNER_A, MODEM_PATH, SMS_PATH, false)),
            ScriptAction::Signal(ProtocolSignal::Unrelated),
            ScriptAction::Signal(added(OWNER_A, MODEM_PATH, SMS_PATH, true)),
        ];
        let (mut subscription, _) = scripted_subscription(OWNER_A, actions, reader);

        let InboundEvent::Added(sms) = subscription.next().await.unwrap();

        assert_eq!(sms.path(), SMS_PATH);
        assert_eq!(sms.properties().await.unwrap(), properties("hello"));
    }

    #[tokio::test]
    async fn owner_change_is_terminal_and_does_not_consume_later_added() {
        let reader = Arc::new(ScriptedPropertiesReader::new("A", vec![]));
        let actions = vec![
            ScriptAction::Signal(ProtocolSignal::NameOwnerChanged {
                name: MM_DESTINATION.to_string(),
                old_owner: OWNER_A.to_string(),
                new_owner: OWNER_B.to_string(),
            }),
            ScriptAction::Signal(added(OWNER_A, MODEM_PATH, SMS_PATH, true)),
        ];
        let (mut subscription, reads) = scripted_subscription(OWNER_A, actions, reader);

        assert_eq!(
            subscription.next().await.unwrap_err().to_string(),
            "ModemManager owner changed"
        );
        assert_eq!(
            subscription.next().await.unwrap_err().to_string(),
            "ModemManager owner changed"
        );
        assert_eq!(reads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn only_bound_owner_removal_of_monitored_path_is_terminal() {
        let reader = Arc::new(ScriptedPropertiesReader::new("A", vec![]));
        let actions = vec![
            ScriptAction::Signal(ProtocolSignal::InterfacesRemoved {
                sender: Some(OWNER_A.to_string()),
                removed_path: "/org/freedesktop/ModemManager1/Modem/9".to_string(),
            }),
            ScriptAction::Signal(ProtocolSignal::InterfacesRemoved {
                sender: Some(OWNER_B.to_string()),
                removed_path: MODEM_PATH.to_string(),
            }),
            ScriptAction::Signal(ProtocolSignal::InterfacesRemoved {
                sender: Some(OWNER_A.to_string()),
                removed_path: MODEM_PATH.to_string(),
            }),
        ];
        let (mut subscription, _) = scripted_subscription(OWNER_A, actions, reader);

        assert_eq!(
            subscription.next().await.unwrap_err().to_string(),
            "monitored modem object removed"
        );
    }

    #[tokio::test]
    async fn stream_end_uses_existing_error_text() {
        let reader = Arc::new(ScriptedPropertiesReader::new("A", vec![]));
        let (mut subscription, _) = scripted_subscription(OWNER_A, vec![ScriptAction::End], reader);

        assert_eq!(
            subscription.next().await.unwrap_err().to_string(),
            "ModemManager signal stream ended"
        );
    }

    #[tokio::test]
    async fn stream_and_property_errors_propagate_without_retry() {
        let property_reader = Arc::new(ScriptedPropertiesReader::new(
            "A",
            vec![(SMS_PATH, Err(anyhow::anyhow!("property failed")))],
        ));
        let property_calls = property_reader.calls.clone();
        let (mut property_subscription, _) = scripted_subscription(
            OWNER_A,
            vec![ScriptAction::Signal(added(
                OWNER_A, MODEM_PATH, SMS_PATH, true,
            ))],
            property_reader,
        );
        let InboundEvent::Added(sms) = property_subscription.next().await.unwrap();

        assert_eq!(
            sms.properties().await.unwrap_err().to_string(),
            "property failed"
        );
        assert_eq!(property_calls.lock().unwrap().len(), 1);

        let reader = Arc::new(ScriptedPropertiesReader::new("A", vec![]));
        let (mut stream_subscription, _) = scripted_subscription(
            OWNER_A,
            vec![ScriptAction::Error(anyhow::anyhow!("stream failed"))],
            reader,
        );
        assert_eq!(
            stream_subscription.next().await.unwrap_err().to_string(),
            "stream failed"
        );
    }

    #[tokio::test]
    async fn old_handle_never_falls_back_to_new_session_reader() {
        let reader_a = Arc::new(ScriptedPropertiesReader::new(
            "A",
            vec![(SMS_PATH, Err(anyhow::anyhow!("old owner gone")))],
        ));
        let calls_a = reader_a.calls.clone();
        let (mut subscription_a, _) = scripted_subscription(
            OWNER_A,
            vec![
                ScriptAction::Signal(added(OWNER_A, MODEM_PATH, SMS_PATH, true)),
                ScriptAction::Signal(ProtocolSignal::NameOwnerChanged {
                    name: MM_DESTINATION.to_string(),
                    old_owner: OWNER_A.to_string(),
                    new_owner: OWNER_B.to_string(),
                }),
            ],
            reader_a,
        );
        let InboundEvent::Added(old_sms) = subscription_a.next().await.unwrap();
        assert_eq!(
            subscription_a.next().await.unwrap_err().to_string(),
            "ModemManager owner changed"
        );

        let reader_b = Arc::new(ScriptedPropertiesReader::new(
            "B",
            vec![(SMS_PATH, Ok(properties("new owner data")))],
        ));
        let calls_b = reader_b.calls.clone();
        let (_subscription_b, _) = scripted_subscription(OWNER_B, vec![], reader_b);

        assert_eq!(
            old_sms.properties().await.unwrap_err().to_string(),
            "old owner gone"
        );
        assert_eq!(
            calls_a.lock().unwrap().as_slice(),
            &[("A".to_string(), SMS_PATH.to_string())]
        );
        assert!(calls_b.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn malformed_and_unrelated_signals_are_ignored() {
        let reader = Arc::new(ScriptedPropertiesReader::new("A", vec![]));
        let actions = vec![
            ScriptAction::Signal(ProtocolSignal::Unrelated),
            ScriptAction::Signal(added(OWNER_B, MODEM_PATH, SMS_PATH, true)),
            ScriptAction::Signal(added(
                OWNER_A,
                "/org/freedesktop/ModemManager1/Modem/9",
                SMS_PATH,
                true,
            )),
            ScriptAction::Signal(added(OWNER_A, MODEM_PATH, SMS_PATH, true)),
        ];
        let (mut subscription, reads) = scripted_subscription(OWNER_A, actions, reader);

        let InboundEvent::Added(sms) = subscription.next().await.unwrap();

        assert_eq!(sms.path(), SMS_PATH);
        assert_eq!(reads.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn owner_change_requires_reconnect_only_for_modem_manager() {
        assert!(modem_owner_changed(MM_DESTINATION, ":1.1", ":1.2"));
        assert!(!modem_owner_changed(MM_DESTINATION, ":1.1", ":1.1"));
        assert!(!modem_owner_changed("org.example.Other", ":1.1", ":1.2"));
    }
}
