# P2 Web API and Frontend Design

Date: 2026-07-08
Status: Approved design

## Purpose

P2 adds an authenticated local web management console to `sms-relayed`. The console is for users managing a router or gateway with a SIM attached through ModemManager. It should make SMS history, sending, and configuration management available from a browser on the router WiFi/LAN.

P2 builds on the P1 typed TOML config and runtime structure. The older `src/web.rs` send-only page/API is legacy and should be replaced rather than extended.

## Goals

- Run the Web API and frontend from the existing `sms-relayed run` service.
- Store inbound and outbound SMS history in SQLite.
- Provide password-protected browser access with a simple single-password model.
- Support SMS receive history, sending, search, deletion, unread state, and CSV/JSON export.
- Provide full config editing for `app`, `sms`, `forward`, `channels`, and `api` sections.
- Provide config validation and service restart actions from the UI.
- Keep deployment simple for OpenWrt: one service process, one binary/release artifact path, no separate web server requirement.

## Non-Goals

- Multi-user accounts or role-based access control.
- HTTPS termination inside `sms-relayed`.
- Contact management or address book features.
- Hot reloading the full runtime after config saves.
- Operating on SIM/ModemManager stored SMS when deleting local history.
- Complex frontend styling beyond a practical shadcn-based management UI.

## Runtime Architecture

`sms-relayed run` becomes the single long-running service. It loads the config once on startup, validates it, and starts these cooperating parts:

1. D-Bus SMS monitor for ModemManager receive events.
2. SQLite storage for inbound and outbound message history.
3. Axum web server for JSON APIs, SSE events, cookie sessions, and the React frontend.

The API server is controlled by `[api]`, but P2 defaults it on. The default API config should include:

```toml
[api]
enabled = true
bind = "0.0.0.0"
port = 8080
enable_ipv6 = false
password = ""
database_path = "/etc/sms-relayed/sms-relayed.sqlite"
```

The default bind is IPv4 LAN-friendly because the expected device is behind NAT and accessed from the router WiFi. IPv6 listening is disabled by default and can be enabled explicitly later through config.

If `api.enabled = true` and `api.password` is empty, the Web API must not start. Setup should prompt for a password before enabling the API, and the runtime should fail fast with a clear config error if an enabled API has no password.

Config changes made through the UI are written to TOML but do not hot reload the running service. The UI must show that a restart is required. A restart endpoint schedules a service restart after returning its HTTP response.

## Storage Design

Use SQLite for message history. P2 needs one primary table:

```text
messages
- id INTEGER PRIMARY KEY AUTOINCREMENT
- direction TEXT NOT NULL              -- inbound | outbound
- phone_number TEXT NOT NULL
- body TEXT NOT NULL
- timestamp TEXT NOT NULL              -- modem timestamp for inbound, local time for outbound
- status TEXT NOT NULL                 -- received | sending | sent | failed
- source TEXT NOT NULL                 -- modem | web | cli
- modem_sms_path TEXT NULL
- read_at TEXT NULL                    -- NULL means unread
- error TEXT NULL
- created_at TEXT NOT NULL
- updated_at TEXT NOT NULL
```

Inbound messages default to unread by storing `read_at = NULL`. Outbound messages can be considered read immediately.

Recommended indexes:

- `phone_number`
- `timestamp`
- `direction`
- `status`
- `read_at`

Text search can start with SQLite `LIKE` over `phone_number` and `body`. If FTS5 is straightforward in the selected Rust SQLite crate and target environment, implementation may use it, but P2 should not depend on FTS5 being available.

Deletion is hard deletion from the local SQLite history. It does not delete SMS records from SIM storage or ModemManager.

## Message Flow

### Inbound

1. D-Bus monitor receives a ModemManager SMS `Added` signal.
2. Existing storage filtering and content retry behavior still apply.
3. Once a complete inbound SMS is available, insert it into SQLite with `direction = inbound`, `status = received`, `source = modem`, and `read_at = NULL`.
4. Broadcast an SSE `message.created` event.
5. Run the existing forwarding profiles.

