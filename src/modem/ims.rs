use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;

mod qmi;
use qmi::{
    NativeQmiError, ProxyQmiClient, QmiRequestClient, QmiResponse, QMI_CTL_GET_VERSION_INFO,
    QMI_IMSA_GET_REGISTRATION_STATUS, QMI_IMSA_GET_SERVICES_STATUS, QMI_IMS_GET_SERVICES_ENABLED,
    QMI_NAS_GET_SYSTEM_INFO, QMI_SERVICE_CTL, QMI_SERVICE_IMS, QMI_SERVICE_IMSA, QMI_SERVICE_NAS,
};

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
pub enum VoiceOverImsStatus {
    Volte,
    Vowifi,
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
pub enum ImsServiceStatus {
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
    QmiProxy,
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
    pub voice_over_ims: VoiceOverImsStatus,
    pub support: ImsSupport,
    pub lte_voice_support: Option<bool>,
    pub ims_voice_support: Option<bool>,
    pub configured: ImsConfigured,
    pub volte_configured: ImsConfigured,
    pub vowifi_configured: ImsConfigured,
    pub registration: ImsRegistration,
    pub voice_service: ImsServiceStatus,
    pub voice_technology: ImsTechnology,
    pub sms_service: ImsServiceStatus,
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
            voice_over_ims: VoiceOverImsStatus::Unknown,
            support: ImsSupport::Unknown,
            lte_voice_support: None,
            ims_voice_support: None,
            configured: ImsConfigured::Unknown,
            volte_configured: ImsConfigured::Unknown,
            vowifi_configured: ImsConfigured::Unknown,
            registration: ImsRegistration::Unknown,
            voice_service: ImsServiceStatus::Unknown,
            voice_technology: ImsTechnology::Unknown,
            sms_service: ImsServiceStatus::Unknown,
            technology: ImsTechnology::Unknown,
            probe: ImsProbeInfo {
                tool: "native-qmi",
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
            || self.sms_service != ImsServiceStatus::Unknown
        {
            ImsSupport::Supported
        } else {
            ImsSupport::Unknown
        };
        self.status = if self.registration == ImsRegistration::Registered
            && self.sms_service == ImsServiceStatus::Available
        {
            SmsOverImsStatus::Available
        } else if self.registration == ImsRegistration::Limited
            || self.sms_service == ImsServiceStatus::Limited
        {
            SmsOverImsStatus::Limited
        } else if self.registration == ImsRegistration::Registering {
            SmsOverImsStatus::Registering
        } else if self.registration == ImsRegistration::NotRegistered {
            SmsOverImsStatus::NotRegistered
        } else if self.registration == ImsRegistration::Registered
            && self.sms_service == ImsServiceStatus::Unavailable
        {
            SmsOverImsStatus::Unavailable
        } else {
            SmsOverImsStatus::Unknown
        };
        self.voice_over_ims = if self.registration == ImsRegistration::Registered
            && self.voice_service == ImsServiceStatus::Available
        {
            match self.voice_technology {
                ImsTechnology::Wwan => VoiceOverImsStatus::Volte,
                ImsTechnology::Wlan | ImsTechnology::InterworkingWlan => VoiceOverImsStatus::Vowifi,
                ImsTechnology::Unknown => VoiceOverImsStatus::Unknown,
            }
        } else if self.registration == ImsRegistration::Limited
            || self.voice_service == ImsServiceStatus::Limited
        {
            VoiceOverImsStatus::Limited
        } else if self.registration == ImsRegistration::Registering {
            VoiceOverImsStatus::Registering
        } else if self.registration == ImsRegistration::NotRegistered {
            VoiceOverImsStatus::NotRegistered
        } else if self.registration == ImsRegistration::Registered
            && self.voice_service == ImsServiceStatus::Unavailable
        {
            VoiceOverImsStatus::Unavailable
        } else {
            VoiceOverImsStatus::Unknown
        };
    }
}

fn qmi_imsa_registration(value: Option<u32>) -> ImsRegistration {
    match value {
        Some(0) => ImsRegistration::NotRegistered,
        Some(1) => ImsRegistration::Registering,
        Some(2) => ImsRegistration::Registered,
        Some(3) => ImsRegistration::Limited,
        _ => ImsRegistration::Unknown,
    }
}

fn qmi_imsa_service(value: Option<u32>) -> ImsServiceStatus {
    match value {
        Some(0) => ImsServiceStatus::Unavailable,
        Some(1) => ImsServiceStatus::Limited,
        Some(2) => ImsServiceStatus::Available,
        _ => ImsServiceStatus::Unknown,
    }
}

fn qmi_imsa_technology(value: Option<u32>) -> ImsTechnology {
    match value {
        Some(0) => ImsTechnology::Wlan,
        Some(1) => ImsTechnology::Wwan,
        Some(2) => ImsTechnology::InterworkingWlan,
        _ => ImsTechnology::Unknown,
    }
}

fn qmi_configured(value: Option<bool>) -> ImsConfigured {
    match value {
        Some(true) => ImsConfigured::Enabled,
        Some(false) => ImsConfigured::Disabled,
        None => ImsConfigured::Unknown,
    }
}

#[derive(Debug)]
struct QmiServiceVersions {
    ctl_version: Option<(u16, u16)>,
    nas: bool,
    ims: bool,
    imsa: bool,
}

fn parse_service_versions(response: &QmiResponse) -> Result<QmiServiceVersions, NativeQmiError> {
    let value = response.tlvs.get(&0x01).ok_or(NativeQmiError::Protocol)?;
    let (&count, services) = value.split_first().ok_or(NativeQmiError::Protocol)?;
    if services.len() != usize::from(count) * 5 {
        return Err(NativeQmiError::Protocol);
    }
    let mut parsed = QmiServiceVersions {
        ctl_version: None,
        nas: false,
        ims: false,
        imsa: false,
    };
    for service in services.chunks_exact(5) {
        let version = (
            u16::from_le_bytes([service[1], service[2]]),
            u16::from_le_bytes([service[3], service[4]]),
        );
        match service[0] {
            QMI_SERVICE_CTL => parsed.ctl_version = Some(version),
            QMI_SERVICE_NAS => parsed.nas = true,
            QMI_SERVICE_IMS => parsed.ims = true,
            QMI_SERVICE_IMSA => parsed.imsa = true,
            _ => {}
        }
    }
    Ok(parsed)
}

#[derive(Clone)]
pub struct NativeImsProbe {
    client: Arc<dyn QmiRequestClient>,
}

impl NativeImsProbe {
    pub fn new() -> Self {
        Self::with_client(ProxyQmiClient::new())
    }

