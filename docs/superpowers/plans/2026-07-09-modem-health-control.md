# Modem Health Control Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build public SmsRelayed health reporting plus an authenticated Modem page that reads and controls the configured ModemManager modem through `mmcli`.

**Architecture:** Add a focused Rust `modem` module that owns `mmcli` execution, capability detection, parsing, health classification, path resolution, action locking, reset rate limiting, and public health caching. Expose it through new Axum routes while keeping `/api/health` public and `/api/modem/*` protected. Add a standalone React `/modem` route with a reusable status panel and leave the existing Config editor mostly untouched.

**Tech Stack:** Rust 2021, Tokio, Axum 0.8, serde/serde_json, rusqlite, existing in-process session store, Vite, React 19, TanStack Router, Tailwind CSS, existing shadcn components.

## Global Constraints

- `mmcli` is an accepted runtime dependency for modem health and control.
- Core SMS send/receive continues to use ModemManager D-Bus through the existing zbus path.
- Public `/api/health` returns separate `service` and `modem` sub-statuses.
- `service.status` values are only `ok` and `error`; it checks the existing message database with a cheap query.
- `modem.status` values are `ok`, `degraded`, `error`, and `unknown`.
- Public health must not expose raw `mmcli` output, operator names, SIM identifiers, phone numbers, SMS bodies, object paths, modem ids, signal values, or network-layer details.
- Public modem health uses a 5 second cache.
- Authenticated `GET /api/modem/status` does not use the public health cache; every request runs a fresh status check.
- Modem actions operate only on configured `app.modem_path`.
- Do not auto-select first modem, only modem, or a modem by list order.
- Do not automatically persistently rewrite `app.modem_path`.
- Reset requires `{ "confirm": true }` at the API layer and a frontend confirmation dialog.
- `POST /api/modem/*` routes require JSON content type, valid session, same-origin Origin/Referer when present, one action at a time, and reset rate limiting.
- Concurrent modem actions are rejected with `409 conflict` and code `action_in_progress`.
- Reset is rate-limited per session to once per 60 seconds and does not block enable or disable.
- No factory reset, arbitrary AT command execution, bearer management, IP/DNS/gateway/default-route management, or cellular-as-WAN management.
- Do not log phone numbers, SMS bodies, SIM identifiers, full session tokens, or full command output.
- Rust commands run from the repository root.
- Frontend commands run from `frontend/` and use `pnpm`.
- After adding the TanStack Router `/modem` route, run `pnpm generate-routes`.

---

## File Structure

- Create `src/modem.rs`: typed public/internal modem models, `MmcliRunner` trait, real runner, parser/classifier, path resolution, action execution, action lock, reset rate limiter, public health cache, and unit tests.
- Create `src/api/modem.rs`: Axum routes for `GET /api/modem/status`, `POST /api/modem/enable`, `POST /api/modem/disable`, `POST /api/modem/reset`, content-type/origin hardening, reset confirmation validation, session token extraction for rate limiting/audit.
- Create `src/api/health.rs`: public `GET /api/health` route and DB service health response.
- Modify `src/api/mod.rs`: register `health::routes()` outside the protected router, register `modem::routes()` inside the protected router, add modem service to `ApiState`.
- Modify `src/runtime.rs`: initialize the shared modem service when API state is created.
- Modify `src/main.rs`: declare `mod modem;`.
- Modify `src/storage.rs`: add `MessageStore::health_check()`.
- Modify `Cargo.toml` only if implementation proves an existing dependency is insufficient. Start without adding dependencies.
- Create fixture files under `tests/fixtures/mmcli/`: JSON/text/list outputs used by modem unit tests.
- Create `frontend/src/lib/modem-api.ts`: TypeScript types and API helpers for modem status/actions.
- Create `frontend/src/components/modem/modem-status-panel.tsx`: standalone modem panel UI, polling, actions, reset dialog.
- Create `frontend/src/routes/modem.tsx`: TanStack route for `/modem`.
- Modify `frontend/src/routes/__root.tsx`: add `Modem` nav link.
- Modify generated `frontend/src/routeTree.gen.ts` only by running `pnpm generate-routes`.
- Modify `README.md`: document `mmcli` dependency, public `/api/health`, and deployment/access-control note.
- Modify `install.sh`: keep the existing `mmcli` warning but make the message explicitly mention Web modem status/control and health diagnostics.

---

### Task 1: Backend Modem Models, Fixtures, Parsing, and Health Classification

**Files:**
- Create: `src/modem.rs`
- Create: `tests/fixtures/mmcli/healthy.json`
- Create: `tests/fixtures/mmcli/sim-missing.json`
- Create: `tests/fixtures/mmcli/disabled.json`
- Create: `tests/fixtures/mmcli/messaging-missing.json`
- Create: `tests/fixtures/mmcli/text-registered.txt`
- Create: `tests/fixtures/mmcli/modem-list.txt`
- Modify: `src/main.rs`

