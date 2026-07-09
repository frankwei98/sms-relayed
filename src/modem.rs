use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;
use time::OffsetDateTime;
use tokio::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthLevel {
    Ok,
    Degraded,
    Error,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolInfo {
    pub available: bool,
    pub version_raw: Option<String>,
    pub supports_json: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedModem {
    pub present: bool,
    pub id: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthSummary {
    pub status: HealthLevel,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModemDetails {
    pub enabled: Option<bool>,
    pub state: Option<String>,
    pub sim_state: Option<String>,
    pub operator_name: Option<String>,
    pub signal_quality: Option<u8>,
    pub access_technologies: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MessagingDetails {
    pub available: bool,
    pub supported_storages: Vec<String>,
    pub default_storage: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Diagnostics {
    pub last_error: Option<String>,
    pub path_drift_candidate: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModemStatus {
    #[serde(with = "time::serde::rfc3339")]
    pub checked_at: OffsetDateTime,
    pub tool: ToolInfo,
    pub configured_modem_path: String,
    pub resolved: ResolvedModem,
    pub health: HealthSummary,
    pub modem: ModemDetails,
    pub messaging: MessagingDetails,
    pub diagnostics: Diagnostics,
}

#[derive(Debug, Clone)]
pub struct ModemError {
    code: &'static str,
    message: String,
}

impl ModemError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for ModemError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ModemError {}

fn base_status(configured_path: &str, id: Option<String>) -> ModemStatus {
    ModemStatus {
        checked_at: OffsetDateTime::now_utc(),
        tool: ToolInfo {
            available: true,
            version_raw: None,
            supports_json: true,
        },
        configured_modem_path: configured_path.to_string(),
        resolved: ResolvedModem {
            present: true,
            id,
            path: Some(configured_path.to_string()),
        },
        health: HealthSummary {
            status: HealthLevel::Unknown,
            reasons: Vec::new(),
        },
        modem: ModemDetails {
            enabled: None,
            state: None,
            sim_state: None,
            operator_name: None,
            signal_quality: None,
            access_technologies: Vec::new(),
        },
        messaging: MessagingDetails {
            available: false,
            supported_storages: Vec::new(),
            default_storage: None,
        },
        diagnostics: Diagnostics {
            last_error: None,
            path_drift_candidate: None,
        },
    }
}

pub fn parse_modem_json(
    configured_path: &str,
    id: Option<String>,
    raw: &str,
) -> Result<ModemStatus, ModemError> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|e| ModemError::new("mmcli_parse_failed", e.to_string()))?;
    let modem = value
        .get("modem")
        .ok_or_else(|| ModemError::new("mmcli_parse_failed", "missing modem object"))?;
    let mut status = base_status(configured_path, id);

    if let Some(path) = get_str(modem, &["generic", "dbus-path"]) {
        status.resolved.path = Some(path);
        status.resolved.present = status.resolved.path.as_deref() == Some(configured_path);
    }
    status.modem.state = get_str(modem, &["generic", "state"]);
    status.modem.enabled = status
        .modem
        .state
        .as_deref()
        .map(|s| !matches!(s, "disabled" | "failed" | "locked" | "unavailable"));
    status.modem.sim_state = get_str(modem, &["sim", "state"]);
    status.modem.operator_name = get_str(modem, &["3gpp", "operator-name"]);
    status.modem.signal_quality = modem
        .pointer("/generic/signal-quality/value")
        .and_then(|v| v.as_u64())
        .and_then(|n| u8::try_from(n.min(100)).ok());
    status.modem.access_technologies = modem
        .pointer("/generic/access-technologies")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default();
    status.messaging.supported_storages = modem
        .pointer("/messaging/supported-storages")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default();
    status.messaging.default_storage = get_str(modem, &["messaging", "default-storage"]);
    status.messaging.available = !status.messaging.supported_storages.is_empty();
    classify(&mut status);
    Ok(status)
}

pub fn parse_modem_text(
    configured_path: &str,
    id: Option<String>,
    raw: &str,
) -> Result<ModemStatus, ModemError> {
    let mut status = base_status(configured_path, id);
    status.tool.supports_json = false;
    status
        .health
        .reasons
        .push("text_fallback_limited".to_string());
    let mut section = "";

    for line in raw.lines() {
        let line = line.trim();
        let Some((left, right)) = line.split_once('|') else {
            continue;
        };
        let left = left.trim();
        if !left.is_empty() {
            section = left;
        }
        let right = right.trim();
        if let Some(value) = right.strip_prefix("path:") {
            status.resolved.path = Some(value.trim().to_string());
            status.resolved.present = status.resolved.path.as_deref() == Some(configured_path);
        } else if let Some(value) = right.strip_prefix("state:") {
            match section {
                "SIM" => status.modem.sim_state = Some(value.trim().to_string()),
                _ => status.modem.state = Some(value.trim().to_string()),
            }
        } else if let Some(value) = right.strip_prefix("operator name:") {
            status.modem.operator_name = Some(value.trim().to_string());
        } else if let Some(value) = right.strip_prefix("supported storages:") {
            status.messaging.supported_storages = split_csv(value);
            status.messaging.available = !status.messaging.supported_storages.is_empty();
        } else if let Some(value) = right.strip_prefix("default storage:") {
            status.messaging.default_storage = Some(value.trim().to_string());
        } else if let Some(value) = right.strip_prefix("quality:") {
            status.modem.signal_quality = value
                .trim()
                .split('%')
                .next()
                .and_then(|n| n.trim().parse::<u8>().ok());
        } else if let Some(value) = right
            .strip_prefix("access techologies:")
            .or_else(|| right.strip_prefix("access technologies:"))
        {
            status.modem.access_technologies = split_csv(value);
        }
    }
    status.modem.enabled = status
        .modem
        .state
        .as_deref()
        .map(|s| !matches!(s, "disabled" | "failed" | "locked" | "unavailable"));
    classify(&mut status);
    Ok(status)
}

pub fn classify(status: &mut ModemStatus) {
    if !status.resolved.present {
        status.health.status = HealthLevel::Error;
        push_reason(&mut status.health.reasons, "modem_path_unresolved");
        return;
    }
    if status.modem.enabled == Some(false) {
        status.health.status = HealthLevel::Degraded;
        push_reason(&mut status.health.reasons, "modem_disabled");
    }
    if let Some(sim) = status.modem.sim_state.as_deref() {
        if sim != "ready" {
            status.health.status = HealthLevel::Degraded;
            push_reason(&mut status.health.reasons, "sim_not_ready");
        }
    }
    if !status.messaging.available {
        status.health.status = HealthLevel::Degraded;
        push_reason(&mut status.health.reasons, "messaging_unavailable");
    }
    if let Some(state) = status.modem.state.as_deref() {
        if matches!(state, "failed" | "locked" | "unavailable") {
            status.health.status = HealthLevel::Degraded;
            push_reason(&mut status.health.reasons, "modem_state_unhealthy");
        }
    }
    if status.health.status == HealthLevel::Unknown {
        let usable = matches!(
            status.modem.state.as_deref(),
            Some("registered" | "connected")
        );
        if usable
            && status.modem.enabled != Some(false)
            && status.modem.sim_state.as_deref() == Some("ready")
            && status.messaging.available
        {
            status.health.status = HealthLevel::Ok;
        } else {
            status.health.status = HealthLevel::Degraded;
            push_reason(&mut status.health.reasons, "modem_state_unhealthy");
        }
    }
}

pub fn map_list_path_to_id(configured_path: &str, list_output: &str) -> Result<String, ModemError> {
    let mut matches = Vec::new();
    for line in list_output.lines() {
        let line = line.trim();
        let Some(path) = line.split_whitespace().next() else {
            continue;
        };
        if path != configured_path {
            continue;
        }
        let Some(id) = path
            .split("/Modem/")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
        else {
            continue;
        };
        matches.push(id.to_string());
    }
    match matches.len() {
        1 => Ok(matches.remove(0)),
        _ => Err(ModemError::new(
            "modem_path_unresolved",
            "configured modem path was not found exactly once",
        )),
    }
}

pub fn path_drift_candidate(configured_path: &str, list_output: &str) -> Option<String> {
    let paths: Vec<String> = list_output
        .lines()
        .filter_map(|line| line.trim().split_whitespace().next())
        .filter(|path| path.starts_with("/org/freedesktop/ModemManager1/Modem/"))
        .filter(|path| *path != configured_path)
        .map(ToString::to_string)
        .collect();
    match paths.as_slice() {
        [only] => Some(only.clone()),
        _ => None,
    }
}

fn get_str(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str().map(ToString::to_string)
}

fn push_reason(reasons: &mut Vec<String>, reason: &str) {
    if !reasons.iter().any(|r| r == reason) {
        reasons.push(reason.to_string());
    }
}

fn split_csv(value: &str) -> Vec<String> {
    value
        .trim()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .collect()
}

#[derive(Debug, Clone)]
pub struct MmcliOutput {
    pub stdout: String,
    pub stderr: String,
    pub status_success: bool,
}

pub trait MmcliRunner: Send + Sync {
    fn run<'a>(
        &'a self,
        args: &'a [&'a str],
        timeout: std::time::Duration,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<MmcliOutput, ModemError>> + Send + 'a>,
    >;
}

#[derive(Debug, Clone, Copy)]
pub enum ModemAction {
    Enable,
    Disable,
    Reset,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActionResponse {
    pub accepted: bool,
    pub action: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicModemHealth {
    pub status: HealthLevel,
    pub reason: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub checked_at: OffsetDateTime,
}

const MMCLI_TIMEOUT: Duration = Duration::from_secs(5);
const HEALTH_CACHE_TTL: Duration = Duration::from_secs(5);
const RESET_RATE_LIMIT: Duration = Duration::from_secs(60);

#[derive(Clone, Default)]
pub struct RealMmcliRunner;

impl MmcliRunner for RealMmcliRunner {
    fn run<'a>(
        &'a self,
        args: &'a [&'a str],
        timeout: Duration,
    ) -> Pin<Box<dyn Future<Output = Result<MmcliOutput, ModemError>> + Send + 'a>> {
        Box::pin(async move {
            let mut command = Command::new("mmcli");
            command.args(args);
            let output = tokio::time::timeout(timeout, command.output())
                .await
                .map_err(|_| ModemError::new("mmcli_timeout", "mmcli command timed out"))?
                .map_err(|e| ModemError::new("mmcli_probe_failed", e.to_string()))?;
            Ok(MmcliOutput {
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr)
                    .chars()
                    .take(300)
                    .collect(),
                status_success: output.status.success(),
            })
        })
    }
}

impl ModemAction {
    pub fn name(self) -> &'static str {
        match self {
            ModemAction::Enable => "enable",
            ModemAction::Disable => "disable",
            ModemAction::Reset => "reset",
        }
    }

    fn flag(self) -> &'static str {
        match self {
            ModemAction::Enable => "--enable",
            ModemAction::Disable => "--disable",
            ModemAction::Reset => "--reset",
        }
    }
}

