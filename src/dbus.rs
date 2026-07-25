use std::collections::HashMap;

use zbus::zvariant::{OwnedValue, Value};

mod connection;
mod inbound;
mod outbound;

#[allow(unused_imports)]
pub(crate) use inbound::{
    InboundEvent, InboundSms, InboundSmsProperties, InboundSubscription, SystemInboundSource,
};

// Preserve the existing `crate::dbus` facade, including raw helpers with no in-crate caller.
#[allow(unused_imports)]
pub use outbound::{
    create_sms, get_sms_snapshot, get_sms_state, send_prepared_sms, ModemSmsState, PreparedSms,
    SendAttemptOutcome, SmsSender, SmsSnapshot, SystemSmsSender,
};

const MM_SMS_INTERFACE: &str = "org.freedesktop.ModemManager1.Sms";
const MM_MESSAGING_INTERFACE: &str = "org.freedesktop.ModemManager1.Modem.Messaging";
const DBUS_PROPERTIES_INTERFACE: &str = "org.freedesktop.DBus.Properties";
const DBUS_INTERFACE: &str = "org.freedesktop.DBus";
const OBJECT_MANAGER_INTERFACE: &str = "org.freedesktop.DBus.ObjectManager";
const MM_DESTINATION: &str = "org.freedesktop.ModemManager1";

fn extract_string(props: &HashMap<String, OwnedValue>, key: &str) -> String {
    props
        .get(key)
        .and_then(|v| {
            let val: Value = (**v).clone();
            if let Value::Str(s) = val {
                Some(s.to_string())
            } else {
                None
            }
        })
        .unwrap_or_default()
}

fn extract_u32(props: &HashMap<String, OwnedValue>, key: &str) -> u32 {
    props
        .get(key)
        .and_then(|v| {
            let val: Value = (**v).clone();
            if let Value::U32(n) = val {
                Some(n)
            } else {
                None
            }
        })
        .unwrap_or(100)
}
