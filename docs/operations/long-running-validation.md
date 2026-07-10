# Long-Running SMS Relay Validation

This document describes the repeatable soak and fault-injection procedure for
validating `sms-relayed` on the target ImmortalWrt board (~392 MB RAM, no swap).
Follow steps in order.  Record all results in the evidence log below.

## Prerequisites

- `sms-relayed` binary installed and configured on the target board.
- `ssh` access to the board.
- A backup of the production database before any destructive test.
- `sqlite3`, `pgrep`, `ps`, `logread` available on the board.

## 1. Baseline Recording

Before any fault cycles, record the steady-state baseline:

```sh
# Process info
pidof sms-relayed
cat /proc/$(pidof sms-relayed)/status | grep -E "VmRSS|Threads"
ls -1 /proc/$(pidof sms-relayed)/fd | wc -l
pgrep -P $(pidof sms-relayed) | wc -l

# Database size
ls -lh /etc/sms-relayed/sms-relayed.sqlite

# Delivery state
sqlite3 /etc/sms-relayed/sms-relayed.sqlite \
  "SELECT state, COUNT(*) FROM forward_deliveries GROUP BY state;"
```

## 2. Idle Sampler

Run the read-only sampler for 24 hours:

```sh
# Copy to board, then:
chmod +x soak-sms-relayed.sh
./soak-sms-relayed.sh --interval 60 --count 1440 > soak-idle-24h.tsv
```

Expected: RSS/PSS end-to-start delta ≤ 5 MiB, threads return to 5, no children.

## 3. Fault Injection Matrix

Each fault test requires operator supervision.  Record pass/fail for each row.

### 3.1 Black-Hole HTTP Provider

1. Configure a `bark` profile pointing to a non-routable address (e.g., `10.255.255.1`).
2. Send a test SMS.
3. Verify the delivery reaches `retry_wait` after the HTTP timeout.
4. Verify `mmcli` and `sh` children are zero 2 seconds after timeout.
5. Restore the correct profile URL.

### 3.2 Shell Child Timeout

1. Configure a `shell` profile whose script runs `sleep 120`.
2. Send a test SMS.
3. Verify the delivery enters `retry_wait` after the shell timeout (30s default).
4. Verify no child process survives (`pgrep -P $(pidof sms-relayed)`).
5. Restore the correct shell script or remove the profile.

### 3.3 ModemManager Restart and Path Drift

1. Record the current modem path: `mmcli -L`.
2. Restart ModemManager: `/etc/init.d/modemmanager restart`.
3. After restart, check `mmcli -L` — the modem may have a new path.
4. Verify `sms-relayed` reconnects and resumes SMS reception within 30 seconds.
5. Verify the health endpoint shows OK status.

### 3.4 Provider Outage across Restart

1. Configure an unreachable provider (black-hole address).
2. Send a test SMS — it enters `retry_wait`.
3. Restart `sms-relayed`: `/etc/init.d/sms-relayed restart`.
4. Verify the delivery is recovered (still in `retry_wait` or retried).
5. Restore reachable provider — verify the delivery eventually succeeds.

### 3.5 Concurrent Health Load

```sh
for i in $(seq 1 100); do
    curl -s http://localhost:8080/api/health > /dev/null &
done
wait
```

1. Verify health returns without error for all 100 requests.
2. Verify only one `mmcli` process was spawned during the burst.

### 3.6 Large Export

1. Copy the production database to a test copy: `cp /etc/sms-relayed/sms-relayed.sqlite /tmp/test-export.sqlite`
2. Point a temporary config to the test database.
3. Run: `curl -o /tmp/export.csv "http://localhost:8080/api/messages/export?format=csv"`
4. Verify the CSV is valid and complete.
5. Check peak RSS during export stays within 32 MiB above baseline.
6. Verify health responds within 1 second during export.
7. Clean up: `rm /tmp/test-export.sqlite /tmp/export.csv`

### 3.7 Retention Cleanup

1. On a copied test database, create old terminal messages:
   ```sql
   INSERT INTO messages (direction,phone_number,body,timestamp,status,source,created_at,updated_at)
   VALUES ('inbound','+15550000001','old','2020-01-01T00:00:00Z','received','modem','2020-01-01T00:00:00Z','2020-01-01T00:00:00Z');
   ```
2. Enable retention: set `retention.enabled = true` in config.
3. Restart the service (with the test database).
4. Verify the old message is deleted after the retention interval passes.
5. A message with a `pending` delivery must NOT be deleted.

## 4. Evidence Log

| Test | Date | Result | Operator | Notes |
|------|------|--------|----------|-------|
| Baseline | | | | |
| 24h idle | | | | |
| Black-hole HTTP | | | | |
| Shell timeout | | | | |
| ModemManager restart | | | | |
| Outage + restart | | | | |
| Health concurrency | | | | |
| Large export | | | | |
| Retention | | | | |

## 5. Threshold Summary

| Metric | Threshold |
|--------|-----------|
| Idle RSS/PSS delta | ≤ 5 MiB over 24h |
| Idle CPU | < 1% average |
| Threads | Return to 5 after each fault |
| Child processes | 0 after timeout cleanup |
| Receive recovery | ≤ 30 s after ModemManager restart |
| Retry concurrency | ≤ 2 (worker concurrency) |
| Delivery durability | All test messages have terminal/scheduled rows |
| Export peak RSS | ≤ 32 MiB above baseline for 100k rows |
| Export health response | ≤ 1 s locally |
| Health single-flight | 1 mmcli for 100 concurrent callers |
| Frontend bulk read | 1 mutation per conversation open |

## 6. Config Redactions

Before publishing any soak evidence, redact:
- Phone numbers (show last 4 digits only)
- SMS bodies (replace with `[REDACTED]`)
- Provider tokens and secrets
- Full session tokens
- SIM/device identifiers
- Raw `mmcli` output
