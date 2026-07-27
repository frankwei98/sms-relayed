use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

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
    let mut in_sms_section = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.eq_ignore_ascii_case("SMS:") {
            in_sms_section = true;
            continue;
        }
        if trimmed.ends_with(':') {
            in_sms_section = false;
            continue;
        }
        let Some((label, value)) = line.trim().split_once(':') else {
            continue;
        };
        let label = label.trim().to_ascii_lowercase();
        let value = normalized_value(value);
        if (in_sms_section && label == "status") || label == "ims sms service status" {
            parsed.nonstandard |= label == "ims sms service status";
            parsed.sms_service = match value.as_str() {
                "available" | "full service" => ImsSmsService::Available,
                "limited" | "limited service" => ImsSmsService::Limited,
                "unavailable" | "no service" => ImsSmsService::Unavailable,
                _ => ImsSmsService::Unknown,
            };
        } else if (in_sms_section && label == "technology")
            || label == "ims sms service rat"
            || label == "ims sms service technology"
        {
            parsed.nonstandard |= label != "technology";
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
    let mut in_registration_section = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.to_ascii_lowercase().ends_with("ims registration:") {
            in_registration_section = true;
            continue;
        }
        let Some((label, value)) = line.trim().split_once(':') else {
            continue;
        };
        let label = label.trim().to_ascii_lowercase();
        if !(in_registration_section && label == "status")
            && label != "ims registration status"
            && label != "registration status"
        {
            continue;
        }
        parsed.nonstandard |= label != "status";
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
        if label != "sms service enabled"
            && label != "ims sms service"
            && label != "ims sms enabled"
        {
            continue;
        }
        parsed.nonstandard |= label != "sms service enabled";
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

#[derive(Debug, Clone)]
pub struct QmicliOutput {
    pub stdout: String,
    pub status_success: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QmicliRunError {
    Missing,
    PathInvalid,
    PermissionDenied,
    ProxyUnavailable,
    Timeout,
    Failed,
}

pub trait QmicliRunner: Send + Sync {
    fn run<'a>(
        &'a self,
        args: &'a [String],
        timeout: Duration,
    ) -> Pin<Box<dyn Future<Output = Result<QmicliOutput, QmicliRunError>> + Send + 'a>>;
}

#[derive(Clone)]
pub struct RealQmicliRunner {
    program: PathBuf,
    path_valid: bool,
}

impl RealQmicliRunner {
    fn from_env() -> Self {
        match std::env::var_os("SMS_RELAYED_QMICLI_PATH") {
            Some(path) => {
                let path = PathBuf::from(path);
                let path_valid = path.is_absolute();
                Self {
                    program: path,
                    path_valid,
                }
            }
            None => Self {
                program: PathBuf::from("qmicli"),
                path_valid: true,
            },
        }
    }
}

impl QmicliRunner for RealQmicliRunner {
    fn run<'a>(
        &'a self,
        args: &'a [String],
        timeout: Duration,
    ) -> Pin<Box<dyn Future<Output = Result<QmicliOutput, QmicliRunError>> + Send + 'a>> {
        Box::pin(async move {
            if !self.path_valid {
                return Err(QmicliRunError::PathInvalid);
            }
            let mut command = Command::new(&self.program);
            command
                .args(args)
                .env("LC_ALL", "C")
                .kill_on_drop(true)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            let mut child = command.spawn().map_err(map_spawn_error)?;
            let stdout = child.stdout.take().ok_or(QmicliRunError::Failed)?;
            let stderr = child.stderr.take().ok_or(QmicliRunError::Failed)?;
            let (overflow_tx, mut overflow_rx) = tokio::sync::mpsc::channel(1);
            let mut stdout_task = tokio::spawn(read_capped(stdout, overflow_tx.clone()));
            let mut stderr_task = tokio::spawn(read_capped(stderr, overflow_tx.clone()));
            let deadline = tokio::time::Instant::now() + timeout;

            let status = tokio::time::timeout_at(deadline, async {
                tokio::select! {
                    status = child.wait() => status.map_err(|_| QmicliRunError::Failed),
                    Some(()) = overflow_rx.recv() => Err(QmicliRunError::Failed),
                }
            })
            .await;
            let status = match status {
                Ok(Ok(status)) => status,
                Ok(Err(error)) => {
                    terminate_child(&mut child);
                    abort_readers(&mut stdout_task, &mut stderr_task).await;
                    return Err(error);
                }
                Err(_) => {
                    terminate_child(&mut child);
                    abort_readers(&mut stdout_task, &mut stderr_task).await;
                    return Err(QmicliRunError::Timeout);
                }
            };
            drop(overflow_tx);
            let drained = tokio::time::timeout_at(deadline, async {
                let stdout = (&mut stdout_task)
                    .await
                    .map_err(|_| QmicliRunError::Failed)??;
                let stderr = (&mut stderr_task)
                    .await
                    .map_err(|_| QmicliRunError::Failed)??;
                Ok::<_, QmicliRunError>((stdout, stderr))
            })
            .await;
            let (stdout, stderr) = match drained {
                Ok(Ok(output)) => output,
                Ok(Err(error)) => {
                    terminate_child(&mut child);
                    abort_readers(&mut stdout_task, &mut stderr_task).await;
                    return Err(error);
                }
                Err(_) => {
                    terminate_child(&mut child);
                    abort_readers(&mut stdout_task, &mut stderr_task).await;
                    return Err(QmicliRunError::Timeout);
                }
            };
            let stderr = String::from_utf8_lossy(&stderr);
            if !status.success() {
                let lower = stderr.to_ascii_lowercase();
                if lower.contains("permission denied") || lower.contains("operation not permitted")
                {
                    return Err(QmicliRunError::PermissionDenied);
                }
                if lower.contains("qmi-proxy") || lower.contains("proxy") {
                    return Err(QmicliRunError::ProxyUnavailable);
                }
            }
            Ok(QmicliOutput {
                stdout: String::from_utf8_lossy(&stdout).to_string(),
                status_success: status.success(),
            })
        })
    }
}