#[derive(Clone)]
pub struct ModemService {
    runner: Arc<dyn MmcliRunner>,
    capabilities: Arc<Mutex<Option<ToolInfo>>>,
    health_cache: Arc<Mutex<Option<(String, Instant, PublicModemHealth)>>>,
    pub(crate) action_lock: Arc<tokio::sync::Mutex<()>>,
    reset_limits: Arc<Mutex<HashMap<String, Instant>>>,
}

impl ModemService {
    pub fn new() -> Self {
        Self::new_with_runner(RealMmcliRunner)
    }

    pub fn new_with_runner<R>(runner: R) -> Self
    where
        R: MmcliRunner + 'static,
    {
        Self {
            runner: Arc::new(runner),
            capabilities: Arc::new(Mutex::new(None)),
            health_cache: Arc::new(Mutex::new(None)),
            action_lock: Arc::new(tokio::sync::Mutex::new(())),
            reset_limits: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn status(&self, configured_path: &str) -> ModemStatus {
        let tool = self.detect_capabilities().await;
        if !tool.available {
            return unavailable_status(configured_path, tool, "mmcli_probe_failed");
        }

        let args = if tool.supports_json {
            vec!["--modem", configured_path, "--output-json"]
        } else {
            vec!["--modem", configured_path]
        };

        match self.runner.run(&args, MMCLI_TIMEOUT).await {
            Ok(output) if output.status_success => {
                let parsed = if tool.supports_json {
                    parse_modem_json(configured_path, path_id(configured_path), &output.stdout)
                } else {
                    parse_modem_text(configured_path, path_id(configured_path), &output.stdout)
                };
                parsed
                    .map(|mut status| {
                        status.tool = tool.clone();
                        status
                    })
                    .unwrap_or_else(|err| {
                        error_status(configured_path, tool, err.code(), err.to_string())
                    })
            }
            Ok(output) => {
                let resolved = self.resolve_id(configured_path).await;
                match resolved {
                    Ok(id) => {
                        let retry_args = if tool.supports_json {
                            vec!["--modem", id.as_str(), "--output-json"]
                        } else {
                            vec!["--modem", id.as_str()]
                        };
                        match self.runner.run(&retry_args, MMCLI_TIMEOUT).await {
                            Ok(retry) if retry.status_success => {
                                let parsed = if tool.supports_json {
                                    parse_modem_json(configured_path, Some(id), &retry.stdout)
                                } else {
                                    parse_modem_text(configured_path, Some(id), &retry.stdout)
                                };
                                parsed.unwrap_or_else(|err| {
                                    error_status(configured_path, tool, err.code(), err.to_string())
                                })
                            }
                            _ => error_status(
                                configured_path,
                                tool,
                                "modem_path_unresolved",
                                output.stderr,
                            ),
                        }
                    }
                    Err(err) => {
                        let mut status =
                            error_status(configured_path, tool, err.code(), err.to_string());
                        if let Ok(candidate) = self.path_drift_candidate(configured_path).await {
                            status.health.status = HealthLevel::Degraded;
                            status.health.reasons.clear();
                            status
                                .health
                                .reasons
                                .push("modem_path_drift_candidate".to_string());
                            status.diagnostics.path_drift_candidate = Some(candidate);
                        }
                        status
                    }
                }
            }
            Err(err) => error_status(configured_path, tool, err.code(), err.to_string()),
        }
    }

    pub async fn public_health(&self, configured_path: &str) -> PublicModemHealth {
        if let Some((path, at, cached)) = self.health_cache.lock().unwrap().clone() {
            if path == configured_path && at.elapsed() < HEALTH_CACHE_TTL {
                return cached;
            }
        }

        let status = self.status(configured_path).await;
        let health = PublicModemHealth {
            status: status.health.status,
            reason: status.health.reasons.first().cloned(),
            checked_at: status.checked_at,
        };
        *self.health_cache.lock().unwrap() =
            Some((configured_path.to_string(), Instant::now(), health.clone()));
        health
    }

    pub async fn run_action(
        &self,
        configured_path: &str,
        session_token: &str,
        action: ModemAction,
    ) -> Result<ActionResponse, ModemError> {
        let Ok(_guard) = self.action_lock.try_lock() else {
            return Err(ModemError::new(
                "action_in_progress",
                "another modem action is running",
            ));
        };
        if matches!(action, ModemAction::Reset) {
            self.can_reset(session_token).await?;
        }
        let tool = self.detect_capabilities().await;
        if !tool.available {
            return Err(ModemError::new("mmcli_missing", "mmcli is not available"));
        }
        let id = self.resolve_target(configured_path, &tool).await?;
        let args = ["--modem", id.as_str(), action.flag()];
        let output = self.runner.run(&args, MMCLI_TIMEOUT).await?;
        if !output.status_success {
            return Err(ModemError::new("modem_action_failed", output.stderr));
        }
        log::info!(
            "modem action={} session={} result=accepted",
            action.name(),
            short_hash(session_token)
        );
        Ok(ActionResponse {
            accepted: true,
            action: action.name(),
        })
    }

    pub async fn can_reset(&self, session_token: &str) -> Result<(), ModemError> {
        let key = short_hash(session_token);
        let mut guard = self.reset_limits.lock().unwrap();
        guard.retain(|_, last| last.elapsed() < RESET_RATE_LIMIT);
        if let Some(last) = guard.get(&key) {
            if last.elapsed() < RESET_RATE_LIMIT {
                return Err(ModemError::new(
                    "reset_rate_limited",
                    "reset is rate limited",
                ));
            }
        }
        guard.insert(key, Instant::now());
        Ok(())
    }

    async fn detect_capabilities(&self) -> ToolInfo {
        if let Some(cached) = self.capabilities.lock().unwrap().clone() {
            return cached;
        }
        let tool = match self.runner.run(&["--version"], MMCLI_TIMEOUT).await {
            Ok(output) if output.status_success => ToolInfo {
                available: true,
                version_raw: Some(output.stdout.trim().to_string()),
                supports_json: true,
            },
            _ => {
                return ToolInfo {
                    available: false,
                    version_raw: None,
                    supports_json: false,
                };
            }
        };
        *self.capabilities.lock().unwrap() = Some(tool.clone());
        tool
    }

    async fn resolve_target(
        &self,
        configured_path: &str,
        _tool: &ToolInfo,
    ) -> Result<String, ModemError> {
        if self
            .runner
            .run(&["--modem", configured_path], MMCLI_TIMEOUT)
            .await
            .map(|o| o.status_success)
            .unwrap_or(false)
        {
            return Ok(configured_path.to_string());
        }
        self.resolve_id(configured_path).await
    }

    async fn resolve_id(&self, configured_path: &str) -> Result<String, ModemError> {
        let output = self.runner.run(&["-L"], MMCLI_TIMEOUT).await?;
        if !output.status_success {
            return Err(ModemError::new("modem_path_unresolved", output.stderr));
        }
        map_list_path_to_id(configured_path, &output.stdout)
    }

    async fn path_drift_candidate(&self, configured_path: &str) -> Result<String, ModemError> {
        let output = self.runner.run(&["-L"], MMCLI_TIMEOUT).await?;
        if !output.status_success {
            return Err(ModemError::new("modem_path_lost", output.stderr));
        }
        path_drift_candidate(configured_path, &output.stdout)
            .ok_or_else(|| ModemError::new("modem_path_lost", "no single drift candidate"))
    }
}

fn unavailable_status(configured_path: &str, tool: ToolInfo, reason: &str) -> ModemStatus {
    let mut status = base_status(configured_path, None);
    status.tool = tool;
    status.resolved.present = false;
    status.resolved.path = None;
    status.health.status = HealthLevel::Unknown;
    status.health.reasons.push(reason.to_string());
    status
}

fn error_status(configured_path: &str, tool: ToolInfo, code: &str, message: String) -> ModemStatus {
    let mut status = base_status(configured_path, None);
    status.tool = tool;
    status.resolved.present = false;
    status.resolved.path = None;
    status.health.status = HealthLevel::Error;
    status.health.reasons.push(code.to_string());
    status.diagnostics.last_error = Some(message.chars().take(300).collect());
    status
}

fn path_id(path: &str) -> Option<String> {
    path.rsplit("/Modem/").next().map(ToString::to_string)
}

fn short_hash(value: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(value.as_bytes());
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, &digest)[..12]
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATH: &str = "/org/freedesktop/ModemManager1/Modem/0";

    #[test]
    fn parses_healthy_json_as_ok() {
        let raw = include_str!("../tests/fixtures/mmcli/healthy.json");
        let status = parse_modem_json(PATH, Some("0".to_string()), raw).unwrap();

        assert_eq!(status.health.status, HealthLevel::Ok);
        assert_eq!(status.resolved.id.as_deref(), Some("0"));
        assert_eq!(status.resolved.path.as_deref(), Some(PATH));
        assert_eq!(status.modem.enabled, Some(true));
        assert_eq!(status.modem.state.as_deref(), Some("registered"));
        assert_eq!(status.modem.sim_state.as_deref(), Some("ready"));
        assert_eq!(status.modem.operator_name.as_deref(), Some("CHN-UNICOM"));
        assert_eq!(status.modem.signal_quality, Some(78));
        assert_eq!(status.modem.access_technologies, vec!["lte"]);
        assert!(status.messaging.available);
    }

    #[test]
    fn classifies_sim_missing_as_degraded() {
        let raw = include_str!("../tests/fixtures/mmcli/sim-missing.json");
        let status = parse_modem_json(PATH, Some("0".to_string()), raw).unwrap();

        assert_eq!(status.health.status, HealthLevel::Degraded);
        assert!(status.health.reasons.contains(&"sim_not_ready".to_string()));
    }

    #[test]
    fn classifies_disabled_modem_as_degraded() {
        let raw = include_str!("../tests/fixtures/mmcli/disabled.json");
        let status = parse_modem_json(PATH, Some("0".to_string()), raw).unwrap();

        assert_eq!(status.health.status, HealthLevel::Degraded);
        assert_eq!(status.modem.enabled, Some(false));
        assert!(status
            .health
            .reasons
            .contains(&"modem_disabled".to_string()));
    }

    #[test]
    fn classifies_missing_messaging_as_degraded() {
        let raw = include_str!("../tests/fixtures/mmcli/messaging-missing.json");
        let status = parse_modem_json(PATH, Some("0".to_string()), raw).unwrap();

        assert_eq!(status.health.status, HealthLevel::Degraded);
        assert!(!status.messaging.available);
        assert!(status
            .health
            .reasons
            .contains(&"messaging_unavailable".to_string()));
    }

    #[test]
    fn limited_text_fallback_marks_limited_reason() {
        let raw = include_str!("../tests/fixtures/mmcli/text-registered.txt");
        let status = parse_modem_text(PATH, Some("0".to_string()), raw).unwrap();

        assert_eq!(status.resolved.path.as_deref(), Some(PATH));
        assert_eq!(status.modem.state.as_deref(), Some("registered"));
        assert_eq!(status.modem.sim_state.as_deref(), Some("ready"));
        assert_eq!(status.modem.signal_quality, Some(78));
        assert!(status
            .health
            .reasons
            .contains(&"text_fallback_limited".to_string()));
    }

    #[test]
    fn maps_exact_object_path_to_modem_id() {
        let raw = include_str!("../tests/fixtures/mmcli/modem-list.txt");
        assert_eq!(map_list_path_to_id(PATH, raw).unwrap(), "0");
        assert_eq!(
            map_list_path_to_id("/org/freedesktop/ModemManager1/Modem/3", raw).unwrap(),
            "3"
        );
    }

    #[test]
    fn rejects_missing_path_mapping() {
        let raw = include_str!("../tests/fixtures/mmcli/modem-list.txt");
        let err = map_list_path_to_id("/org/freedesktop/ModemManager1/Modem/99", raw).unwrap_err();
        assert_eq!(err.code(), "modem_path_unresolved");
    }

    #[test]
    fn reports_single_path_drift_candidate() {
        let raw = "/org/freedesktop/ModemManager1/Modem/7 [Quectel] EC25\n";
        assert_eq!(
            path_drift_candidate("/org/freedesktop/ModemManager1/Modem/0", raw).as_deref(),
            Some("/org/freedesktop/ModemManager1/Modem/7")
        );
    }

    #[test]
    fn does_not_prefix_match_modem_ids() {
        let raw = "/org/freedesktop/ModemManager1/Modem/03 [Quectel] EC25\n";
        let err = map_list_path_to_id("/org/freedesktop/ModemManager1/Modem/0", raw).unwrap_err();
        assert_eq!(err.code(), "modem_path_unresolved");
    }
}

#[cfg(test)]
mod service_tests {
    use super::*;
    use std::collections::VecDeque;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[derive(Clone)]
    struct FakeRunner {
        calls: Arc<Mutex<Vec<Vec<String>>>>,
        outputs: Arc<Mutex<VecDeque<Result<MmcliOutput, ModemError>>>>,
    }