**Interfaces:**
- Produces:
  - `pub enum HealthLevel { Ok, Degraded, Error, Unknown }`
  - `pub struct ToolInfo { pub available: bool, pub version_raw: Option<String>, pub supports_json: bool }`
  - `pub struct ResolvedModem { pub present: bool, pub id: Option<String>, pub path: Option<String> }`
  - `pub struct HealthSummary { pub status: HealthLevel, pub reasons: Vec<String> }`
  - `pub struct ModemDetails { pub enabled: Option<bool>, pub state: Option<String>, pub sim_state: Option<String>, pub operator_name: Option<String>, pub signal_quality: Option<u8>, pub access_technologies: Vec<String> }`
  - `pub struct MessagingDetails { pub available: bool, pub supported_storages: Vec<String>, pub default_storage: Option<String> }`
  - `pub struct Diagnostics { pub last_error: Option<String>, pub path_drift_candidate: Option<String> }`
  - `pub struct ModemStatus { pub checked_at: time::OffsetDateTime, pub tool: ToolInfo, pub configured_modem_path: String, pub resolved: ResolvedModem, pub health: HealthSummary, pub modem: ModemDetails, pub messaging: MessagingDetails, pub diagnostics: Diagnostics }`
  - `pub fn classify(status: &mut ModemStatus)`
  - `pub fn parse_modem_json(configured_path: &str, id: Option<String>, raw: &str) -> Result<ModemStatus, ModemError>`
  - `pub fn parse_modem_text(configured_path: &str, id: Option<String>, raw: &str) -> Result<ModemStatus, ModemError>`
  - `pub fn map_list_path_to_id(configured_path: &str, list_output: &str) -> Result<String, ModemError>`
- Consumes: no earlier task output.

- [ ] **Step 1: Add fixture files**

Create `tests/fixtures/mmcli/healthy.json`:

```json
{
  "modem": {
    "generic": {
      "dbus-path": "/org/freedesktop/ModemManager1/Modem/0",
      "state": "registered",
      "power-state": "on",
      "access-technologies": ["lte"],
      "signal-quality": { "value": 78, "recent": true }
    },
    "3gpp": {
      "operator-name": "CHN-UNICOM",
      "registration-state": "home"
    },
    "sim": {
      "state": "ready"
    },
    "messaging": {
      "supported-storages": ["sm", "me"],
      "default-storage": "sm"
    }
  }
}
```

Create `tests/fixtures/mmcli/sim-missing.json`:

```json
{
  "modem": {
    "generic": {
      "dbus-path": "/org/freedesktop/ModemManager1/Modem/0",
      "state": "registered",
      "power-state": "on",
      "access-technologies": ["lte"],
      "signal-quality": { "value": 51, "recent": true }
    },
    "3gpp": { "operator-name": "CHN-UNICOM" },
    "sim": { "state": "missing" },
    "messaging": { "supported-storages": ["sm"], "default-storage": "sm" }
  }
}
```

Create `tests/fixtures/mmcli/disabled.json`:

```json
{
  "modem": {
    "generic": {
      "dbus-path": "/org/freedesktop/ModemManager1/Modem/0",
      "state": "disabled",
      "power-state": "off",
      "access-technologies": [],
      "signal-quality": { "value": 0, "recent": false }
    },
    "sim": { "state": "ready" },
    "messaging": { "supported-storages": ["sm"], "default-storage": "sm" }
  }
}
```

Create `tests/fixtures/mmcli/messaging-missing.json`:

```json
{
  "modem": {
    "generic": {
      "dbus-path": "/org/freedesktop/ModemManager1/Modem/0",
      "state": "registered",
      "power-state": "on",
      "access-technologies": ["lte"],
      "signal-quality": { "value": 66, "recent": true }
    },
    "sim": { "state": "ready" },
    "messaging": { "supported-storages": [], "default-storage": null }
  }
}
```

Create `tests/fixtures/mmcli/text-registered.txt`:

```text
  --------------------------------
  General  |                 path: /org/freedesktop/ModemManager1/Modem/0
           |                state: registered
           |          power state: on
           |    access techologies: lte
  --------------------------------
  3GPP     |        operator name: CHN-UNICOM
  --------------------------------
  SIM      |                state: ready
  --------------------------------
  Messaging | supported storages: sm, me
            |    default storage: sm
  --------------------------------
  Signal   |      quality: 78% (recent)
```

Create `tests/fixtures/mmcli/modem-list.txt`:

```text
/org/freedesktop/ModemManager1/Modem/0 [Quectel] EC25
/org/freedesktop/ModemManager1/Modem/3 [Fibocom] FM350
```

- [ ] **Step 2: Write failing parser/classifier tests**