const MAX_QMICLI_OUTPUT_BYTES: usize = 64 * 1024;

fn map_spawn_error(error: std::io::Error) -> QmicliRunError {
    if error.kind() == std::io::ErrorKind::NotFound {
        QmicliRunError::Missing
    } else if error.kind() == std::io::ErrorKind::PermissionDenied {
        QmicliRunError::PermissionDenied
    } else {
        QmicliRunError::Failed
    }
}

async fn read_capped<R>(
    mut reader: R,
    overflow: tokio::sync::mpsc::Sender<()>,
) -> Result<Vec<u8>, QmicliRunError>
where
    R: AsyncRead + Unpin,
{
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = reader
            .read(&mut buffer)
            .await
            .map_err(|_| QmicliRunError::Failed)?;
        if read == 0 {
            return Ok(output);
        }
        let remaining = MAX_QMICLI_OUTPUT_BYTES.saturating_sub(output.len());
        output.extend_from_slice(&buffer[..read.min(remaining)]);
        if read > remaining {
            let _ = overflow.try_send(());
            return Err(QmicliRunError::Failed);
        }
    }
}

fn terminate_child(child: &mut tokio::process::Child) {
    let _ = child.start_kill();
}

async fn abort_readers(
    stdout: &mut tokio::task::JoinHandle<Result<Vec<u8>, QmicliRunError>>,
    stderr: &mut tokio::task::JoinHandle<Result<Vec<u8>, QmicliRunError>>,
) {
    stdout.abort();
    stderr.abort();
    let _ = tokio::join!(stdout, stderr);
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

#[derive(Clone)]
struct ToolCapabilities {
    version: Option<String>,
    capabilities: ImsCapabilities,
}

#[derive(Clone)]
pub struct RealImsProbe {
    runner: Arc<dyn QmicliRunner>,
    tool: Arc<Mutex<Option<Result<ToolCapabilities, QmicliRunError>>>>,
    flight: Arc<tokio::sync::Mutex<()>>,
    last: Arc<Mutex<Option<(Instant, SmsOverIms)>>>,
}

impl RealImsProbe {
    pub fn new() -> Self {
        Self::with_runner(RealQmicliRunner::from_env())
    }

    pub fn with_runner<R>(runner: R) -> Self
    where
        R: QmicliRunner + 'static,
    {
        Self {
            runner: Arc::new(runner),
            tool: Arc::new(Mutex::new(None)),
            flight: Arc::new(tokio::sync::Mutex::new(())),
            last: Arc::new(Mutex::new(None)),
        }
    }

    async fn detect_tool(
        &self,
        deadline: tokio::time::Instant,
    ) -> Result<ToolCapabilities, QmicliRunError> {
        if let Some(cached) = self.tool.lock().unwrap().clone() {
            return cached;
        }
        let detected = async {
            let help = self
                .runner
                .run(
                    &["--help-all".to_string()],
                    remaining_until(deadline).ok_or(QmicliRunError::Timeout)?,
                )
                .await?;
            if !help.status_success {
                return Err(QmicliRunError::Failed);
            }
            let version = match remaining_until(deadline) {
                Some(remaining) => self
                    .runner
                    .run(
                        &["--version".to_string()],
                        remaining.min(Duration::from_millis(250)),
                    )
                    .await
                    .ok()
                    .filter(|output| output.status_success)
                    .and_then(|output| output.stdout.lines().next().map(ToString::to_string)),
                None => None,
            };
            Ok(ToolCapabilities {
                version,
                capabilities: ImsCapabilities {
                    ims_settings: help
                        .stdout
                        .contains("--ims-get-ims-services-enabled-setting"),
                    imsa_registration: help.stdout.contains("--imsa-get-ims-registration-status"),
                    imsa_services: help.stdout.contains("--imsa-get-ims-services-status"),
                },
            })
        };
        let detected = detected.await;
        if let Err(error) = detected.as_ref() {
            log::warn!(
                "SMS over IMS qmicli capability probe failed: {}",
                tool_error_code(*error)
            );
            return detected;
        }
        *self.tool.lock().unwrap() = Some(detected.clone());
        detected
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
        result.probe.transport = ImsTransport::DirectQmi;
        result.probe.device = Some(device.clone());
        result.reasons.clear();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let tool = match self.detect_tool(deadline).await {
            Ok(tool) => tool,
            Err(error) => {
                result.reasons.push(tool_error_code(error).to_string());
                return result;
            }
        };
        result.probe.available = true;
        result.probe.version_raw = tool.version;
        result.probe.capabilities = tool.capabilities.clone();

        let mut blockers = Vec::new();
        if tool.capabilities.imsa_services {
            match self
                .run_query(
                    &device,
                    "--imsa-get-ims-services-status",
                    deadline,
                    "ims_services_query_failed",
                )
                .await
            {
                Ok(output) => {
                    let parsed = parse_imsa_services(&output);
                    result.sms_service = parsed.sms_service;
                    result.technology = parsed.technology;
                    if parsed.sms_service == ImsSmsService::Unknown {
                        blockers.push("ims_services_output_unrecognized".to_string());
                    } else {
                        result.evidence.push("qmi_imsa_services".to_string());
                    }
                    if parsed.nonstandard {
                        result
                            .warnings
                            .push("ims_services_output_nonstandard".to_string());
                    }
                }
                Err(code) => blockers.push(code),
            }
        } else {
            blockers.push("ims_services_query_unavailable".to_string());
        }

        if tool.capabilities.imsa_registration {
            match self
                .run_query(
                    &device,
                    "--imsa-get-ims-registration-status",
                    deadline,
                    "ims_registration_query_failed",
                )
                .await
            {
                Ok(output) => {
                    let parsed = parse_imsa_registration(&output);
                    result.registration = parsed.registration;
                    if parsed.registration == ImsRegistration::Unknown {
                        blockers.push("ims_registration_output_unrecognized".to_string());
                    } else {
                        result.evidence.push("qmi_imsa_registration".to_string());
                    }
                    if parsed.nonstandard {
                        result
                            .warnings
                            .push("ims_registration_output_nonstandard".to_string());
                    }
                }
                Err(code) => blockers.push(code),
            }
        } else {
            blockers.push("ims_registration_query_unavailable".to_string());
        }

        if tool.capabilities.ims_settings {
            match self
                .run_query(
                    &device,
                    "--ims-get-ims-services-enabled-setting",
                    deadline,
                    "ims_settings_query_failed",
                )
                .await
            {
                Ok(output) => {
                    let parsed = parse_ims_settings(&output);
                    result.configured = parsed.configured;
                    if parsed.configured == ImsConfigured::Unknown {
                        result
                            .warnings
                            .push("ims_settings_output_unrecognized".to_string());
                    } else {
                        result.evidence.push("qmi_ims_settings".to_string());
                    }
                    if parsed.nonstandard {
                        result
                            .warnings
                            .push("ims_settings_output_nonstandard".to_string());
                    }
                }
                Err(code) => result.warnings.push(code),
            }
        } else {
            result
                .warnings
                .push("ims_settings_query_unavailable".to_string());
        }
        result.classify();
        if result.configured == ImsConfigured::Disabled
            && result.status == SmsOverImsStatus::Available
        {
            result.warnings.push("ims_state_inconsistent".to_string());
        }
        if result.status == SmsOverImsStatus::Unknown {
            result.reasons = blockers;
        } else {
            result.warnings.extend(blockers);
        }
        result
    }

    async fn run_query(
        &self,
        device: &str,
        action: &str,
        deadline: tokio::time::Instant,
        failed_code: &str,
    ) -> Result<String, String> {
        let timeout = deadline
            .checked_duration_since(tokio::time::Instant::now())
            .ok_or_else(|| "ims_probe_timeout".to_string())?;
        let args = vec![
            "-d".to_string(),
            device.to_string(),
            "--device-open-proxy".to_string(),
            action.to_string(),
        ];
        match self.runner.run(&args, timeout).await {
            Ok(output) if output.status_success => Ok(output.stdout),
            Ok(_) => Err(failed_code.to_string()),
            Err(error) => Err(query_error_code(error, failed_code).to_string()),
        }
    }
}

impl ImsProbe for RealImsProbe {
    fn probe<'a>(
        &'a self,
        modem_output: &'a str,
        json: bool,
        enabled: Option<bool>,
    ) -> Pin<Box<dyn Future<Output = SmsOverIms> + Send + 'a>> {
        Box::pin(async move {
            let requested_at = Instant::now();
            let _guard = self.flight.lock().await;
            if let Some((completed_at, result)) = self.last.lock().unwrap().clone() {
                if completed_at >= requested_at {
                    return result;
                }
            }
            let result = self.probe_inner(modem_output, json, enabled).await;
            *self.last.lock().unwrap() = Some((Instant::now(), result.clone()));
            result
        })
    }
}