Forwarding failure does not change the SMS record status. The message was received successfully; forwarding failures remain logs/diagnostics for the relay path.

### Outbound From Web/API

1. API validates `phone_number` and `body`.
2. Insert an outbound row with `status = sending`, `source = web`, and a non-null `read_at`.
3. Call ModemManager through the existing send path.
4. On success, update the row to `status = sent`.
5. On failure, update the row to `status = failed` and store the error string.
6. Broadcast `message.created` and `message.updated` SSE events as state changes.

CLI sends may also be written into the same table with `source = cli`, but P2 must at least persist Web/API sends.

## Authentication

Use a single password stored as plaintext in `[api].password`. This keeps setup simple and matches the existing config-file security model where `/etc/sms-relayed/config.toml` is written with restrictive permissions.

Login uses cookie sessions:

- `POST /api/auth/login` checks the configured password and sets an HttpOnly cookie.
- `POST /api/auth/logout` clears the cookie.
- `GET /api/auth/me` reports the current auth state.

All `/api/*` routes except login and `auth/me` require a valid session cookie. Session tokens should be random, stored server-side in memory, and invalidated on process restart.

## API Design

Errors use a consistent JSON shape:

```json
{
  "error": {
    "code": "config_invalid",
    "message": "app.modem_path must be a ModemManager modem object path"
  }
}
```

### Auth

- `POST /api/auth/login`
  - Body: `{ "password": "..." }`
  - Success: sets HttpOnly cookie and returns `{ "authenticated": true }`.
- `POST /api/auth/logout`
  - Clears the session cookie.
- `GET /api/auth/me`
  - Returns `{ "authenticated": true }` for authenticated sessions and `{ "authenticated": false }` otherwise.

### Messages

- `GET /api/messages`
  - Query parameters:
    - `limit`
    - `before_id`
    - `phone_number`
    - `q`
    - `direction`
    - `status`
    - `unread`
    - `from`
    - `to`
  - Returns paginated message rows ordered newest first.
- `GET /api/conversations`
  - Groups messages by `phone_number`, ordered by each conversation's latest message timestamp, and returns last message summary plus unread count.
- `POST /api/messages/send`
  - Body: `{ "phone_number": "...", "body": "..." }`
  - Sends through ModemManager and returns the message row.
- `POST /api/messages/:id/read`
  - Sets `read_at`.
- `POST /api/messages/:id/unread`
  - Clears `read_at`.
- `POST /api/conversations/:phone_number/read`
  - Marks all inbound messages for that phone number as read.
- `DELETE /api/messages/:id`
  - Deletes a local history row.
- `POST /api/messages/delete`
  - Body: `{ "ids": [1, 2, 3] }`
  - Deletes multiple local history rows.
- `GET /api/messages/export`
  - Query parameters include all message filters plus `format=csv|json`.
  - Exports the full filtered result set, not just the current page limit.

CSV export uses stable columns:

```text
id,direction,phone_number,body,timestamp,status,source,read_at,error,created_at,updated_at
```

JSON export returns an array of message objects with the same fields.

### Events

- `GET /api/events`
  - Server-Sent Events stream for authenticated sessions.
  - Event names:
    - `message.created`
    - `message.updated`
    - `message.deleted`
    - `message.read_state_changed`
    - `config.saved`
    - `service.restart_scheduled`

The frontend should load initial state through ordinary HTTP endpoints, then use SSE for live updates and reconnect recovery.

### Config

- `GET /api/config`
  - Returns the current typed config as JSON.
  - Secrets are returned because this is an authenticated full-management UI.
- `PUT /api/config`
  - Accepts the full config JSON, validates it with the same validation rules as CLI config check, writes TOML, and returns `{ "requires_restart": true }`.
- `POST /api/config/check`
  - Accepts either config JSON or TOML text, parses and validates it, and returns success or validation errors without writing to disk.

The frontend's config check button uses `POST /api/config/check`.

### Service