Add `src/modem.rs` with only test scaffolding first:

```rust
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
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run:

```bash
cargo test modem::tests -- --nocapture
```

Expected: FAIL because `HealthLevel`, `parse_modem_json`, `parse_modem_text`, and `map_list_path_to_id` are not implemented.

- [ ] **Step 4: Implement modem models and parsing**

Fill the top of `src/modem.rs` with the minimal implementation:

```rust
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

    for line in raw.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("|                 path:") {
            status.resolved.path = Some(value.trim().to_string());
            status.resolved.present = status.resolved.path.as_deref() == Some(configured_path);
        } else if let Some(value) = line.strip_prefix("|                state:") {
            status.modem.state = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("|        operator name:") {
            status.modem.operator_name = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("| supported storages:") {
            status.messaging.supported_storages = split_csv(value);
            status.messaging.available = !status.messaging.supported_storages.is_empty();
        } else if let Some(value) = line.strip_prefix("|    default storage:") {
            status.messaging.default_storage = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("|      quality:") {
            status.modem.signal_quality = value
                .trim()
                .split('%')
                .next()
                .and_then(|n| n.trim().parse::<u8>().ok());
        } else if line.contains("|                state: ready") {
            status.modem.sim_state = Some("ready".to_string());
        } else if let Some(value) = line.strip_prefix("|    access techologies:") {
            status.modem.access_technologies = split_csv(value);
        }
    }
    if status.modem.sim_state.is_none() && raw.contains("SIM") && raw.contains("state: ready") {
        status.modem.sim_state = Some("ready".to_string());
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
        if !line.starts_with(configured_path) {
            continue;
        }
        let Some(id) = line
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
```

Modify `src/main.rs` module declarations:

```rust
mod api;
mod assets;
mod cli;
mod config;
mod dbus;
mod events;
mod forward;
mod message;
mod modem;
mod runtime;
mod smscode;
mod storage;
mod util;
mod web;
mod wizard;
```

- [ ] **Step 5: Run parser/classifier tests**

Run:

```bash
cargo test modem::tests -- --nocapture
```

Expected: PASS for all modem parser/classifier tests.

- [ ] **Step 6: Commit**

```bash
git add src/main.rs src/modem.rs tests/fixtures/mmcli
git commit -m "feat: add modem status parser"
```

---

### Task 2: Mmcli Runner, Capability Probe, Modem Service, Resolution, and Actions

**Files:**
- Modify: `src/modem.rs`

**Interfaces:**
- Consumes: `ModemStatus`, `HealthLevel`, `ModemError`, `parse_modem_json`, `parse_modem_text`, `map_list_path_to_id` from Task 1.
- Produces:
  - `pub enum ModemAction { Enable, Disable, Reset }`
  - `pub struct ActionResponse { pub accepted: bool, pub action: &'static str }`
  - `pub struct PublicModemHealth { pub status: HealthLevel, pub reason: Option<String>, pub checked_at: OffsetDateTime }`
  - `pub trait MmcliRunner`
  - `pub struct RealMmcliRunner`
  - `pub struct ModemService`
  - `impl ModemService { pub fn new() -> Self; pub async fn status(&self, configured_path: &str) -> ModemStatus; pub async fn public_health(&self, configured_path: &str) -> PublicModemHealth; pub async fn run_action(&self, configured_path: &str, session_token: &str, action: ModemAction) -> Result<ActionResponse, ModemError>; pub async fn can_reset(&self, session_token: &str) -> Result<(), ModemError>; }`

- [ ] **Step 1: Write failing service tests**

Append tests to `src/modem.rs`:

```rust
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
                self.outputs
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or_else(|| Err(ModemError::new("missing_fake_output", "no fake output")))
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
    async fn missing_mmcli_reports_unknown() {
        let runner = FakeRunner::new(vec![Err(ModemError::new("mmcli_probe_failed", "not found"))]);
        let service = ModemService::new_with_runner(runner);
        let status = service
            .status("/org/freedesktop/ModemManager1/Modem/0")
            .await;

        assert_eq!(status.tool.available, false);
        assert_eq!(status.health.status, HealthLevel::Unknown);
        assert!(status.health.reasons.contains(&"mmcli_probe_failed".to_string()));
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test modem::service_tests -- --nocapture
```

Expected: FAIL because runner/service/action types are missing.

- [ ] **Step 3: Implement runner and service**

Extend `src/modem.rs`:

```rust
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::process::Command;

const MMCLI_TIMEOUT: Duration = Duration::from_secs(5);
const HEALTH_CACHE_TTL: Duration = Duration::from_secs(5);
const RESET_RATE_LIMIT: Duration = Duration::from_secs(60);

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
        timeout: Duration,
    ) -> Pin<Box<dyn Future<Output = Result<MmcliOutput, ModemError>> + Send + 'a>>;
}

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
                stderr: String::from_utf8_lossy(&output.stderr).chars().take(300).collect(),
                status_success: output.status.success(),
            })
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ModemAction {
    Enable,
    Disable,
    Reset,
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
                        status.tool = tool;
                        status
                    })
                    .unwrap_or_else(|err| error_status(configured_path, tool, err.code(), err.to_string()))
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
                    Err(err) => error_status(configured_path, tool, err.code(), err.to_string()),
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
            return Err(ModemError::new("action_in_progress", "another modem action is running"));
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
        log::info!("modem action={} session={} result=accepted", action.name(), short_hash(session_token));
        Ok(ActionResponse { accepted: true, action: action.name() })
    }

    pub async fn can_reset(&self, session_token: &str) -> Result<(), ModemError> {
        let key = short_hash(session_token);
        let mut guard = self.reset_limits.lock().unwrap();
        if let Some(last) = guard.get(&key) {
            if last.elapsed() < RESET_RATE_LIMIT {
                return Err(ModemError::new("reset_rate_limited", "reset is rate limited"));
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
            _ => ToolInfo {
                available: false,
                version_raw: None,
                supports_json: false,
            },
        };
        *self.capabilities.lock().unwrap() = Some(tool.clone());
        tool
    }

    async fn resolve_target(&self, configured_path: &str, _tool: &ToolInfo) -> Result<String, ModemError> {
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
```

- [ ] **Step 4: Run service tests**

Run:

```bash
cargo test modem::service_tests -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Run all modem tests**

Run:

```bash
cargo test modem:: -- --nocapture
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/modem.rs
git commit -m "feat: add mmcli modem service"
```

---

### Task 3: Public Health API and Protected Modem API Routes

**Files:**
- Create: `src/api/health.rs`
- Create: `src/api/modem.rs`
- Modify: `src/api/mod.rs`
- Modify: `src/runtime.rs`
- Modify: `src/storage.rs`
- Modify: `Cargo.toml`

**Interfaces:**
- Consumes: `crate::modem::{ActionResponse, HealthLevel, ModemAction, ModemService, PublicModemHealth}` from Task 2.
- Produces:
  - Public route `GET /api/health`
  - Protected routes `GET /api/modem/status`, `POST /api/modem/enable`, `POST /api/modem/disable`, `POST /api/modem/reset`
  - `MessageStore::health_check(&self) -> anyhow::Result<()>`
  - `ApiState { modem: ModemService, ... }`

- [ ] **Step 1: Add route test dependency and write failing route tests**

Add this to `Cargo.toml` because the route tests need `tower::ServiceExt::oneshot`:

```toml
[dev-dependencies]
tower = "0.5"
```

Then add tests to `src/api/mod.rs` test module. Use the existing imports and add `axum::body::Body`, `axum::http::{Method, Request}`, and `tower::ServiceExt`.

```rust
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use tower::ServiceExt;

fn test_state() -> ApiState {
    let mut cfg = AppConfig::default();
    cfg.api.enabled = true;
    cfg.api.password = "secret".to_string();
    ApiState {
        config: std::sync::Arc::new(cfg),
        config_path: std::path::PathBuf::from("/tmp/sms-relayed-test.toml"),
        store: crate::storage::MessageStore::open_in_memory().unwrap(),
        events: crate::events::EventBus::new(),
        started_at: std::time::Instant::now(),
        sessions: SessionStore::default(),
        modem: crate::modem::ModemService::new(),
    }
}

#[tokio::test]
async fn health_route_is_public() {
    let app = router(test_state());
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn modem_status_route_requires_session() {
    let app = router(test_state());
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/modem/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn reset_rejects_missing_confirmation() {
    let state = test_state();
    let token = state.sessions.create_session();
    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/modem/reset")
                .header("cookie", format!("sms-relayed-session={token}"))
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test api::tests -- --nocapture
```

Expected: FAIL because routes and `ApiState.modem` do not exist.

- [ ] **Step 3: Add `MessageStore::health_check`**

Append this method inside `impl MessageStore` in `src/storage.rs`:

```rust
pub fn health_check(&self) -> Result<()> {
    let conn = self.conn.lock().unwrap();
    conn.query_row("SELECT 1", [], |_| Ok(()))?;
    Ok(())
}
```

- [ ] **Step 4: Add public health route**

Create `src/api/health.rs`:

```rust
use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use time::OffsetDateTime;

use crate::modem::PublicModemHealth;

use super::ApiState;

#[derive(Serialize)]
struct ServiceHealth {
    status: &'static str,
    #[serde(with = "time::serde::rfc3339")]
    checked_at: OffsetDateTime,
}

#[derive(Serialize)]
struct HealthResponse {
    service: ServiceHealth,
    modem: PublicModemHealth,
}

pub fn routes() -> Router<ApiState> {
    Router::new().route("/api/health", get(health))
}

async fn health(State(state): State<ApiState>) -> Json<HealthResponse> {
    let checked_at = OffsetDateTime::now_utc();
    let service = ServiceHealth {
        status: if state.store.health_check().is_ok() {
            "ok"
        } else {
            "error"
        },
        checked_at,
    };
    let modem = state
        .modem
        .public_health(&state.config.app.modem_path)
        .await;
    Json(HealthResponse { service, modem })
}
```

- [ ] **Step 5: Add protected modem route**

Create `src/api/modem.rs`:

```rust
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::modem::{ActionResponse, ModemAction, ModemStatus};

use super::auth;
use super::{ApiError, ApiResult, ApiState};

#[derive(Deserialize)]
struct ResetRequest {
    confirm: Option<bool>,
}

pub fn routes() -> Router<ApiState> {
    Router::new()
        .route("/api/modem/status", get(status))
        .route("/api/modem/enable", post(enable))
        .route("/api/modem/disable", post(disable))
        .route("/api/modem/reset", post(reset))
}

async fn status(State(state): State<ApiState>) -> ApiResult<Json<ModemStatus>> {
    Ok(Json(state.modem.status(&state.config.app.modem_path).await))
}

async fn enable(State(state): State<ApiState>, headers: HeaderMap) -> ApiResult<Json<ActionResponse>> {
    harden_action(&headers)?;
    run_action(state, headers, ModemAction::Enable).await
}

async fn disable(State(state): State<ApiState>, headers: HeaderMap) -> ApiResult<Json<ActionResponse>> {
    harden_action(&headers)?;
    run_action(state, headers, ModemAction::Disable).await
}

async fn reset(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<ResetRequest>,
) -> ApiResult<Json<ActionResponse>> {
    harden_action(&headers)?;
    if req.confirm != Some(true) {
        return Err(ApiError::bad_request("reset requires confirm=true"));
    }
    run_action(state, headers, ModemAction::Reset).await
}

async fn run_action(
    state: ApiState,
    headers: HeaderMap,
    action: ModemAction,
) -> ApiResult<Json<ActionResponse>> {
    let token = auth::session_token(&headers);
    state
        .modem
        .run_action(&state.config.app.modem_path, &token, action)
        .await
        .map(Json)
        .map_err(map_modem_error)
}

fn harden_action(headers: &HeaderMap) -> ApiResult<()> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !content_type.to_ascii_lowercase().starts_with("application/json") {
        return Err(ApiError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
            "modem actions require application/json",
        ));
    }
    if !same_origin(headers) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "cross_origin_rejected",
            "cross-origin modem action rejected",
        ));
    }
    Ok(())
}

fn same_origin(headers: &HeaderMap) -> bool {
    let Some(expected_host) = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(header::HOST))
        .and_then(|v| v.to_str().ok())
        .map(strip_port)
    else {
        return true;
    };

    for name in [header::ORIGIN, header::REFERER] {
        let Some(value) = headers.get(name).and_then(|v| v.to_str().ok()) else {
            continue;
        };
        let Some(host) = origin_host(value) else {
            return false;
        };
        if host != expected_host {
            return false;
        }
    }
    true
}

fn origin_host(value: &str) -> Option<String> {
    let without_scheme = value.split("://").nth(1).unwrap_or(value);
    let host = without_scheme.split('/').next()?;
    Some(strip_port(host))
}

fn strip_port(host: &str) -> String {
    host.split(':').next().unwrap_or(host).to_ascii_lowercase()
}

fn map_modem_error(err: crate::modem::ModemError) -> ApiError {
    match err.code() {
        "action_in_progress" => ApiError::new(StatusCode::CONFLICT, "action_in_progress", err.to_string()),
        "reset_rate_limited" => ApiError::new(StatusCode::TOO_MANY_REQUESTS, "reset_rate_limited", err.to_string()),
        "modem_path_unresolved" => ApiError::new(StatusCode::CONFLICT, "modem_path_unresolved", err.to_string()),
        _ => ApiError::internal(err.to_string()),
    }
}
```

- [ ] **Step 6: Wire routes and state**

Modify `src/api/mod.rs`:

```rust
pub mod auth;
pub mod config;
pub mod health;
pub mod messages;
pub mod modem;
pub mod service;
```

Add to `ApiState`:

```rust
pub modem: crate::modem::ModemService,
```

Modify `router`:

```rust
let protected = Router::new()
    .merge(messages::routes())
    .merge(config::routes())
    .merge(service::routes())
    .merge(modem::routes())
    .layer(middleware::from_fn(
        move |req: axum::extract::Request, next: middleware::Next| {
            let sessions = sessions.clone();
            async move {
                let token = auth::session_token(req.headers());
                if !sessions.is_valid(&token) {
                    return ApiError::unauthorized("authentication required").into_response();
                }
                next.run(req).await
            }
        },
    ));

Router::new()
    .merge(health::routes())
    .merge(auth_routes)
    .merge(protected)
    .with_state(state)
    .fallback(crate::assets::serve)
```

Modify `src/runtime.rs` API state construction:

```rust
let state = api::ApiState {
    config: Arc::new(config.clone()),
    config_path: config_path.to_path_buf(),
    store: store.clone(),
    events: events.clone(),
    started_at,
    sessions: api::auth::SessionStore::default(),
    modem: crate::modem::ModemService::new(),
};
```

- [ ] **Step 7: Run route tests**

Run:

```bash
cargo test api::tests -- --nocapture
```

Expected: PASS.

- [ ] **Step 8: Run backend tests**

Run:

```bash
cargo test
```

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add src/api/mod.rs src/api/health.rs src/api/modem.rs src/runtime.rs src/storage.rs Cargo.toml Cargo.lock
git commit -m "feat: expose modem health api"
```

---

### Task 4: Frontend Modem API Helper, Route, Panel, and Navigation

**Files:**
- Create: `frontend/src/lib/modem-api.ts`
- Create: `frontend/src/components/modem/modem-status-panel.tsx`
- Create: `frontend/src/routes/modem.tsx`
- Modify: `frontend/src/routes/__root.tsx`
- Modify generated: `frontend/src/routeTree.gen.ts` by running `pnpm generate-routes`

**Interfaces:**
- Consumes backend routes from Task 3:
  - `GET /api/modem/status`
  - `POST /api/modem/enable`
  - `POST /api/modem/disable`
  - `POST /api/modem/reset` with `{ confirm: true }`
- Produces:
  - `fetchModemStatus(): Promise<ModemStatus>`
  - `runModemAction(action: "enable" | "disable" | "reset"): Promise<ActionResponse>`
  - React route `/modem`
  - `ModemStatusPanel`

- [ ] **Step 1: Add modem API helper**

Create `frontend/src/lib/modem-api.ts`:

```ts
import { apiFetch } from "#/lib/api";

export type HealthLevel = "ok" | "degraded" | "error" | "unknown";

export type ModemStatus = {
	checked_at: string;
	tool: {
		available: boolean;
		version_raw: string | null;
		supports_json: boolean;
	};
	configured_modem_path: string;
	resolved: {
		present: boolean;
		id: string | null;
		path: string | null;
	};
	health: {
		status: HealthLevel;
		reasons: string[];
	};
	modem: {
		enabled: boolean | null;
		state: string | null;
		sim_state: string | null;
		operator_name: string | null;
		signal_quality: number | null;
		access_technologies: string[];
	};
	messaging: {
		available: boolean;
		supported_storages: string[];
		default_storage: string | null;
	};
	diagnostics: {
		last_error: string | null;
		path_drift_candidate: string | null;
	};
};

export type ModemAction = "enable" | "disable" | "reset";

export type ActionResponse = {
	accepted: boolean;
	action: ModemAction;
};

export function fetchModemStatus() {
	return apiFetch<ModemStatus>("/api/modem/status");
}

export function runModemAction(action: ModemAction) {
	return apiFetch<ActionResponse>(`/api/modem/${action}`, {
		method: "POST",
		body: JSON.stringify(action === "reset" ? { confirm: true } : {}),
	});
}
```

- [ ] **Step 2: Add route**

Create `frontend/src/routes/modem.tsx`:

```tsx
import { createFileRoute } from "@tanstack/react-router";
import { ModemStatusPanel } from "#/components/modem/modem-status-panel";

export const Route = createFileRoute("/modem")({
	component: ModemPage,
});

function ModemPage() {
	return <ModemStatusPanel />;
}
```

- [ ] **Step 3: Add modem panel**

Create `frontend/src/components/modem/modem-status-panel.tsx`:

```tsx
import { RefreshCw, RotateCcw, Power, PowerOff } from "lucide-react";
import { useEffect, useState } from "react";
import { Button } from "#/components/ui/button";
import {
	Dialog,
	DialogContent,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	DialogTrigger,
} from "#/components/ui/dialog";
import {
	type ModemAction,
	type ModemStatus,
	fetchModemStatus,
	runModemAction,
} from "#/lib/modem-api";

const POLL_INTERVAL_MS = 2000;
const POLL_LIMIT = 15;

export function ModemStatusPanel() {
	const [status, setStatus] = useState<ModemStatus | null>(null);
	const [loading, setLoading] = useState(true);
	const [busy, setBusy] = useState<ModemAction | null>(null);
	const [error, setError] = useState("");
	const [resetOpen, setResetOpen] = useState(false);

	async function refresh() {
		setError("");
		try {
			setStatus(await fetchModemStatus());
		} catch (e) {
			setError((e as Error).message);
		} finally {
			setLoading(false);
		}
	}

	useEffect(() => {
		void refresh();
	}, []);

	async function run(action: ModemAction) {
		setBusy(action);
		setError("");
		try {
			await runModemAction(action);
			if (action === "reset") {
				setResetOpen(false);
			}
			await pollStatus();
		} catch (e) {
			setError((e as Error).message);
		} finally {
			setBusy(null);
		}
	}

	async function pollStatus() {
		for (let i = 0; i < POLL_LIMIT; i++) {
			await new Promise((resolve) => setTimeout(resolve, POLL_INTERVAL_MS));
			const next = await fetchModemStatus();
			setStatus(next);
			if (next.resolved.present || next.diagnostics.path_drift_candidate) {
				return;
			}
		}
	}

	if (loading) return <p>Loading modem status...</p>;

	return (
		<div className="mx-auto max-w-5xl space-y-6">
			<div className="flex flex-wrap items-center justify-between gap-3">
				<div>
					<h2 className="text-lg font-semibold">Modem</h2>
					<p className="text-sm text-muted-foreground">
						{status ? `Last checked ${formatDate(status.checked_at)}` : "Status unavailable"}
					</p>
				</div>
				<div className="flex items-center gap-2">
					{status && <StatusBadge value={status.health.status} />}
					<Button variant="outline" onClick={refresh} disabled={!!busy}>
						<RefreshCw className="size-4" />
						Refresh
					</Button>
				</div>
			</div>

			{error && <div className="rounded border border-destructive/30 bg-destructive/10 p-3 text-sm text-destructive">{error}</div>}

			{status && (
				<>
					<section className="grid gap-3 md:grid-cols-2">
						<Field label="Configured path" value={status.configured_modem_path} />
						<Field label="Resolved modem" value={status.resolved.path ?? "not found"} />
						<Field label="Enabled" value={formatBool(status.modem.enabled)} />
						<Field label="State" value={status.modem.state ?? "unknown"} />
						<Field label="SIM" value={status.modem.sim_state ?? "unknown"} />
						<Field label="Operator" value={status.modem.operator_name ?? "unknown"} />
						<Field label="Signal" value={status.modem.signal_quality == null ? "unknown" : `${status.modem.signal_quality}%`} />
						<Field label="Access" value={status.modem.access_technologies.join(", ") || "unknown"} />
						<Field label="Messaging" value={status.messaging.available ? "available" : "unavailable"} />
						<Field label="mmcli" value={status.tool.available ? status.tool.version_raw ?? "available" : "missing"} />
					</section>

					{(status.health.reasons.length > 0 || status.diagnostics.last_error || status.diagnostics.path_drift_candidate) && (
						<section className="rounded border bg-muted/30 p-4 text-sm">
							<h3 className="mb-2 font-medium">Diagnostics</h3>
							{status.health.reasons.length > 0 && <p>Reasons: {status.health.reasons.join(", ")}</p>}
							{status.diagnostics.path_drift_candidate && (
								<p>Possible new modem path: {status.diagnostics.path_drift_candidate}</p>
							)}
							{status.diagnostics.last_error && <p>Error: {status.diagnostics.last_error}</p>}
						</section>
					)}

					<section className="flex flex-wrap gap-2">
						<Button onClick={() => run("enable")} disabled={busy !== null || status.modem.enabled === true}>
							<Power className="size-4" />
							Enable
						</Button>
						<Button variant="outline" onClick={() => run("disable")} disabled={busy !== null || status.modem.enabled === false}>
							<PowerOff className="size-4" />
							Disable
						</Button>
					</section>

					<section className="space-y-2 border-t pt-4">
						<h3 className="font-medium text-destructive">Danger zone</h3>
						<Dialog open={resetOpen} onOpenChange={setResetOpen}>
							<DialogTrigger asChild>
								<Button variant="destructive" disabled={busy !== null}>
									<RotateCcw className="size-4" />
									Reset modem
								</Button>
							</DialogTrigger>
							<DialogContent>
								<DialogHeader>
									<DialogTitle>Reset modem?</DialogTitle>
								</DialogHeader>
								<p className="text-sm text-muted-foreground">
									This can disconnect cellular service and cause the modem to disappear while it re-enumerates.
								</p>
								<DialogFooter>
									<Button variant="outline" onClick={() => setResetOpen(false)}>
										Cancel
									</Button>
									<Button variant="destructive" onClick={() => run("reset")} disabled={busy !== null}>
										Reset
									</Button>
								</DialogFooter>
							</DialogContent>
						</Dialog>
					</section>
				</>
			)}
		</div>
	);
}

function StatusBadge({ value }: { value: ModemStatus["health"]["status"] }) {
	const className =
		value === "ok"
			? "bg-emerald-100 text-emerald-800"
			: value === "degraded"
				? "bg-amber-100 text-amber-800"
				: value === "error"
					? "bg-red-100 text-red-800"
					: "bg-slate-100 text-slate-700";
	return <span className={`rounded px-2 py-1 text-xs font-medium ${className}`}>{value.toUpperCase()}</span>;
}

function Field({ label, value }: { label: string; value: string }) {
	return (
		<div className="rounded border p-3">
			<div className="text-xs text-muted-foreground">{label}</div>
			<div className="mt-1 break-all text-sm font-medium">{value}</div>
		</div>
	);
}

function formatBool(value: boolean | null) {
	if (value === true) return "yes";
	if (value === false) return "no";
	return "unknown";
}

function formatDate(value: string) {
	return new Date(value).toLocaleString();
}
```

- [ ] **Step 4: Add navigation link**

Modify `frontend/src/routes/__root.tsx` nav:

```tsx
<nav className="flex gap-4">
	<Link to="/" className="text-sm hover:underline">
		SMS
	</Link>
	<Link to="/modem" className="text-sm hover:underline">
		Modem
	</Link>
	<Link to="/config" className="text-sm hover:underline">
		Config
	</Link>
</nav>
```

- [ ] **Step 5: Generate routes**

Run:

```bash
cd frontend && pnpm generate-routes
```

Expected: `frontend/src/routeTree.gen.ts` includes `/modem`.

- [ ] **Step 6: Run frontend checks**

Run:

```bash
cd frontend && pnpm check
```

Expected: PASS. If Biome reports formatting issues, run `cd frontend && pnpm format --write` and then rerun `cd frontend && pnpm check`.

- [ ] **Step 7: Commit**

```bash
git add frontend/src/lib/modem-api.ts frontend/src/components/modem/modem-status-panel.tsx frontend/src/routes/modem.tsx frontend/src/routes/__root.tsx frontend/src/routeTree.gen.ts
git commit -m "feat: add modem status page"
```

---

### Task 5: Documentation, Installer Messaging, and Full Verification

**Files:**
- Modify: `README.md`
- Modify: `install.sh`

**Interfaces:**
- Consumes backend and frontend behavior from Tasks 1-4.
- Produces user-facing docs for `mmcli`, `/api/health`, access-control expectations, and installer warning language.

- [ ] **Step 1: Update README dependencies and health docs**

Add or adjust README prose near the dependency/setup section:

```markdown
### Modem health and control dependency

The Web Modem page and `/api/health` modem sub-status use `mmcli` from ModemManager.
Core SMS send/receive still uses ModemManager over system D-Bus.

If `mmcli` is missing, SmsRelayed can still start, but modem health/control reports `unknown`.
Install or enable ModemManager's `mmcli` package on the target device when you want Web modem diagnostics or enable/disable/reset controls.
```

Add a public health note near API/Web documentation:

```markdown
`GET /api/health` is public and returns separate `service` and `modem` status objects.
Expose it only on trusted networks or behind access control if the service is reachable beyond the device.
The public response intentionally omits modem object paths, operator names, signal values, SIM identifiers, phone numbers, SMS bodies, and raw command output.
```

- [ ] **Step 2: Update installer warning**

Modify `install.sh` in `warn_environment()`:

```sh
warn_environment() {
  have mmcli || warn "mmcli not found; SMS relay can still use ModemManager D-Bus, but Web modem status/control and /api/health modem diagnostics will report unknown"
}
```

- [ ] **Step 3: Run backend verification**

Run:

```bash
cargo fmt --check
cargo test
```

Expected: PASS.

- [ ] **Step 4: Run frontend verification**

Run:

```bash
cd frontend && pnpm generate-routes && pnpm check && pnpm build
```

Expected: PASS. If `pnpm build` fails due unrelated environment memory limits, rerun `pnpm check` and report the exact build failure separately.

- [ ] **Step 5: Inspect final diff for secrets and scope**

Run:

```bash
git diff --stat HEAD
rg -n "phone_number|body|SIM|raw command|factory-reset|AT command" src frontend/src README.md install.sh
```

Expected: no new logging of SMS bodies, phone numbers, SIM identifiers, full command output, factory reset, or arbitrary AT command paths.

- [ ] **Step 6: Commit**

```bash
git add README.md install.sh
git commit -m "docs: document modem health dependency"
```

---

## Final Verification Before Handoff

- [ ] Run backend:

```bash
cargo fmt --check
cargo test
```

- [ ] Run frontend:

```bash
cd frontend && pnpm generate-routes && pnpm check && pnpm build
```

- [ ] Confirm route generation is committed:

```bash
git status --short
```

Expected: no unstaged generated route changes.

- [ ] Report live modem verification status:

```bash
command -v mmcli && mmcli -L || true
```

Expected: if `mmcli` or modem hardware is unavailable locally, say live modem verification was not performed and point to parser/runner tests as the no-hardware coverage.
