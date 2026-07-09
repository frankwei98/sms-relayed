use std::fmt;

use serde::Serialize;
use serde_json::Value;
use time::OffsetDateTime;

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
        Self { code, message: message.into() }
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
    status.health.reasons.push("text_fallback_limited".to_string());
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
        let usable = matches!(status.modem.state.as_deref(), Some("registered" | "connected"));
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
        assert!(status.health.reasons.contains(&"modem_disabled".to_string()));
    }

    #[test]
    fn classifies_missing_messaging_as_degraded() {
        let raw = include_str!("../tests/fixtures/mmcli/messaging-missing.json");
        let status = parse_modem_json(PATH, Some("0".to_string()), raw).unwrap();

        assert_eq!(status.health.status, HealthLevel::Degraded);
        assert!(!status.messaging.available);
        assert!(status.health.reasons.contains(&"messaging_unavailable".to_string()));
    }

    #[test]
    fn limited_text_fallback_marks_limited_reason() {
        let raw = include_str!("../tests/fixtures/mmcli/text-registered.txt");
        let status = parse_modem_text(PATH, Some("0".to_string()), raw).unwrap();

        assert_eq!(status.resolved.path.as_deref(), Some(PATH));
        assert_eq!(status.modem.state.as_deref(), Some("registered"));
        assert_eq!(status.modem.sim_state.as_deref(), Some("ready"));
        assert_eq!(status.modem.signal_quality, Some(78));
        assert!(status.health.reasons.contains(&"text_fallback_limited".to_string()));
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
        let err = map_list_path_to_id("/org/freedesktop/ModemManager1/Modem/99", raw)
            .unwrap_err();
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
        let err = map_list_path_to_id("/org/freedesktop/ModemManager1/Modem/0", raw)
            .unwrap_err();
        assert_eq!(err.code(), "modem_path_unresolved");
    }
}