fn tool_error_code(error: QmicliRunError) -> &'static str {
    match error {
        QmicliRunError::Missing => "qmicli_missing",
        QmicliRunError::PathInvalid => "qmicli_path_invalid",
        QmicliRunError::PermissionDenied => "ims_probe_permission_denied",
        QmicliRunError::ProxyUnavailable => "qmi_proxy_unavailable",
        QmicliRunError::Timeout => "ims_probe_timeout",
        QmicliRunError::Failed => "qmicli_probe_failed",
    }
}

fn query_error_code(error: QmicliRunError, failed_code: &str) -> &str {
    match error {
        QmicliRunError::PermissionDenied => "ims_probe_permission_denied",
        QmicliRunError::ProxyUnavailable => "qmi_proxy_unavailable",
        QmicliRunError::Timeout => "ims_probe_timeout",
        _ => failed_code,
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
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Clone)]
    struct FakeQmicliRunner {
        outputs: Arc<Mutex<VecDeque<QmicliOutput>>>,
    }

    impl QmicliRunner for FakeQmicliRunner {
        fn run<'a>(
            &'a self,
            _args: &'a [String],
            _timeout: std::time::Duration,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<QmicliOutput, QmicliRunError>> + Send + 'a>,
        > {
            Box::pin(async move {
                tokio::task::yield_now().await;
                self.outputs
                    .lock()
                    .unwrap()
                    .pop_front()
                    .ok_or(QmicliRunError::Failed)
            })
        }
    }

    #[derive(Clone)]
    struct ErrorQmicliRunner {
        error: QmicliRunError,
        calls: Arc<AtomicUsize>,
    }

    impl QmicliRunner for ErrorQmicliRunner {
        fn run<'a>(
            &'a self,
            _args: &'a [String],
            _timeout: Duration,
        ) -> Pin<Box<dyn Future<Output = Result<QmicliOutput, QmicliRunError>> + Send + 'a>>
        {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move { Err(self.error) })
        }
    }

    #[tokio::test]
    async fn probe_reports_available_from_qmi_runtime_evidence() {
        let outputs = [
            success(
                "--ims-get-ims-services-enabled-setting\n\
                 --imsa-get-ims-registration-status\n\
                 --imsa-get-ims-services-status",
            ),
            success("qmicli 1.36.0"),
            success("IMS SMS service status: 'available'\nIMS SMS service RAT: 'wwan'"),
            success("IMS registration status: 'registered'"),
            success("IMS SMS service: 'enabled'"),
        ];
        let probe = RealImsProbe::with_runner(FakeQmicliRunner {
            outputs: Arc::new(Mutex::new(outputs.into())),
        });

        let status = probe
            .probe(
                r#"{"modem":{"generic":{"primary-port":"wwan0qmi0","ports":["wwan0qmi0 (qmi)"]}}}"#,
                true,
                Some(true),
            )
            .await;

        assert_eq!(status.status, SmsOverImsStatus::Available);
        assert_eq!(status.configured, ImsConfigured::Enabled);
        assert_eq!(status.probe.device.as_deref(), Some("/dev/wwan0qmi0"));
        assert!(status.reasons.is_empty());
    }

    #[tokio::test]
    async fn old_qmicli_capabilities_report_unknown_without_running_queries() {
        let probe = RealImsProbe::with_runner(FakeQmicliRunner {
            outputs: Arc::new(Mutex::new(
                [success("--help-wms"), success("qmicli 1.32.2")].into(),
            )),
        });

        let status = probe
            .probe(
                r#"{"modem":{"generic":{"ports":["wwan0qmi0 (qmi)"]}}}"#,
                true,
                Some(true),
            )
            .await;

        assert_eq!(status.status, SmsOverImsStatus::Unknown);
        assert_eq!(status.probe.version_raw.as_deref(), Some("qmicli 1.32.2"));
        assert!(status
            .reasons
            .contains(&"ims_services_query_unavailable".to_string()));
        assert!(status
            .reasons
            .contains(&"ims_registration_query_unavailable".to_string()));
    }

    #[tokio::test]
    async fn version_failure_does_not_block_supported_ims_queries() {
        let outputs = [
            success(
                "--ims-get-ims-services-enabled-setting\n\
                 --imsa-get-ims-registration-status\n\
                 --imsa-get-ims-services-status",
            ),
            QmicliOutput {
                stdout: String::new(),
                status_success: false,
            },
            success(
                "SMS:\n\
                 \tStatus: 'full service'\n\
                 \tTechnology: 'wwan'\n",
            ),
            success("IMS registration:\n\tStatus: 'registered'\n"),
            success("SMS service enabled: yes\n"),
        ];
        let probe = RealImsProbe::with_runner(FakeQmicliRunner {
            outputs: Arc::new(Mutex::new(outputs.into())),
        });

        let status = probe
            .probe(
                r#"{"modem":{"generic":{"ports":["wwan0qmi0 (qmi)"]}}}"#,
                true,
                Some(true),
            )
            .await;

        assert_eq!(status.status, SmsOverImsStatus::Available);
        assert_eq!(status.probe.version_raw, None);
    }

    #[tokio::test]
    async fn disabled_modem_skips_port_selection_and_qmicli() {
        let calls = Arc::new(AtomicUsize::new(0));
        let probe = RealImsProbe::with_runner(ErrorQmicliRunner {
            error: QmicliRunError::Failed,
            calls: calls.clone(),
        });

        let status = probe.probe("{}", true, Some(false)).await;

        assert_eq!(status.status, SmsOverImsStatus::Unknown);
        assert_eq!(status.reasons, ["modem_disabled"]);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn capability_failures_return_fixed_safe_codes() {
        for (error, code) in [
            (QmicliRunError::Missing, "qmicli_missing"),
            (
                QmicliRunError::PermissionDenied,
                "ims_probe_permission_denied",
            ),
            (QmicliRunError::Timeout, "ims_probe_timeout"),
        ] {
            let probe = RealImsProbe::with_runner(ErrorQmicliRunner {
                error,
                calls: Arc::new(AtomicUsize::new(0)),
            });
            let status = probe
                .probe(
                    r#"{"modem":{"generic":{"ports":["wwan0qmi0 (qmi)"]}}}"#,
                    true,
                    Some(true),
                )
                .await;

            assert_eq!(status.status, SmsOverImsStatus::Unknown);
            assert_eq!(status.reasons, [code]);
        }
    }

    #[tokio::test]
    async fn query_failures_and_unrecognized_output_degrade_to_unknown() {
        let outputs = [
            success(
                "--ims-get-ims-services-enabled-setting\n\
                 --imsa-get-ims-registration-status\n\
                 --imsa-get-ims-services-status",
            ),
            success("qmicli 1.36.0"),
            QmicliOutput {
                stdout: String::new(),
                status_success: false,
            },
            success("unexpected registration output"),
            QmicliOutput {
                stdout: String::new(),
                status_success: false,
            },
        ];
        let probe = RealImsProbe::with_runner(FakeQmicliRunner {
            outputs: Arc::new(Mutex::new(outputs.into())),
        });

        let status = probe
            .probe(
                r#"{"modem":{"generic":{"ports":["wwan0qmi0 (qmi)"]}}}"#,
                true,
                Some(true),
            )
            .await;

        assert_eq!(status.status, SmsOverImsStatus::Unknown);
        assert!(status
            .reasons
            .contains(&"ims_services_query_failed".to_string()));
        assert!(status
            .reasons
            .contains(&"ims_registration_output_unrecognized".to_string()));
        assert!(status
            .warnings
            .contains(&"ims_settings_query_failed".to_string()));
    }

    #[tokio::test]
    async fn capped_reader_signals_and_retains_only_the_output_limit() {
        let (mut writer, reader) = tokio::io::duplex(MAX_QMICLI_OUTPUT_BYTES * 2);
        let payload = vec![b'x'; MAX_QMICLI_OUTPUT_BYTES + 1];
        let writer_task = tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            writer.write_all(&payload).await.unwrap();
        });
        let (overflow_tx, mut overflow_rx) = tokio::sync::mpsc::channel(1);

        let output = read_capped(reader, overflow_tx).await;

        writer_task.await.unwrap();
        assert_eq!(output, Err(QmicliRunError::Failed));
        assert_eq!(overflow_rx.recv().await, Some(()));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn runner_deadline_includes_pipe_drain_after_child_exit() {
        let runner = RealQmicliRunner {
            program: PathBuf::from("/bin/sh"),
            path_valid: true,
        };
        let args = vec!["-c".to_string(), "(sleep 2) & exit 0".to_string()];
        let started = Instant::now();

        let result = runner.run(&args, Duration::from_millis(50)).await;

        assert!(matches!(result, Err(QmicliRunError::Timeout)));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn concurrent_requests_share_the_in_flight_probe_result() {
        let outputs = [
            success(
                "--ims-get-ims-services-enabled-setting\n\
                 --imsa-get-ims-registration-status\n\
                 --imsa-get-ims-services-status",
            ),
            success("qmicli 1.36.0"),
            success("IMS SMS service status: 'available'\nIMS SMS service RAT: 'wwan'"),
            success("IMS registration status: 'registered'"),
            success("IMS SMS service: 'enabled'"),
        ];
        let probe = RealImsProbe::with_runner(FakeQmicliRunner {
            outputs: Arc::new(Mutex::new(outputs.into())),
        });
        let raw =
            r#"{"modem":{"generic":{"primary-port":"wwan0qmi0","ports":["wwan0qmi0 (qmi)"]}}}"#;

        let (first, second) = tokio::join!(
            probe.probe(raw, true, Some(true)),
            probe.probe(raw, true, Some(true))
        );

        assert_eq!(first.status, SmsOverImsStatus::Available);
        assert_eq!(second.status, SmsOverImsStatus::Available);
    }

    fn success(stdout: &str) -> QmicliOutput {
        QmicliOutput {
            stdout: stdout.to_string(),
            status_success: true,
        }
    }

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
    fn classifies_transitional_and_registration_states() {
        for (registration, sms_service, expected) in [
            (
                ImsRegistration::Registering,
                ImsSmsService::Unknown,
                SmsOverImsStatus::Registering,
            ),
            (
                ImsRegistration::Limited,
                ImsSmsService::Available,
                SmsOverImsStatus::Limited,
            ),
            (
                ImsRegistration::Registered,
                ImsSmsService::Limited,
                SmsOverImsStatus::Limited,
            ),
            (
                ImsRegistration::NotRegistered,
                ImsSmsService::Unknown,
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
    fn parses_available_sms_service_over_wlan() {
        let parsed = parse_imsa_services(
            "[/dev/wwan0qmi0] IMS services:\n\
             \tSMS:\n\
             \t\tStatus: 'full service'\n\
             \t\tTechnology: 'wlan'\n\
             \tVoice:\n\
             \t\tStatus: 'no service'\n\
             \t\tTechnology: 'wwan'\n",
        );

        assert_eq!(parsed.sms_service, ImsSmsService::Available);
        assert_eq!(parsed.technology, ImsTechnology::Wlan);
        assert!(!parsed.nonstandard);
    }

    #[test]
    fn parses_registration_and_sms_enabled_setting() {
        assert_eq!(
            parse_imsa_registration(
                "[/dev/wwan0qmi0] IMS registration:\n\
                 \t    Status: 'registered'\n\
                 \tTechnology: 'wwan'\n"
            )
            .registration,
            ImsRegistration::Registered
        );
        assert_eq!(
            parse_ims_settings(
                "[/dev/wwan0qmi0] IMS services:\n\
                 \tVoice service enabled: no\n\
                 \tSMS service enabled: yes\n"
            )
            .configured,
            ImsConfigured::Enabled
        );
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