    fn with_client<C>(client: C) -> Self
    where
        C: QmiRequestClient + 'static,
    {
        Self {
            client: Arc::new(client),
        }
    }

    async fn request_response(
        &self,
        device: &str,
        service: u8,
        message: u16,
        deadline: tokio::time::Instant,
    ) -> Result<QmiResponse, NativeQmiError> {
        let timeout = remaining_until(deadline).ok_or(NativeQmiError::Timeout)?;
        self.client
            .request(device, service, message, timeout)
            .await
            .and_then(|raw| QmiResponse::parse(&raw, service, message))
    }

    async fn probe_inner(
        &self,
        modem_output: &str,
        json: bool,
        enabled: Option<bool>,
    ) -> SmsOverIms {
        let mut result = SmsOverIms::default();
        if enabled == Some(false) {
            result.reasons = vec!["modem_disabled".to_string()];
            return result;
        }
        let selection = if json {
            select_qmi_device_from_modem_json(modem_output)
        } else {
            select_qmi_device_from_modem_text(modem_output)
        };
        let device = match selection {
            QmiPortSelection::Selected(device) => device,
            QmiPortSelection::Unavailable => {
                result.reasons = vec!["qmi_port_unavailable".to_string()];
                return result;
            }
            QmiPortSelection::Ambiguous => {
                result.reasons = vec!["qmi_port_ambiguous".to_string()];
                return result;
            }
        };
        result.probe.tool = "native-qmi";
        result.probe.transport = ImsTransport::QmiProxy;
        result.probe.device = Some(device.clone());
        result.reasons.clear();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let versions = match self
            .request_response(&device, QMI_SERVICE_CTL, QMI_CTL_GET_VERSION_INFO, deadline)
            .await
            .and_then(|response| parse_service_versions(&response))
        {
            Ok(versions) => versions,
            Err(error) => {
                result.reasons.push(native_error_code(error).to_string());
                return result;
            }
        };
        result.probe.available = true;
        result.probe.version_raw = Some(match versions.ctl_version {
            Some((major, minor)) => format!("native QMI · CTL {major}.{minor}"),
            None => "native QMI".to_string(),
        });
        result.probe.capabilities = ImsCapabilities {
            ims_settings: versions.ims,
            imsa_registration: versions.imsa,
            imsa_services: versions.imsa,
        };
        let mut blockers = Vec::new();

        if versions.nas {
            let nas = self
                .request_response(&device, QMI_SERVICE_NAS, QMI_NAS_GET_SYSTEM_INFO, deadline)
                .await;
            if let Ok(response) = nas {
                result.lte_voice_support = response.bool(0x21);
                result.ims_voice_support = response.bool(0x29);
                if result.ims_voice_support == Some(true) {
                    result
                        .evidence
                        .push("qmi_nas_ims_voice_support".to_string());
                }
            }
        }

        if versions.imsa {
            let services = self
                .request_response(
                    &device,
                    QMI_SERVICE_IMSA,
                    QMI_IMSA_GET_SERVICES_STATUS,
                    deadline,
                )
                .await;
            match services {
                Ok(response) => {
                    result.sms_service = qmi_imsa_service(response.u32(0x10));
                    result.voice_service = qmi_imsa_service(response.u32(0x11));
                    result.technology = qmi_imsa_technology(response.u32(0x13));
                    result.voice_technology = qmi_imsa_technology(response.u32(0x14));
                    if result.sms_service == ImsServiceStatus::Unknown {
                        blockers.push("ims_services_output_unrecognized".to_string());
                    } else {
                        result.evidence.push("qmi_imsa_services".to_string());
                    }
                    if result.voice_service == ImsServiceStatus::Unknown {
                        blockers.push("ims_voice_output_unrecognized".to_string());
                    } else {
                        result.evidence.push("qmi_imsa_voice".to_string());
                    }
                }
                Err(error) => {
                    blockers.push(native_query_error_code(error, "ims_services_query_failed"))
                }
            }

            let registration = self
                .request_response(
                    &device,
                    QMI_SERVICE_IMSA,
                    QMI_IMSA_GET_REGISTRATION_STATUS,
                    deadline,
                )
                .await;
            match registration {
                Ok(response) => {
                    result.registration = qmi_imsa_registration(response.u32(0x12));
                    if result.registration == ImsRegistration::Unknown {
                        blockers.push("ims_registration_output_unrecognized".to_string());
                    } else {
                        result.evidence.push("qmi_imsa_registration".to_string());
                    }
                }
                Err(error) => blockers.push(native_query_error_code(
                    error,
                    "ims_registration_query_failed",
                )),
            }
        } else {
            blockers.push("ims_services_query_unavailable".to_string());
            blockers.push("ims_registration_query_unavailable".to_string());
        }

        if versions.ims {
            let settings = self
                .request_response(
                    &device,
                    QMI_SERVICE_IMS,
                    QMI_IMS_GET_SERVICES_ENABLED,
                    deadline,
                )
                .await;
            match settings {
                Ok(response) => {
                    result.configured = qmi_configured(response.bool(0x1a));
                    result.volte_configured = qmi_configured(response.bool(0x11));
                    result.vowifi_configured = qmi_configured(response.bool(0x15));
                    if result.volte_configured == ImsConfigured::Unknown {
                        result
                            .warnings
                            .push("ims_volte_setting_unavailable".to_string());
                    }
                    if result.vowifi_configured == ImsConfigured::Unknown {
                        result
                            .warnings
                            .push("ims_vowifi_setting_unavailable".to_string());
                    }
                    if result.configured == ImsConfigured::Unknown {
                        result
                            .warnings
                            .push("ims_sms_setting_unavailable".to_string());
                    }
                    if result.volte_configured != ImsConfigured::Unknown
                        || result.vowifi_configured != ImsConfigured::Unknown
                        || result.configured != ImsConfigured::Unknown
                    {
                        result.evidence.push("qmi_ims_settings".to_string());
                    }
                }
                Err(error) => result
                    .warnings
                    .push(native_query_error_code(error, "ims_settings_query_failed")),
            }
        } else {
            result
                .warnings
                .push("ims_settings_query_unavailable".to_string());
        }
        result.classify();
        if result.status == SmsOverImsStatus::Unknown
            && result.voice_over_ims == VoiceOverImsStatus::Unknown
        {
            result.reasons = blockers;
        } else {
            result.warnings.extend(blockers);
        }
        result
    }
}

impl ImsProbe for NativeImsProbe {
    fn probe<'a>(
        &'a self,
        modem_output: &'a str,
        json: bool,
        enabled: Option<bool>,
    ) -> Pin<Box<dyn Future<Output = SmsOverIms> + Send + 'a>> {
        Box::pin(async move { self.probe_inner(modem_output, json, enabled).await })
    }
}

