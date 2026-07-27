use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SmsOverImsStatus {
    Available,
    Registering,
    Limited,
    NotRegistered,
    Unavailable,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImsSupport {
    Supported,
    #[allow(dead_code)]
    Unsupported,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImsConfigured {
    Enabled,
    Disabled,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImsRegistration {
    Registered,
    Registering,
    Limited,
    NotRegistered,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImsSmsService {
    Available,
    Limited,
    Unavailable,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImsTechnology {
    Wwan,
    Wlan,
    InterworkingWlan,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImsTransport {
    #[allow(dead_code)]
    DirectQmi,
    Unknown,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ImsCapabilities {
    pub ims_settings: bool,
    pub imsa_registration: bool,
    pub imsa_services: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImsProbeInfo {
    pub tool: &'static str,
    pub available: bool,
    pub version_raw: Option<String>,
    pub transport: ImsTransport,
    pub device: Option<String>,
    pub capabilities: ImsCapabilities,
}

#[derive(Debug, Clone, Serialize)]
pub struct SmsOverIms {
    pub status: SmsOverImsStatus,
    pub support: ImsSupport,
    pub configured: ImsConfigured,
    pub registration: ImsRegistration,
    pub sms_service: ImsSmsService,
    pub technology: ImsTechnology,
    pub probe: ImsProbeInfo,
    pub evidence: Vec<String>,
    pub reasons: Vec<String>,
    pub warnings: Vec<String>,
}

impl Default for SmsOverIms {
    fn default() -> Self {
        Self {
            status: SmsOverImsStatus::Unknown,
            support: ImsSupport::Unknown,
            configured: ImsConfigured::Unknown,
            registration: ImsRegistration::Unknown,
            sms_service: ImsSmsService::Unknown,
            technology: ImsTechnology::Unknown,
            probe: ImsProbeInfo {
                tool: "qmicli",
                available: false,
                version_raw: None,
                transport: ImsTransport::Unknown,
                device: None,
                capabilities: ImsCapabilities::default(),
            },
            evidence: Vec::new(),
            reasons: vec!["ims_probe_not_attempted".to_string()],
            warnings: Vec::new(),
        }
    }
}

impl SmsOverIms {
    pub fn classify(&mut self) {
        self.support = if self.configured != ImsConfigured::Unknown
            || self.sms_service != ImsSmsService::Unknown
        {
            ImsSupport::Supported
        } else {
            ImsSupport::Unknown
        };
        self.status = if self.registration == ImsRegistration::Registered
            && self.sms_service == ImsSmsService::Available
        {
            SmsOverImsStatus::Available
        } else if self.registration == ImsRegistration::Limited
            || self.sms_service == ImsSmsService::Limited
        {
            SmsOverImsStatus::Limited
        } else if self.registration == ImsRegistration::Registering {
            SmsOverImsStatus::Registering
        } else if self.registration == ImsRegistration::NotRegistered {
            SmsOverImsStatus::NotRegistered
        } else if self.registration == ImsRegistration::Registered
            && self.sms_service == ImsSmsService::Unavailable
        {
            SmsOverImsStatus::Unavailable
        } else {
            SmsOverImsStatus::Unknown
        };
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedImsaServices {
    pub sms_service: ImsSmsService,
    pub technology: ImsTechnology,
    pub nonstandard: bool,
}

pub fn parse_imsa_services(raw: &str) -> ParsedImsaServices {
    let mut parsed = ParsedImsaServices {
        sms_service: ImsSmsService::Unknown,
        technology: ImsTechnology::Unknown,
        nonstandard: false,
    };
    for line in raw.lines() {
        let Some((label, value)) = line.trim().split_once(':') else {
            continue;
        };
        let label = label.trim().to_ascii_lowercase();
        let value = normalized_value(value);
        if label == "ims sms service status" {
            parsed.sms_service = match value.as_str() {
                "available" | "full service" => ImsSmsService::Available,
                "limited" | "limited service" => ImsSmsService::Limited,
                "unavailable" | "no service" => ImsSmsService::Unavailable,
                _ => ImsSmsService::Unknown,
            };
        } else if label == "ims sms service rat" || label == "ims sms service technology" {
            parsed.nonstandard |= label != "ims sms service rat";
            parsed.technology = match value.as_str() {
                "wwan" => ImsTechnology::Wwan,
                "wlan" => ImsTechnology::Wlan,
                "interworking wlan" | "interworking-wlan" | "iwlan" => {
                    ImsTechnology::InterworkingWlan
                }
                _ => ImsTechnology::Unknown,
            };
        }
    }
    parsed
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedImsaRegistration {
    pub registration: ImsRegistration,
    pub nonstandard: bool,
}

pub fn parse_imsa_registration(raw: &str) -> ParsedImsaRegistration {
    let mut parsed = ParsedImsaRegistration {
        registration: ImsRegistration::Unknown,
        nonstandard: false,
    };
    for line in raw.lines() {
        let Some((label, value)) = line.trim().split_once(':') else {
            continue;
        };
        let label = label.trim().to_ascii_lowercase();
        if label != "ims registration status" && label != "registration status" {
            continue;
        }
        parsed.nonstandard |= label != "ims registration status";
        parsed.registration = match normalized_value(value).as_str() {
            "registered" => ImsRegistration::Registered,
            "registering" => ImsRegistration::Registering,
            "limited" | "limited service" => ImsRegistration::Limited,
            "not registered" | "not-registered" => ImsRegistration::NotRegistered,
            _ => ImsRegistration::Unknown,
        };
    }
    parsed
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedImsSettings {
    pub configured: ImsConfigured,
    pub nonstandard: bool,
}

pub fn parse_ims_settings(raw: &str) -> ParsedImsSettings {
    let mut parsed = ParsedImsSettings {
        configured: ImsConfigured::Unknown,
        nonstandard: false,
    };
    for line in raw.lines() {
        let Some((label, value)) = line.trim().split_once(':') else {
            continue;
        };
        let label = label.trim().to_ascii_lowercase();
        if label != "ims sms service" && label != "ims sms enabled" {
            continue;
        }
        parsed.nonstandard |= label != "ims sms service";
        parsed.configured = match normalized_value(value).as_str() {
            "enabled" | "yes" | "true" => ImsConfigured::Enabled,
            "disabled" | "no" | "false" => ImsConfigured::Disabled,
            _ => ImsConfigured::Unknown,
        };
    }
    parsed
}

fn normalized_value(value: &str) -> String {
    value
        .trim()
        .trim_matches(|character| matches!(character, '\'' | '"'))
        .trim()
        .to_ascii_lowercase()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QmiPortSelection {
    Selected(String),
    Unavailable,
    Ambiguous,
}

pub fn select_qmi_device_from_modem_json(raw: &str) -> QmiPortSelection {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return QmiPortSelection::Unavailable;
    };
    let generic = &value["modem"]["generic"];
    let primary = generic.get("primary-port").and_then(|value| value.as_str());
    let ports = generic
        .get("ports")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str())
        .filter_map(qmi_port_basename)
        .collect::<Vec<_>>();

    if let Some(primary) = primary.filter(|primary| ports.iter().any(|port| port == primary)) {
        return QmiPortSelection::Selected(format!("/dev/{primary}"));
    }
    match ports.as_slice() {
        [port] => QmiPortSelection::Selected(format!("/dev/{port}")),
        [] => QmiPortSelection::Unavailable,
        _ => QmiPortSelection::Ambiguous,
    }
}

pub fn select_qmi_device_from_modem_text(raw: &str) -> QmiPortSelection {
    let primary = raw.lines().find_map(|line| {
        let lower = line.to_ascii_lowercase();
        let offset = lower.find("primary port:")?;
        line.get(offset + "primary port:".len()..)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    });
    let mut ports = Vec::new();
    for (offset, _) in raw.match_indices(" (qmi)") {
        let prefix = &raw[..offset];
        let basename = prefix
            .chars()
            .rev()
            .take_while(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
            })
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>();
        if qmi_port_basename(&format!("{basename} (qmi)")).is_some() {
            ports.push(basename);
        }
    }
    if let Some(primary) = primary.filter(|primary| ports.iter().any(|port| port == primary)) {
        return QmiPortSelection::Selected(format!("/dev/{primary}"));
    }
    match ports.as_slice() {
        [port] => QmiPortSelection::Selected(format!("/dev/{port}")),
        [] => QmiPortSelection::Unavailable,
        _ => QmiPortSelection::Ambiguous,
    }
}

fn qmi_port_basename(port: &str) -> Option<String> {
    let basename = port.strip_suffix(" (qmi)")?;
    if basename.is_empty()
        || basename == "."
        || basename == ".."
        || basename
            .chars()
            .any(|character| character == '/' || character.is_control())
    {
        return None;
    }
    Some(basename.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_available_sms_is_available_over_wwan() {
        let mut status = SmsOverIms::default();
        status.registration = ImsRegistration::Registered;
        status.sms_service = ImsSmsService::Available;
        status.technology = ImsTechnology::Wwan;

        status.classify();

        assert_eq!(status.status, SmsOverImsStatus::Available);
        assert_eq!(status.support, ImsSupport::Supported);
    }

    #[test]
    fn limited_evidence_takes_priority_over_registering() {
        let mut status = SmsOverIms {
            registration: ImsRegistration::Registering,
            sms_service: ImsSmsService::Limited,
            ..SmsOverIms::default()
        };

        status.classify();

        assert_eq!(status.status, SmsOverImsStatus::Limited);
    }

    #[test]
    fn registered_unavailable_sms_is_unavailable() {
        let mut status = SmsOverIms {
            registration: ImsRegistration::Registered,
            sms_service: ImsSmsService::Unavailable,
            ..SmsOverIms::default()
        };

        status.classify();

        assert_eq!(status.status, SmsOverImsStatus::Unavailable);
    }

    #[test]
    fn parses_available_sms_service_over_wlan() {
        let parsed = parse_imsa_services(
            "IMS SMS service status: 'available'\nIMS SMS service RAT: 'wlan'\n",
        );

        assert_eq!(parsed.sms_service, ImsSmsService::Available);
        assert_eq!(parsed.technology, ImsTechnology::Wlan);
        assert!(!parsed.nonstandard);
    }

    #[test]
    fn parses_registration_and_sms_enabled_setting() {
        assert_eq!(
            parse_imsa_registration("IMS registration status: 'registered'\n").registration,
            ImsRegistration::Registered
        );
        assert_eq!(
            parse_ims_settings("IMS SMS service: 'enabled'\n").configured,
            ImsConfigured::Enabled
        );
    }

    #[test]
    fn selects_sd410_primary_qmi_port_from_modem_json() {
        let raw = r#"{
            "modem": {
                "generic": {
                    "primary-port": "wwan0qmi0",
                    "ports": [
                        "rpmsg_ctrl2 (ignored)",
                        "wwan0 (net)",
                        "wwan0at0 (at)",
                        "wwan0at1 (at)",
                        "wwan0qmi0 (qmi)"
                    ]
                }
            }
        }"#;

        assert_eq!(
            select_qmi_device_from_modem_json(raw),
            QmiPortSelection::Selected("/dev/wwan0qmi0".to_string())
        );
    }

    #[test]
    fn selects_sd410_primary_qmi_port_from_text_fallback() {
        let raw = "  -------------------------\n\
                   Hardware |       primary port: wwan0qmi0\n\
                            |              ports: rpmsg_ctrl2 (ignored), wwan0 (net), \\\n\
                            |                     wwan0at0 (at), wwan0qmi0 (qmi)\n";

        assert_eq!(
            select_qmi_device_from_modem_text(raw),
            QmiPortSelection::Selected("/dev/wwan0qmi0".to_string())
        );
    }

    #[test]
    fn selects_qmi_port_from_real_sd410_text_fixture() {
        assert_eq!(
            select_qmi_device_from_modem_text(include_str!(
                "../../tests/fixtures/mmcli/sd410-ports.txt"
            )),
            QmiPortSelection::Selected("/dev/wwan0qmi0".to_string())
        );
    }

    #[test]
    fn refuses_to_guess_between_multiple_non_primary_qmi_ports() {
        let raw = r#"{
            "modem": {
                "generic": {
                    "primary-port": "wwan0at0",
                    "ports": ["wwan0qmi0 (qmi)", "wwan0qmi1 (qmi)"]
                }
            }
        }"#;

        assert_eq!(
            select_qmi_device_from_modem_json(raw),
            QmiPortSelection::Ambiguous
        );
    }
}
