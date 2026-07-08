# P2 API and Frontend Discussion Summary

Date: 2026-07-08

## Purpose

This note captures only the P2 direction discussed before the machine switch. It is not an approved implementation design. It exists so work can continue on another machine without losing the product intent.

## Context From P1

P1 is the CLI and installer overhaul:

- OpenWrt/procd first, with systemd compatibility.
- `curl -fsSL https://raw.githubusercontent.com/frankwei98/sms-relayed/main/install.sh | sh`.
- New interactive CLI setup using `inquire`.
- New typed TOML config at `/etc/sms-relayed/config.toml`.
- Multiple named push channel profiles.
- Existing six push channels kept: Bark, Telegram, PushPlus, WeCom, DingTalk, Shell.
- Default service behavior is SMS forwarding.

P2 is explicitly after P1 and should build on the new P1 config/runtime structure.

## Discussed P2 Direction

The user wants a new API and frontend for SMS management.

Confirmed product requirements:

- Build a new API.
- Build a matching frontend.
- The frontend has password protection only.
- Users can review previously received SMS messages in the frontend.
- Users can reply to messages from the frontend.
- P2 should work with the existing ModemManager send/receive behavior.

The older `src/web.rs` implementation is not the intended P2 experience. It is a legacy unauthenticated send-only page/API and should be treated as replaceable when P2 is designed.

## Known Product Shape

The P2 experience should feel like a small local SMS console for the OpenWrt device:

- Login with a password.
- See a history of received SMS.
- Open a previous message.
- Reply to that message.
- Send through the device SIM via ModemManager.

The password model discussed is intentionally simple: password protection only, not multi-user accounts.

## Open Questions For P2 Design

These were not confirmed and need discussion before implementation:

- Where and how to store SMS history.
- Whether outbound SMS attempts should also be stored.
- Which database or storage format to use.
- Whether the API/frontend runs inside the same `sms-relayed run` process or as a separate mode.
- Whether the web server is enabled by default or only after explicit setup.
- Exact API routes and JSON shapes.
- Exact frontend stack and asset packaging.
- Session/cookie details for password protection.
- Whether API should be LAN-only by default, and what bind address should be used.
- Whether the frontend needs polling, live updates, or only manual refresh.
- Whether message deletion, search, filters, or export belong in P2 or later.
- How setup should collect or rotate the web password.
- What acceptance tests should define P2 done.

## Suggested Next Step

Before implementing P2, run a short design pass focused on the open questions above. The first decisions to settle should be:

1. Storage model for received SMS history.
2. Runtime shape for API/frontend.
3. Password/session behavior.
4. Frontend scope for the first usable version.

After those are confirmed, write a real P2 design spec and implementation plan.