fn native_error_code(error: NativeQmiError) -> &'static str {
    match error {
        NativeQmiError::PermissionDenied => "ims_probe_permission_denied",
        NativeQmiError::ProxyUnavailable => "qmi_proxy_unavailable",
        NativeQmiError::Timeout => "ims_probe_timeout",
        NativeQmiError::Protocol | NativeQmiError::Qmi(_) => "native_qmi_probe_failed",
    }
}

fn native_query_error_code(error: NativeQmiError, failed_code: &str) -> String {
    match error {
        NativeQmiError::PermissionDenied => "ims_probe_permission_denied".to_string(),
        NativeQmiError::ProxyUnavailable => "qmi_proxy_unavailable".to_string(),
        NativeQmiError::Timeout => "ims_probe_timeout".to_string(),
        NativeQmiError::Protocol | NativeQmiError::Qmi(_) => failed_code.to_string(),
    }
}

pub trait ImsProbe: Send + Sync {
    fn probe<'a>(
        &'a self,
        modem_output: &'a str,
        json: bool,
        enabled: Option<bool>,
    ) -> Pin<Box<dyn Future<Output = SmsOverIms> + Send + 'a>>;
}

#[cfg(test)]
#[derive(Clone, Default)]
pub struct NoopImsProbe;

#[cfg(test)]
impl ImsProbe for NoopImsProbe {
    fn probe<'a>(
        &'a self,
        _modem_output: &'a str,
        _json: bool,
        _enabled: Option<bool>,
    ) -> Pin<Box<dyn Future<Output = SmsOverIms> + Send + 'a>> {
        Box::pin(async { SmsOverIms::default() })
    }
}