- `POST /api/service/restart`
  - Returns `202 Accepted`, emits `service.restart_scheduled`, then asynchronously restarts the service.
  - On OpenWrt, prefer `/etc/init.d/sms-relayed restart`.
  - On systemd systems, use `systemctl restart sms-relayed`.
  - The implementation must delay execution briefly so the HTTP response can reach the browser before the process exits.
- `GET /api/status`
  - Returns version, uptime, API bind/port, database path, and basic runtime status.

## Frontend Design

The frontend is a plain management console using React, TanStack Router, Tailwind CSS, and shadcn components.

Routes:

- `/login`
  - Password form.
  - Redirects to `/` on success.
- `/`
  - SMS console.
  - Conversation/phone-number list with unread counts.
  - Message timeline or table.
  - Send box.
  - Search and filters for number, text, direction, status, unread state, and time range.
  - Mark read/unread actions.
  - Single and batch delete.
  - Export CSV and JSON actions using the current filters.
  - SSE-driven live updates.
- `/config`
  - Full config editor split by sections:
    - `app`: device name, modem path
    - `sms`: ignored storage, code keywords
    - `api`: enabled, bind, port, IPv6 enablement, password, database path
    - `forward`: enabled profiles
    - `channels`: CRUD for Bark, Telegram, PushPlus, WeCom, DingTalk, and Shell profiles
  - Buttons: check, save, restart service.
  - Save shows a restart-required state.

Use shadcn components through the required pinned command form when adding components:

```sh
pnpm dlx --package shadcn@latest --package zod@3.25.76 shadcn add <component>
```

Likely components include button, input, textarea, select, tabs, dialog, table, badge, switch, checkbox, and toast/sonner equivalents if needed.

After route changes, run `pnpm generate-routes` from `frontend/`.

## Config Editing Rules

The Web UI edits the full typed config model, not a partial patch format. On save:

1. Backend deserializes submitted JSON into `AppConfig`.
2. Backend validates through `AppConfig::validate()` and any new API-specific validation.
3. Backend writes TOML securely to the active config path.
4. Backend returns `requires_restart = true`.

Config check performs the same parse/validate process without writing. It should be available even before save so users can validate proposed changes.

## Implementation Notes

- Replace or rewrite `src/web.rs`; do not preserve the legacy unauthenticated GET send API.
- Introduce a storage module for SQLite migrations and message queries.
- Introduce an API module for Axum routes, auth/session middleware, SSE broadcaster, and static frontend serving.
- Decouple SMS sending behind a small interface so API tests can mock sends without real D-Bus.
- Keep phone numbers and SMS bodies out of logs unless explicitly needed for user-requested diagnostics.
- Frontend assets should be packaged with the release in the simplest OpenWrt-friendly way. Prefer embedding or a deterministic install path over requiring a separate web server.

## Testing and Verification

Rust checks:

- SQLite migration tests.
- Message insert/search/delete/export query tests.
- Read/unread state tests.
- Config check/save API tests.
- Auth cookie/session tests.
- Send API tests with mocked sender.
- `cargo test`.
- `cargo fmt --check`.

Frontend checks:

- API client and key component tests where useful.
- `pnpm generate-routes` after route changes.
- `pnpm check`.
- `pnpm build`.

Manual acceptance:

- Start `sms-relayed run` with default API config.
- Log in through the browser.
- Receive an SMS, see it appear through SSE, and see unread count update.
- Send a reply and see `sending` become `sent` or `failed`.
- Search/filter messages.
- Mark messages read and unread.
- Delete one and multiple local history messages.
- Export filtered messages as CSV and JSON.
- Edit config, check config, save config, and trigger restart.

## Open Implementation Decisions

These can be decided during planning without changing product scope:

- Exact Rust SQLite crate (`rusqlite` vs `sqlx`) based on OpenWrt cross-compilation constraints.
- Whether frontend assets are embedded in the binary or installed next to it.
- Exact session cookie name and session expiration duration.
- Whether CLI sends are persisted in P2 or added immediately after Web/API persistence.
