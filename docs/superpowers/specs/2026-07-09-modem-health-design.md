# Modem Health and Control Design

Date: 2026-07-09

## Goal

Add a modem health and control surface for SmsRelayed so the operator can quickly tell whether the configured ModemManager modem is usable for SMS relay and perform basic modem actions from the web UI.

The feature has two layers:

- A public, minimal health endpoint that can be used by uptime tooling and modem-aware monitors.
- An authenticated modem status and control page for the web UI.

This feature focuses on SMS relay health. It does not manage bearer connections, default routes, IP addresses, gateways, DNS, or cellular-as-WAN behavior.

## Runtime Dependency

`mmcli` is an accepted runtime dependency for the modem health and control feature.

The backend should call `mmcli` from Rust and map command results into SmsRelayed's own API model. The API should not require the frontend to understand raw `mmcli` output. Full raw command output should not be logged. Error summaries may be returned to authenticated users after truncation or normalization.

The project already uses zbus directly for SMS send/receive. This feature uses `mmcli` instead of adding a second direct ModemManager property client because `mmcli` matches the user's operational debugging workflow and hides some D-Bus property layout differences behind a stable tool interface. The tradeoff is that the implementation must explicitly handle `mmcli` version differences, command timeouts, and missing-tool diagnostics.

If `mmcli` is missing, the modem sub-status should report `unknown` instead of panicking. SmsRelayed process health and modem health must stay separate so a missing diagnostic tool does not falsely imply the web process is down.

## API Design

### Public Health

`GET /api/health`

This route is public and does not require login. It returns compact service health plus a minimal modem health sub-status:

```json
{
  "service": {
    "status": "ok",
    "checked_at": "2026-07-09T12:00:00Z"
  },
  "modem": {
    "status": "degraded",
    "reason": "sim_not_ready",
    "checked_at": "2026-07-09T12:00:00Z"
  }
}
```

`service.status` reflects SmsRelayed itself. Because reaching this handler already proves Axum can serve the request, the active service check is the message database. Values are:

- `ok`: the health handler can open or query the configured message database.
- `error`: the database check fails.

Do not add `degraded` or `unknown` to `service.status` until there is a concrete non-database service check that needs those states. Keep the database check cheap, such as opening the configured store and performing a minimal query.

`modem.status` values:

- `ok`: configured modem is present and the core SMS-relay prerequisites are healthy.
- `degraded`: the configured modem is present but one or more SMS-relay prerequisites are not healthy.
- `error`: ModemManager is unreachable, the configured modem cannot be resolved, or modem status cannot be checked after the available parser path fails.
- `unknown`: `mmcli` is missing, the feature cannot check modem health in this environment, or modem checking was intentionally skipped.

The public endpoint must not expose raw `mmcli` output, operator names, SIM identifiers, phone numbers, SMS bodies, object paths, modem ids, signal values, or network-layer details. Public `reason` must be an enum-like code such as `mmcli_missing`, `modem_path_unresolved`, `sim_not_ready`, or `messaging_unavailable`, not free-form command output.

Implement a short cache for the modem portion of `/api/health` so frequent external health checks do not spawn `mmcli` on every request. The default cache TTL should be 5 seconds. If a future configuration knob is added, it should live under the API configuration area. Authenticated `GET /api/modem/status` should not use the public health cache; each request should run a fresh status check so the `Refresh` button is meaningful.

### Authenticated Modem Status

`GET /api/modem/status`

This route requires the existing web session. It returns a stable structured model for the Modem page:

```json
{
  "checked_at": "2026-07-09T12:00:00Z",
  "tool": {
    "available": true,
    "version_raw": "mmcli 1.22.0",
    "supports_json": true
  },
  "configured_modem_path": "/org/freedesktop/ModemManager1/Modem/0",
  "resolved": {
    "present": true,
    "id": "0",
    "path": "/org/freedesktop/ModemManager1/Modem/0"
  },
  "health": {
    "status": "ok",
    "reasons": []
  },
  "modem": {
    "enabled": true,
    "state": "registered",
    "sim_state": "ready",
    "operator_name": "CHN-UNICOM",
    "signal_quality": 78,
    "access_technologies": ["lte"]
  },
  "messaging": {
    "available": true,
    "supported_storages": ["sm", "me"],
    "default_storage": "sm"
  },
  "diagnostics": {
    "last_error": null,
    "path_drift_candidate": null
  }
}
```

Fields that cannot be read should be returned as `null` or omitted where appropriate, with a reason added to `health.reasons`. Partial status should not become HTTP 500 when the configured modem can still be identified.

### Health Classification

Use an explicit classification table instead of ad hoc UI logic:

| Condition | Modem status | Reason |
| --- | --- | --- |
| `mmcli` missing | `unknown` | `mmcli_missing` |
| ModemManager unreachable | `error` | `modemmanager_unreachable` |
| `app.modem_path` cannot be resolved | `error` | `modem_path_unresolved` |
| modem present but disabled | `degraded` | `modem_disabled` |
| SIM state is known and not ready | `degraded` | `sim_not_ready` |
| messaging capability/interface unavailable | `degraded` | `messaging_unavailable` |
| modem state indicates failed, locked, or unavailable | `degraded` | `modem_state_unhealthy` |
| JSON unsupported and text fallback is active | status from available fields | include `text_fallback_limited` |
| configured modem present, enabled, SIM ready, modem state usable, messaging available | `ok` | none |

"Usable modem state" should be implemented conservatively from parsed `mmcli` values. `registered` and `connected` are usable. `searching`, `enabled`, or other transitional states are `degraded` unless the parsed output clearly proves SMS messaging is available and the implementation deliberately treats that state as usable with a test fixture.

### Authenticated Actions

All action routes require login and operate only on the configured `app.modem_path`.

- `POST /api/modem/enable`
- `POST /api/modem/disable`
- `POST /api/modem/reset`

The backend must not auto-select a different modem if the configured modem path is missing or stale. It should return a clear error so the user can fix the configuration intentionally.

`POST /api/modem/reset` must require a confirmation payload:

```json
{ "confirm": true }
```

Missing or false confirmation returns `400 bad_request`. The API-level confirmation is permanent and applies to browser and scripted callers.

Enable, disable, and reset should return accepted semantics after the command is submitted successfully. The frontend should then poll status rather than assuming the modem state changed synchronously.

Reset means the ModemManager modem reset operation, equivalent to `mmcli --reset`: the modem may disconnect, disappear temporarily, and re-enumerate. This is not a factory reset. The implementation must not expose factory reset, arbitrary AT commands, route changes, bearer management, or network interface control.

### Action Hardening

All `POST /api/modem/*` routes must enforce API-layer hardening:

- Require `Content-Type: application/json` for modem action routes. This requirement is scoped to `/api/modem/*` and should not be introduced as a global middleware for existing JSON-less action routes such as service restart.
- Require a valid existing web session.
- Check `Origin` or `Referer` when present and reject cross-origin requests whose origin host does not match the incoming `Host` or trusted forwarded host. Do not add a new configured API origin field for this feature.
- Use a per-process action lock so enable, disable, and reset cannot run concurrently. If another action is already running, reject the new request with `409 conflict` and code `action_in_progress`; do not queue modem actions.
- Rate-limit reset per session to once per 60 seconds. This reset rate limit must not block enable or disable actions.

These checks do not replace the frontend confirmation dialog. They prevent accidental or cross-site destructive requests from bypassing UI intent.

## Modem Resolution

The configured `app.modem_path` is a ModemManager D-Bus object path such as `/org/freedesktop/ModemManager1/Modem/0`. The implementation must treat that configured path as the target identity.

Resolution rules:

1. Prefer invoking `mmcli --modem <configured_object_path>` directly.
2. If the installed `mmcli` does not accept object paths, call `mmcli -L` and map the configured object path to the exact listed modem index.
3. If the mapping is missing, ambiguous, or only inferable by list order, return `error` with reason `modem_path_unresolved`.
4. Never fall back to "first modem", "only modem", or index order for an action.
5. Store the resolved id/path only in the response model and short-lived execution context, not in the persistent config.

Reset recovery is separate from action targeting. After reset, if the configured path disappears, the backend may list modems to help diagnose path drift:

- If the configured path comes back within the polling window, report normal post-reset status.
- If the configured path does not come back but exactly one other modem path appears, return `degraded` with reason `modem_path_drift_candidate` and include that candidate path in authenticated diagnostics.
- If no candidate or multiple candidates exist, return `error` with reason `modem_path_lost`.
- Do not automatically write the candidate path into config and do not perform actions against the candidate.

## mmcli Capability and Parsing Strategy

The modem module should lazily detect and cache tool capabilities for the process:

- Run `mmcli --version` or equivalent once to populate `tool.version_raw`.
- Detect whether JSON output is supported.
- If capability detection times out or fails before proving `mmcli` is callable, report `tool.available = false`, `tool.supports_json = false`, and modem status `unknown` with reason `mmcli_probe_failed`.
- If JSON is supported, prefer JSON for full status parsing.
- If JSON is not supported, use a deliberately limited text fallback for core fields only: presence, enabled/state, SIM readiness when visible, signal quality when visible, and messaging availability when visible.
- Fields unavailable in text fallback return `null` and add `text_fallback_limited` to `health.reasons`.
- If fallback parsing cannot confidently identify the configured modem or core state, return `error` rather than layering multiple heuristic parsers.

Representative parser fixtures should be captured from real `mmcli` outputs when possible. Handwritten fixtures are acceptable only when they are named as synthetic and cover an explicit edge case.

## Backend Design

Add a small modem module behind the API layer. Its responsibilities:

- Execute `mmcli` with a short timeout using async process execution.
- Resolve the configured ModemManager object path to the target modem.
- Parse status output into SmsRelayed's stable model.
- Map status details into `ok`, `degraded`, `error`, or `unknown`.
- Execute enable, disable, and reset actions against only the configured modem.
- Return normalized errors for UI display.