fn remaining_until(deadline: tokio::time::Instant) -> Option<Duration> {
    deadline.checked_duration_since(tokio::time::Instant::now())
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
        || !basename.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
        })
    {
        return None;
    }
    Some(basename.to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    use super::*;

    struct SocketPathGuard(PathBuf);

    impl Drop for SocketPathGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[derive(Clone)]
    struct FakeNativeQmiClient {
        responses: Arc<HashMap<(u8, u16), Vec<u8>>>,
    }

    impl QmiRequestClient for FakeNativeQmiClient {
        fn request<'a>(
            &'a self,
            _device: &'a str,
            service: u8,
            message: u16,
            _timeout: Duration,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, NativeQmiError>> + Send + 'a>> {
            Box::pin(async move {
                self.responses
                    .get(&(service, message))
                    .cloned()
                    .ok_or(NativeQmiError::Protocol)
            })
        }
    }

    #[derive(Clone)]
    struct FailingNativeQmiClient {
        error: NativeQmiError,
    }

    impl QmiRequestClient for FailingNativeQmiClient {
        fn request<'a>(
            &'a self,
            _device: &'a str,
            _service: u8,
            _message: u16,
            _timeout: Duration,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, NativeQmiError>> + Send + 'a>> {
            Box::pin(async move { Err(self.error) })
        }
    }

    async fn probe_with_startup_error(error: NativeQmiError) -> SmsOverIms {
        NativeImsProbe::with_client(FailingNativeQmiClient { error })
            .probe(
                r#"{"modem":{"generic":{"ports":["wwan0qmi0 (qmi)"]}}}"#,
                true,
                Some(true),
            )
            .await
    }

    #[tokio::test]
    async fn native_probe_reports_qmi_result_failure() {
        let failed_version_query = vec![
            0x01, 0x12, 0x00, 0x80, 0x00, 0x00, 0x01, 0x01, 0x21, 0x00, 0x07, 0x00, 0x02, 0x04,
            0x00, 0x01, 0x00, 0x47, 0x00,
        ];
        let probe = NativeImsProbe::with_client(FakeNativeQmiClient {
            responses: Arc::new(HashMap::from([(
                (QMI_SERVICE_CTL, QMI_CTL_GET_VERSION_INFO),
                failed_version_query,
            )])),
        });

        let status = probe
            .probe(
                r#"{"modem":{"generic":{"ports":["wwan0qmi0 (qmi)"]}}}"#,
                true,
                Some(true),
            )
            .await;

        assert!(!status.probe.available);
        assert_eq!(status.reasons, ["native_qmi_probe_failed"]);
    }

    #[tokio::test]
    async fn native_probe_reports_permission_denied() {
        let status = probe_with_startup_error(NativeQmiError::PermissionDenied).await;

        assert!(!status.probe.available);
        assert_eq!(status.reasons, ["ims_probe_permission_denied"]);
    }

    #[tokio::test]
    async fn native_probe_reports_unavailable_proxy() {
        let status = probe_with_startup_error(NativeQmiError::ProxyUnavailable).await;

        assert!(!status.probe.available);
        assert_eq!(status.reasons, ["qmi_proxy_unavailable"]);
    }

    #[tokio::test]
    async fn native_probe_reports_startup_timeout() {
        let status = probe_with_startup_error(NativeQmiError::Timeout).await;

        assert!(!status.probe.available);
        assert_eq!(status.reasons, ["ims_probe_timeout"]);
    }

    #[tokio::test]
    async fn native_probe_preserves_network_ims_voice_support_without_claiming_registration() {
        // Known QMUX responses derived from the QMI CTL Get Version Info and
        // NAS Get System Info wire formats. The modem exposes NAS 1.25 but no
        // IMS/IMSA services; NAS reports both LTE voice and IMS voice support.
        let service_versions = vec![
            0x01, 0x1b, 0x00, 0x80, 0x00, 0x00, 0x01, 0x01, 0x21, 0x00, 0x10, 0x00, 0x02, 0x04,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x06, 0x00, 0x01, 0x03, 0x01, 0x00, 0x19, 0x00,
        ];
        let system_info = vec![
            0x01, 0x1b, 0x00, 0x80, 0x03, 0x01, 0x02, 0x01, 0x00, 0x4d, 0x00, 0x0f, 0x00, 0x02,
            0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x21, 0x01, 0x00, 0x01, 0x29, 0x01, 0x00, 0x01,
        ];
        let probe = NativeImsProbe::with_client(FakeNativeQmiClient {
            responses: Arc::new(HashMap::from([
                (
                    (QMI_SERVICE_CTL, QMI_CTL_GET_VERSION_INFO),
                    service_versions,
                ),
                ((QMI_SERVICE_NAS, QMI_NAS_GET_SYSTEM_INFO), system_info),
            ])),
        });

        let status = probe
            .probe(
                r#"{"modem":{"generic":{"primary-port":"wwan0qmi0","ports":["wwan0qmi0 (qmi)"]}}}"#,
                true,
                Some(true),
            )
            .await;

        assert_eq!(status.probe.tool, "native-qmi");
        assert!(status.probe.available);
        assert_eq!(status.lte_voice_support, Some(true));
        assert_eq!(status.ims_voice_support, Some(true));
        assert_eq!(status.registration, ImsRegistration::Unknown);
        assert_eq!(status.voice_over_ims, VoiceOverImsStatus::Unknown);
        assert_eq!(status.status, SmsOverImsStatus::Unknown);
        assert!(status
            .evidence
            .contains(&"qmi_nas_ims_voice_support".to_string()));
        assert!(status
            .reasons
            .contains(&"ims_registration_query_unavailable".to_string()));
    }

    #[tokio::test]
    async fn native_probe_reports_volte_only_from_registered_available_wwan_voice_service() {
        let service_versions = vec![
            0x01, 0x25, 0x00, 0x80, 0x00, 0x00, 0x01, 0x01, 0x21, 0x00, 0x1a, 0x00, 0x02, 0x04,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x10, 0x00, 0x03, 0x03, 0x01, 0x00, 0x19, 0x00,
            0x12, 0x01, 0x00, 0x00, 0x00, 0x21, 0x01, 0x00, 0x00, 0x00,
        ];
        let system_info = vec![
            0x01, 0x1b, 0x00, 0x80, 0x03, 0x01, 0x02, 0x01, 0x00, 0x4d, 0x00, 0x0f, 0x00, 0x02,
            0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x21, 0x01, 0x00, 0x01, 0x29, 0x01, 0x00, 0x01,
        ];
        let services = vec![
            0x01, 0x2f, 0x00, 0x80, 0x21, 0x01, 0x02, 0x01, 0x00, 0x21, 0x00, 0x23, 0x00, 0x02,
            0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x04, 0x00, 0x02, 0x00, 0x00, 0x00, 0x11,
            0x04, 0x00, 0x02, 0x00, 0x00, 0x00, 0x13, 0x04, 0x00, 0x01, 0x00, 0x00, 0x00, 0x14,
            0x04, 0x00, 0x01, 0x00, 0x00, 0x00,
        ];
        let registration = vec![
            0x01, 0x21, 0x00, 0x80, 0x21, 0x01, 0x02, 0x01, 0x00, 0x20, 0x00, 0x15, 0x00, 0x02,
            0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x12, 0x04, 0x00, 0x02, 0x00, 0x00, 0x00, 0x14,
            0x04, 0x00, 0x01, 0x00, 0x00, 0x00,
        ];
        let settings = vec![
            0x01, 0x1f, 0x00, 0x80, 0x12, 0x01, 0x02, 0x01, 0x00, 0x90, 0x00, 0x13, 0x00, 0x02,
            0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x11, 0x01, 0x00, 0x01, 0x15, 0x01, 0x00, 0x00,
            0x1a, 0x01, 0x00, 0x01,
        ];
        let probe = NativeImsProbe::with_client(FakeNativeQmiClient {
            responses: Arc::new(HashMap::from([
                (
                    (QMI_SERVICE_CTL, QMI_CTL_GET_VERSION_INFO),
                    service_versions,
                ),
                ((QMI_SERVICE_NAS, QMI_NAS_GET_SYSTEM_INFO), system_info),
                ((QMI_SERVICE_IMSA, QMI_IMSA_GET_SERVICES_STATUS), services),
                (
                    (QMI_SERVICE_IMSA, QMI_IMSA_GET_REGISTRATION_STATUS),
                    registration,
                ),
                ((QMI_SERVICE_IMS, QMI_IMS_GET_SERVICES_ENABLED), settings),
            ])),
        });

        let status = probe
            .probe(
                r#"{"modem":{"generic":{"ports":["wwan0qmi0 (qmi)"]}}}"#,
                true,
                Some(true),
            )
            .await;

        assert_eq!(status.voice_over_ims, VoiceOverImsStatus::Volte);
        assert_eq!(status.registration, ImsRegistration::Registered);
        assert_eq!(status.voice_service, ImsServiceStatus::Available);
        assert_eq!(status.voice_technology, ImsTechnology::Wwan);
        assert_eq!(status.sms_service, ImsServiceStatus::Available);
        assert_eq!(status.technology, ImsTechnology::Wwan);
        assert_eq!(status.configured, ImsConfigured::Enabled);
        assert_eq!(status.volte_configured, ImsConfigured::Enabled);
        assert_eq!(status.vowifi_configured, ImsConfigured::Disabled);
        assert!(status.reasons.is_empty());
        assert!(status.evidence.contains(&"qmi_imsa_voice".to_string()));
    }

    #[tokio::test]
    async fn native_probe_keeps_partial_ims_configuration_as_evidence() {
        let service_versions = vec![
            0x01, 0x1b, 0x00, 0x80, 0x00, 0x00, 0x01, 0x01, 0x21, 0x00, 0x10, 0x00, 0x02, 0x04,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x06, 0x00, 0x01, 0x12, 0x01, 0x00, 0x00, 0x00,
        ];
        let settings = vec![
            0x01, 0x17, 0x00, 0x80, 0x12, 0x01, 0x02, 0x01, 0x00, 0x90, 0x00, 0x0b, 0x00, 0x02,
            0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x11, 0x01, 0x00, 0x01,
        ];
        let probe = NativeImsProbe::with_client(FakeNativeQmiClient {
            responses: Arc::new(HashMap::from([
                (
                    (QMI_SERVICE_CTL, QMI_CTL_GET_VERSION_INFO),
                    service_versions,
                ),
                ((QMI_SERVICE_IMS, QMI_IMS_GET_SERVICES_ENABLED), settings),
            ])),
        });

        let status = probe
            .probe(
                r#"{"modem":{"generic":{"ports":["wwan0qmi0 (qmi)"]}}}"#,
                true,
                Some(true),
            )
            .await;

        assert_eq!(status.volte_configured, ImsConfigured::Enabled);
        assert_eq!(status.vowifi_configured, ImsConfigured::Unknown);
        assert_eq!(status.configured, ImsConfigured::Unknown);
        assert!(status.evidence.contains(&"qmi_ims_settings".to_string()));
        assert!(status
            .warnings
            .contains(&"ims_vowifi_setting_unavailable".to_string()));
        assert!(status
            .warnings
            .contains(&"ims_sms_setting_unavailable".to_string()));
    }

    #[test]
    fn registered_available_wlan_voice_is_vowifi() {
        let mut status = SmsOverIms {
            registration: ImsRegistration::Registered,
            voice_service: ImsServiceStatus::Available,
            voice_technology: ImsTechnology::Wlan,
            ..SmsOverIms::default()
        };

        status.classify();

        assert_eq!(status.voice_over_ims, VoiceOverImsStatus::Vowifi);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn native_probe_preserves_network_support_when_cid_release_fails() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixListener;

        async fn read_frame(stream: &mut tokio::net::UnixStream) -> Vec<u8> {
            let mut prefix = [0_u8; 3];
            stream.read_exact(&mut prefix).await.unwrap();
            let length = usize::from(u16::from_le_bytes([prefix[1], prefix[2]])) + 1;
            let mut frame = vec![0_u8; length];
            frame[..3].copy_from_slice(&prefix);
            stream.read_exact(&mut frame[3..]).await.unwrap();
            frame
        }

        fn message_id(frame: &[u8]) -> u16 {
            let offset = if frame[4] == QMI_SERVICE_CTL { 8 } else { 9 };
            u16::from_le_bytes([frame[offset], frame[offset + 1]])
        }

        fn ctl_success(transaction: u8, message: u16, extra_tlvs: &[u8]) -> Vec<u8> {
            let tlv_length = 7 + extra_tlvs.len();
            let total = 1 + 5 + 6 + tlv_length;
            let mut frame = vec![
                0x01,
                (total - 1) as u8,
                0x00,
                0x80,
                QMI_SERVICE_CTL,
                0x00,
                0x01,
                transaction,
                message as u8,
                (message >> 8) as u8,
                tlv_length as u8,
                0x00,
                0x02,
                0x04,
                0x00,
                0x00,
                0x00,
                0x00,
                0x00,
            ];
            frame.extend_from_slice(extra_tlvs);
            frame
        }

        let socket_path = SocketPathGuard(PathBuf::from("/tmp").join(format!(
            "srq-{}.sock",
            &uuid::Uuid::new_v4().simple().to_string()[..12]
        )));
        let listener = UnixListener::bind(&socket_path.0).unwrap();
        let service_versions = vec![
            0x01, 0x1b, 0x00, 0x80, 0x00, 0x00, 0x01, 0x02, 0x21, 0x00, 0x10, 0x00, 0x02, 0x04,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x06, 0x00, 0x01, 0x03, 0x01, 0x00, 0x19, 0x00,
        ];
        let system_info = vec![
            0x01, 0x1b, 0x00, 0x80, 0x03, 0x01, 0x02, 0x01, 0x00, 0x4d, 0x00, 0x0f, 0x00, 0x02,
            0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x21, 0x01, 0x00, 0x01, 0x29, 0x01, 0x00, 0x01,
        ];
        let server = tokio::spawn(async move {
            let (mut ctl, _) = listener.accept().await.unwrap();
            let open = read_frame(&mut ctl).await;
            assert_eq!(message_id(&open), 0xff00);
            ctl.write_all(&ctl_success(open[7], 0xff00, &[]))
                .await
                .unwrap();
            let request = read_frame(&mut ctl).await;
            assert_eq!(
                (request[4], message_id(&request)),
                (QMI_SERVICE_CTL, 0x0021)
            );
            ctl.write_all(&service_versions).await.unwrap();
            drop(ctl);

            let (mut nas, _) = listener.accept().await.unwrap();
            let open = read_frame(&mut nas).await;
            nas.write_all(&ctl_success(open[7], 0xff00, &[]))
                .await
                .unwrap();
            let allocate = read_frame(&mut nas).await;
            assert_eq!(message_id(&allocate), 0x0022);
            nas.write_all(&ctl_success(
                allocate[7],
                0x0022,
                &[0x01, 0x02, 0x00, QMI_SERVICE_NAS, 0x01],
            ))
            .await
            .unwrap();
            let request = read_frame(&mut nas).await;
            assert_eq!(
                (request[4], message_id(&request)),
                (QMI_SERVICE_NAS, 0x004d)
            );
            nas.write_all(&system_info).await.unwrap();
            drop(nas);
        });

        let probe =
            NativeImsProbe::with_client(ProxyQmiClient::with_socket_path(socket_path.0.clone()));
        let status = probe
            .probe(
                r#"{"modem":{"generic":{"ports":["wwan0qmi0 (qmi)"]}}}"#,
                true,
                Some(true),
            )
            .await;

        server.await.unwrap();
        assert_eq!(status.ims_voice_support, Some(true));
        assert_eq!(status.voice_over_ims, VoiceOverImsStatus::Unknown);
    }

    #[test]
    fn registered_available_sms_is_available_over_wwan() {
        let mut status = SmsOverIms {
            registration: ImsRegistration::Registered,
            sms_service: ImsServiceStatus::Available,
            technology: ImsTechnology::Wwan,
            ..SmsOverIms::default()
        };

        status.classify();

        assert_eq!(status.status, SmsOverImsStatus::Available);
        assert_eq!(status.support, ImsSupport::Supported);
    }

    #[test]
    fn limited_evidence_takes_priority_over_registering() {
        let mut status = SmsOverIms {
            registration: ImsRegistration::Registering,
            sms_service: ImsServiceStatus::Limited,
            ..SmsOverIms::default()
        };

        status.classify();

        assert_eq!(status.status, SmsOverImsStatus::Limited);
    }

    #[test]
    fn registered_unavailable_sms_is_unavailable() {
        let mut status = SmsOverIms {
            registration: ImsRegistration::Registered,
            sms_service: ImsServiceStatus::Unavailable,
            ..SmsOverIms::default()
        };

        status.classify();

        assert_eq!(status.status, SmsOverImsStatus::Unavailable);
    }

    #[test]
    fn classifies_transitional_and_registration_states() {
        for (registration, sms_service, expected) in [
            (
                ImsRegistration::Registering,
                ImsServiceStatus::Unknown,
                SmsOverImsStatus::Registering,
            ),
            (
                ImsRegistration::Limited,
                ImsServiceStatus::Available,
                SmsOverImsStatus::Limited,
            ),
            (
                ImsRegistration::Registered,
                ImsServiceStatus::Limited,
                SmsOverImsStatus::Limited,
            ),
            (
                ImsRegistration::NotRegistered,
                ImsServiceStatus::Unknown,
                SmsOverImsStatus::NotRegistered,
            ),
        ] {
            let mut status = SmsOverIms {
                registration,
                sms_service,
                ..SmsOverIms::default()
            };
            status.classify();
            assert_eq!(status.status, expected);
        }
    }

    #[test]
    fn selects_sd410_primary_qmi_port_from_modem_json() {
        let raw = include_str!("../../tests/fixtures/mmcli/sd410-ports.json");

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

    #[test]
    fn rejects_unsafe_qmi_basenames_and_non_qmi_transports() {
        for port in [
            "wwan 0 (qmi)",
            "wwan:0 (qmi)",
            "wwan;0 (qmi)",
            "../wwan0 (qmi)",
        ] {
            let raw = format!(r#"{{"modem":{{"generic":{{"ports":["{port}"]}}}}}}"#);
            assert_eq!(
                select_qmi_device_from_modem_json(&raw),
                QmiPortSelection::Unavailable
            );
        }
        assert_eq!(
            select_qmi_device_from_modem_json(
                r#"{"modem":{"generic":{"ports":["cdc-wdm0 (mbim)","ttyUSB2 (at)"]}}}"#
            ),
            QmiPortSelection::Unavailable
        );
    }
}