    impl FakeRunner {
        fn new(outputs: Vec<Result<MmcliOutput, ModemError>>) -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                outputs: Arc::new(Mutex::new(outputs.into())),
            }
        }

        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl MmcliRunner for FakeRunner {
        fn run<'a>(
            &'a self,
            args: &'a [&'a str],
            _timeout: Duration,
        ) -> Pin<Box<dyn Future<Output = Result<MmcliOutput, ModemError>> + Send + 'a>> {
            self.calls
                .lock()
                .unwrap()
                .push(args.iter().map(|s| s.to_string()).collect());
            Box::pin(async move {
                self.outputs.lock().unwrap().pop_front().unwrap_or_else(|| {
                    Err(ModemError::new("missing_fake_output", "no fake output"))
                })
            })
        }
    }

    fn out(stdout: &str) -> MmcliOutput {
        MmcliOutput {
            stdout: stdout.to_string(),
            stderr: String::new(),
            status_success: true,
        }
    }

    #[tokio::test]
    async fn status_uses_json_when_supported() {
        let runner = FakeRunner::new(vec![
            Ok(out("mmcli 1.22.0\n")),
            Ok(out(include_str!("../tests/fixtures/mmcli/healthy.json"))),
        ]);
        let service = ModemService::new_with_runner(runner.clone());
        let status = service
            .status("/org/freedesktop/ModemManager1/Modem/0")
            .await;

        assert_eq!(status.health.status, HealthLevel::Ok);
        assert!(status.tool.supports_json);
        assert_eq!(
            runner.calls(),
            vec![
                vec!["--version".to_string()],
                vec![
                    "--modem".to_string(),
                    "/org/freedesktop/ModemManager1/Modem/0".to_string(),
                    "--output-json".to_string()
                ]
            ]
        );
    }

    #[tokio::test]
    async fn status_reports_path_drift_candidate_when_configured_path_is_stale() {
        let runner = FakeRunner::new(vec![
            Ok(out("mmcli 1.22.0\n")),
            Ok(MmcliOutput {
                stdout: String::new(),
                stderr: "not found".to_string(),
                status_success: false,
            }),
            Ok(out(
                "/org/freedesktop/ModemManager1/Modem/7 [Quectel] EC25\n",
            )),
            Ok(out(
                "/org/freedesktop/ModemManager1/Modem/7 [Quectel] EC25\n",
            )),
        ]);
        let service = ModemService::new_with_runner(runner);
        let status = service
            .status("/org/freedesktop/ModemManager1/Modem/0")
            .await;

        assert_eq!(status.health.status, HealthLevel::Degraded);
        assert_eq!(
            status.diagnostics.path_drift_candidate.as_deref(),
            Some("/org/freedesktop/ModemManager1/Modem/7")
        );
        assert!(status
            .health
            .reasons
            .contains(&"modem_path_drift_candidate".to_string()));
    }

    #[tokio::test]
    async fn missing_mmcli_reports_unknown() {
        let runner = FakeRunner::new(vec![Err(ModemError::new(
            "mmcli_probe_failed",
            "not found",
        ))]);
        let service = ModemService::new_with_runner(runner);
        let status = service
            .status("/org/freedesktop/ModemManager1/Modem/0")
            .await;

        assert_eq!(status.tool.available, false);
        assert_eq!(status.health.status, HealthLevel::Unknown);
        assert!(status
            .health
            .reasons
            .contains(&"mmcli_probe_failed".to_string()));
    }

    #[tokio::test]
    async fn failed_capability_probe_is_not_cached() {
        let runner = FakeRunner::new(vec![
            Err(ModemError::new("mmcli_probe_failed", "not found")),
            Ok(out("mmcli 1.22.0\n")),
            Ok(out(include_str!("../tests/fixtures/mmcli/healthy.json"))),
        ]);
        let service = ModemService::new_with_runner(runner.clone());

        let first = service
            .status("/org/freedesktop/ModemManager1/Modem/0")
            .await;
        let second = service
            .status("/org/freedesktop/ModemManager1/Modem/0")
            .await;

        assert_eq!(first.health.status, HealthLevel::Unknown);
        assert_eq!(second.health.status, HealthLevel::Ok);
        assert_eq!(runner.calls().len(), 3);
    }

    #[tokio::test]
    async fn action_rejects_when_another_action_is_running() {
        let runner = FakeRunner::new(vec![Ok(out("mmcli 1.22.0\n"))]);
        let service = ModemService::new_with_runner(runner);
        let _guard = service.action_lock.try_lock().unwrap();
        let err = service
            .run_action(
                "/org/freedesktop/ModemManager1/Modem/0",
                "session-a",
                ModemAction::Enable,
            )
            .await
            .unwrap_err();

        assert_eq!(err.code(), "action_in_progress");
    }

    #[tokio::test]
    async fn reset_is_rate_limited_per_session() {
        let runner = FakeRunner::new(vec![]);
        let service = ModemService::new_with_runner(runner);
        service.can_reset("session-a").await.unwrap();
        let err = service.can_reset("session-a").await.unwrap_err();

        assert_eq!(err.code(), "reset_rate_limited");
        assert!(service.can_reset("session-b").await.is_ok());
    }

    #[tokio::test]
    async fn public_health_uses_short_cache() {
        let runner = FakeRunner::new(vec![
            Ok(out("mmcli 1.22.0\n")),
            Ok(out(include_str!("../tests/fixtures/mmcli/healthy.json"))),
        ]);
        let service = ModemService::new_with_runner(runner.clone());

        let first = service
            .public_health("/org/freedesktop/ModemManager1/Modem/0")
            .await;
        let second = service
            .public_health("/org/freedesktop/ModemManager1/Modem/0")
            .await;

        assert_eq!(first.status, HealthLevel::Ok);
        assert_eq!(second.status, HealthLevel::Ok);
        assert_eq!(runner.calls().len(), 2);
    }
}
