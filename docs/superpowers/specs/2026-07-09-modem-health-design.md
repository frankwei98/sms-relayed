# Modem Health and Control Design

Date: 2026-07-09

## Goal

Add a modem health and control surface for SmsRelayed so the operator can quickly tell whether the configured ModemManager modem is usable for SMS relay and perform basic modem actions from the web UI.

The feature has two layers:

- A public, minimal health endpoint for monitoring.
- An authenticated modem status and control page for the web UI.

This feature focuses on SMS relay health. It does not manage bearer connections, default routes, IP addresses, gateways, DNS, or cellular-as-WAN behavior.

## Runtime Dependency

`mmcli` is an accepted runtime dependency.

The backend should call `mmcli` from Rust and map command results into SmsRelayed's own API model. The API should not require the frontend to understand raw `mmcli` output. Full raw command output should not be logged. Error summaries may be returned to authenticated users after truncation or normalization.

If `mmcli` is missing, the public health endpoint should report an unhealthy state instead of panicking. The authenticated status endpoint should return a clear tool-missing error or status payload that the UI can render.

## API Design

### Public Health

`GET /api/health`

This route is public and does not require login. It returns a compact payload suitable for uptime or health monitoring:

```json
{
  "status": "ok",
  "modem_present": true,
  "modem_enabled": true,
  "messaging_available": true,
  "checked_at": "2026-07-09T12:00:00Z",
  "reason": null
}
```

`status` values:

- `ok`: configured modem is present, enabled, SIM is ready, registration is usable, and messaging is available.
- `degraded`: the modem is present but one or more SMS-relay prerequisites are not healthy.
- `error`: `mmcli` is missing, ModemManager is unreachable, or the configured modem cannot be found.

The public endpoint should not expose full `mmcli` output, operator details, SIM identifiers, phone numbers, SMS bodies, or network-layer details.

### Authenticated Modem Status

`GET /api/modem/status`

This route requires the existing web session. It returns a stable structured model for the Modem page:

```json
{
  "checked_at": "2026-07-09T12:00:00Z",
  "tool": {
    "available": true,
    "version": "mmcli version"
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
    "last_error": null
  }
}
```

Fields that cannot be read should be returned as `null` or omitted where appropriate, with a reason added to `health.reasons`. Partial status should not become HTTP 500 when the configured modem can still be identified.

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

Missing or false confirmation returns `400 bad_request`.

Reset means the ModemManager modem reset operation, equivalent to `mmcli --reset`: the modem may disconnect, disappear temporarily, and re-enumerate. This is not a factory reset. The implementation must not expose factory reset, arbitrary AT commands, route changes, bearer management, or network interface control.

## Backend Design

Add a small modem module behind the API layer. Its responsibilities:

- Execute `mmcli` with a short timeout using async process execution.
- Resolve the configured ModemManager object path to the target modem.
- Parse status output into SmsRelayed's stable model.
- Map status details into `ok`, `degraded`, or `error`.
- Execute enable, disable, and reset actions against only the configured modem.
- Return normalized errors for UI display.

The module should keep command execution, parsing, and health classification separate enough to test without requiring modem hardware.

Preferred command behavior:

- Use `mmcli --modem <target> --output-json` when available.
- Keep the API model stable even if parsing needs to fall back to text output for older target environments.
- Use short command timeouts so the API cannot hang indefinitely.

Logging rules:

- Do not log phone numbers, SMS bodies, SIM identifiers, or full command output.
- Log high-level failure categories such as tool missing, command timeout, configured modem not found, or action failed.
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
- Show a status badge for `OK`, `Degraded`, or `Error`.
- Show last check time.
- Show configured modem path and resolved modem identity.
- Show enabled state, modem state, SIM state, operator, signal quality, access technologies, and messaging availability.
- Show diagnostic reasons and last error only when present.
- Provide `Enable` and `Disable` actions based on the current enabled state.
- Provide `Reset` in a destructive area with a confirmation dialog.
- After enable or disable, refresh status immediately.
- After reset, show an accepted/in-progress state and refresh after a short delay because the modem may temporarily disappear.

The page should not show bearer/IP/gateway/DNS/default-route information.

## Error Handling

The UI should distinguish:

- Loading status.
- `mmcli` missing.
- ModemManager unreachable.
- Configured modem path not found.
- Modem present but degraded.
- Action rejected because reset was not confirmed.
- Action command failed or timed out.

The config editor must continue working even if the modem status page cannot read modem state.

## Testing and Verification

Backend tests:

- Unit tests for parsing representative `mmcli` output.
- Unit tests for health classification.
- Unit test that reset without `{ "confirm": true }` is rejected.
- API route construction test to catch axum route syntax panics.

Frontend checks:

- Regenerate TanStack Router route tree after adding `/modem`.
- Run the narrowest relevant Biome/type/build checks available for changed frontend files.

End-to-end verification:

- `cargo test` from the repository root.
- Relevant frontend `pnpm` checks from `frontend/`.
- If the local environment lacks `mmcli` or modem hardware, verify the missing-tool and parse/classification paths with tests and report that live modem verification was not performed.

## Non-Goals

- No network route, default gateway, DNS, bearer, or WAN management.
- No factory reset.
- No arbitrary AT command execution.
- No phone number, SMS body, SIM identifier, or full command-output logging.
- No broad redesign of the current configuration editor.