The module must have a test seam: define an `MmcliRunner` trait or equivalent function-parameter injection so parser, resolution, timeout, and action behavior can be tested without modem hardware. Parser and classifier tests must not directly spawn `tokio::process::Command`.

Command behavior:

- Use a default 5 second timeout for status commands and action commands.
- Treat timeout as `error` with reason `mmcli_timeout`.
- Keep one action at a time with an in-process lock; reject concurrent actions with `409 conflict` rather than queueing them.
- Use accepted semantics for successful action submission; status changes are observed by later polling.

Logging rules:

- Do not log phone numbers, SMS bodies, SIM identifiers, or full command output.
- Log high-level failure categories such as tool missing, command timeout, configured modem not found, or action failed.
- Record enable, disable, and reset attempts as audit-style log entries with action, timestamp, result, and a short session/token hash when available. Do not log the full session token.
- Return concise authenticated diagnostics where useful.

## Frontend Design

Add a standalone Modem page instead of adding more UI to the current configuration editor.

Files:

- `frontend/src/routes/modem.tsx`
- `frontend/src/components/modem/modem-status-panel.tsx`

Navigation:

- Add a `Modem` link beside `SMS` and `Config`.
- Regenerate TanStack Router routes after adding the page.

`ModemStatusPanel` owns the feature behavior:

- Load `/api/modem/status` on mount.
- Provide a `Refresh` button.
- Show a status badge for `OK`, `Degraded`, `Error`, or `Unknown`.
- Show last check time.
- Show configured modem path and resolved modem identity.
- Show `mmcli` availability, raw version, and JSON support.
- Show enabled state, modem state, SIM state, operator, signal quality, access technologies, and messaging availability.
- Show diagnostic reasons and last error only when present.
- Provide `Enable` and `Disable` actions based on the current enabled state.
- Provide `Reset` in a destructive area with a confirmation dialog.
- After enable, disable, or reset, enter a polling state instead of refreshing once.
- Poll every 2 seconds for up to 30 seconds after reset. If the configured path does not return, show the path-drift diagnostic or a prompt to check `app.modem_path` in Config.

The page should not show bearer/IP/gateway/DNS/default-route information.

## Error Handling

The UI should distinguish:

- Loading status.
- `mmcli` missing.
- `mmcli` present but JSON unsupported, with limited diagnostics.
- ModemManager unreachable.
- Configured modem path not found.
- Possible modem path drift after reset.
- Modem present but degraded.
- Action rejected because reset was not confirmed.
- Action rejected by cross-origin or content-type checks.
- Action rejected because another modem action is already running.
- Action command failed or timed out.

The config editor must continue working even if the modem status page cannot read modem state.

## Documentation and Installer Updates

Because this feature makes `mmcli` a runtime dependency for modem status and control:

- Update README dependency documentation to mention `mmcli` for the Web modem page and health diagnostics.
- Update install or setup messaging so missing `mmcli` is a clear actionable warning for this feature.
- Keep existing SMS send/receive documentation clear that the core relay path still uses ModemManager D-Bus.
- Document that `/api/health` is public and should normally be exposed only on trusted networks or behind access control if the service is reachable beyond the device.

## Testing and Verification

Backend tests:

- Unit tests for parsing representative `mmcli` JSON output.
- Unit tests for limited text fallback parsing.
- Unit tests for path-to-index mapping from `mmcli -L`, including multiple modems, no match, and stale configured path.
- Unit tests for health classification using the explicit table.
- Unit tests for timeout handling through the injected runner.
- Unit tests for concurrent action handling: a second action while one is running is rejected with `409 conflict` and code `action_in_progress`.
- Unit test that reset without `{ "confirm": true }` is rejected.
- Unit tests for reset rate limiting.
- API route tests that prove `/api/health` is public while `/api/modem/*` routes are protected.
- API route construction test to catch axum route syntax panics.

Frontend checks:

- Regenerate TanStack Router route tree after adding `/modem`.
- Run the narrowest relevant Biome/type/build checks available for changed frontend files.

Fixtures:

- Add representative fixtures under a modem test fixture directory.
- Include at least: healthy registered modem, SIM not ready, modem disabled, messaging unavailable, JSON unsupported/text fallback, path mismatch, multiple modems, and command timeout.

End-to-end verification:

- `cargo test` from the repository root.
- Relevant frontend `pnpm` checks from `frontend/`.
- If the local environment lacks `mmcli` or modem hardware, verify the missing-tool and parse/classification paths with tests and report that live modem verification was not performed.

## Non-Goals

- No network route, default gateway, DNS, bearer, or WAN management.
- No factory reset.
- No arbitrary AT command execution.
- No phone number, SMS body, SIM identifier, or full command-output logging.
- No automatic persistent rewrite of `app.modem_path`.
- No broad redesign of the current configuration editor.
