#!/bin/sh
# soak-sms-relayed.sh — Read-only sampler for long-running sms-relayed validation.
#
# Usage:  ./scripts/soak-sms-relayed.sh [--interval SECS] [--count N]
# Default: samples every 60 seconds, 1440 iterations (24 hours).
#
# This script samples RSS, threads, file descriptors, child processes,
# database size, and delivery counts via read-only commands.  It never
# writes to the database, restarts the service, or injects faults.
# Sensitive fields (phone numbers, SMS bodies, tokens, mmcli raw output)
# are not printed.
#
# Output: tab-separated lines with column headers on the first row.

INTERVAL=${1:-60}
COUNT=${2:-1440}
SERVICE="sms-relayed"
DB="${DB:-/etc/sms-relayed/sms-relayed.sqlite}"
CONFIG="${CONFIG:-/etc/sms-relayed/config.toml}"
PID_FILE="${PID_FILE:-/var/run/sms-relayed.pid}"

SEP="	"

# Header
printf "ts${SEP}rss_kb${SEP}threads${SEP}fds${SEP}children${SEP}db_bytes${SEP}delivery_pending${SEP}delivery_in_flight${SEP}delivery_retry${SEP}delivery_failed${SEP}note\n"

I=0
while [ "$I" -lt "$COUNT" ]; do
	TS=$(date -Iseconds 2>/dev/null || date +%Y-%m-%dT%H:%M:%S%z)
	RSS=""
	THR=""
	FDS=""
	CHILDREN=""
	DB_BYTES=""
	DEL_PEND=""
	DEL_INF=""
	DEL_RET=""
	DEL_FAIL=""
	NOTE=""

	PID=""
	if [ -f "$PID_FILE" ]; then
		PID=$(cat "$PID_FILE" 2>/dev/null)
	fi
	if [ -z "$PID" ] || ! kill -0 "$PID" 2>/dev/null; then
		PID=$(pgrep -x "$SERVICE" 2>/dev/null | head -1)
	fi

	if [ -n "$PID" ]; then
		# RSS in kB from /proc/pid/status
		if [ -r "/proc/$PID/status" ]; then
			RSS=$(awk '/VmRSS/ {print $2}' "/proc/$PID/status" 2>/dev/null)
		fi
		# Thread count
		if [ -r "/proc/$PID/status" ]; then
			THR=$(awk '/Threads/ {print $2}' "/proc/$PID/status" 2>/dev/null)
		fi
		# File descriptors
		if [ -d "/proc/$PID/fd" ]; then
			FDS=$(ls -1 "/proc/$PID/fd" 2>/dev/null | wc -l)
			FDS=$((FDS + 0))
		fi
		# Child processes
		CHILDREN=$(pgrep -P "$PID" 2>/dev/null | wc -l)
		CHILDREN=$((CHILDREN + 0))
	else
		NOTE="process_not_found"
	fi

	if [ -f "$DB" ]; then
		DB_BYTES=$(stat -c%s "$DB" 2>/dev/null || stat -f%z "$DB" 2>/dev/null)
		if command -v sqlite3 >/dev/null 2>&1; then
			DEL_PEND=$(sqlite3 "$DB" "SELECT COUNT(*) FROM forward_deliveries WHERE state='pending'" 2>/dev/null || echo "")
			DEL_INF=$(sqlite3 "$DB" "SELECT COUNT(*) FROM forward_deliveries WHERE state='in_flight'" 2>/dev/null || echo "")
			DEL_RET=$(sqlite3 "$DB" "SELECT COUNT(*) FROM forward_deliveries WHERE state='retry_wait'" 2>/dev/null || echo "")
			DEL_FAIL=$(sqlite3 "$DB" "SELECT COUNT(*) FROM forward_deliveries WHERE state='permanent_failed'" 2>/dev/null || echo "")
		fi
	fi

	printf "%s${SEP}%s${SEP}%s${SEP}%s${SEP}%s${SEP}%s${SEP}%s${SEP}%s${SEP}%s${SEP}%s${SEP}%s\n" \
		"$TS" "$RSS" "$THR" "$FDS" "$CHILDREN" "$DB_BYTES" \
		"$DEL_PEND" "$DEL_INF" "$DEL_RET" "$DEL_FAIL" "$NOTE"

	I=$((I + 1))
	sleep "$INTERVAL"
done
