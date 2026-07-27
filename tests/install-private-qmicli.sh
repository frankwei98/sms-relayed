#!/bin/sh
set -eu

REPO_ROOT="$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)"
INSTALLER="$REPO_ROOT/scripts/install-private-qmicli-debian.sh"
TEST_TMP="$(mktemp -d)"
trap 'rm -rf "$TEST_TMP"' EXIT HUP INT TERM

fail() {
  printf 'not ok - %s\n' "$1" >&2
  exit 1
}

assert_contains() {
  file="$1"
  expected="$2"
  grep -F "$expected" "$file" >/dev/null || fail "$file does not contain: $expected"
}

make_fakes() {
  case_dir="$1"
  mkdir -p "$case_dir/bin"

  cat > "$case_dir/qmicli" <<'EOF'
#!/bin/sh
case "${1:-}" in
  --version) printf '%s\n' 'qmicli 1.36.0' ;;
  --help-all)
    printf '%s\n' \
      '--ims-get-ims-services-enabled-setting' \
      '--imsa-get-ims-registration-status' \
      '--imsa-get-ims-services-status'
    ;;
esac
EOF
  chmod +x "$case_dir/qmicli"

  cat > "$case_dir/bin/systemctl" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" >> "$FAKE_SYSTEMCTL_LOG"
case "${1:-}" in
  is-active) [ "${FAKE_SERVICE_ACTIVE:-0}" = "1" ] ;;
  daemon-reload) [ "${FAKE_RELOAD_FAIL:-0}" != "1" ] ;;
  restart) [ "${FAKE_RESTART_FAIL:-0}" != "1" ] ;;
esac
EOF
  chmod +x "$case_dir/bin/systemctl"
}

run_installer() {
  case_dir="$1"
  shift
  SMS_RELAYED_INSTALLER_TEST_ROOT="$case_dir/root" \
  SMS_RELAYED_INSTALLER_TEST_QMICLI="$case_dir/qmicli" \
  FAKE_SYSTEMCTL_LOG="$case_dir/systemctl.log" \
  PATH="$case_dir/bin:$PATH" \
    sh "$INSTALLER" "$@"
}

case_active="$TEST_TMP/active"
make_fakes "$case_active"
FAKE_SERVICE_ACTIVE=1
export FAKE_SERVICE_ACTIVE
run_installer "$case_active"
drop_in="$case_active/root/etc/systemd/system/sms-relayed.service.d/qmicli.conf"
assert_contains "$drop_in" "SMS_RELAYED_QMICLI_PATH=/opt/sms-relayed/libqmi/bin/qmicli-wrapper"
assert_contains "$case_active/systemctl.log" "restart sms-relayed.service"
[ "$(readlink "$case_active/root/opt/sms-relayed/libqmi")" = "libqmi-1.36.0" ] ||
  fail "current symlink does not select libqmi 1.36.0"

case_inactive="$TEST_TMP/inactive"
make_fakes "$case_inactive"
FAKE_SERVICE_ACTIVE=0
export FAKE_SERVICE_ACTIVE
run_installer "$case_inactive"
if grep -F "restart sms-relayed.service" "$case_inactive/systemctl.log" >/dev/null; then
  fail "inactive service was restarted"
fi

case_no_restart="$TEST_TMP/no-restart"
make_fakes "$case_no_restart"
FAKE_SERVICE_ACTIVE=1
export FAKE_SERVICE_ACTIVE
run_installer "$case_no_restart" --no-restart
if grep -F "restart sms-relayed.service" "$case_no_restart/systemctl.log" >/dev/null; then
  fail "--no-restart restarted the service"
fi

case_rollback="$TEST_TMP/rollback"
make_fakes "$case_rollback"
FAKE_SERVICE_ACTIVE=1
FAKE_RESTART_FAIL=1
export FAKE_SERVICE_ACTIVE FAKE_RESTART_FAIL
if run_installer "$case_rollback"; then
  fail "restart failure did not fail the installer"
fi
[ ! -e "$case_rollback/root/opt/sms-relayed/libqmi" ] ||
  fail "restart failure left the private symlink"
[ ! -e "$case_rollback/root/opt/sms-relayed/libqmi-1.36.0" ] ||
  fail "restart failure left the newly deployed version"
[ ! -e "$case_rollback/root/etc/systemd/system/sms-relayed.service.d/qmicli.conf" ] ||
  fail "restart failure left the systemd drop-in"
unset FAKE_RESTART_FAIL

case_reload_rollback="$TEST_TMP/reload-rollback"
make_fakes "$case_reload_rollback"
FAKE_SERVICE_ACTIVE=0
FAKE_RELOAD_FAIL=1
export FAKE_SERVICE_ACTIVE FAKE_RELOAD_FAIL
if run_installer "$case_reload_rollback"; then
  fail "daemon-reload failure did not fail the installer"
fi
[ ! -e "$case_reload_rollback/root/opt/sms-relayed/libqmi" ] ||
  fail "daemon-reload failure left the private symlink"
[ ! -e "$case_reload_rollback/root/opt/sms-relayed/libqmi-1.36.0" ] ||
  fail "daemon-reload failure left the newly deployed version"
[ ! -e "$case_reload_rollback/root/etc/systemd/system/sms-relayed.service.d/qmicli.conf" ] ||
  fail "daemon-reload failure left the systemd drop-in"
unset FAKE_RELOAD_FAIL

case_uninstall="$TEST_TMP/uninstall"
make_fakes "$case_uninstall"
FAKE_SERVICE_ACTIVE=0
export FAKE_SERVICE_ACTIVE
run_installer "$case_uninstall"
run_installer "$case_uninstall" --uninstall
[ ! -e "$case_uninstall/root/opt/sms-relayed/libqmi" ] ||
  fail "uninstall left the managed symlink"
[ ! -e "$case_uninstall/root/opt/sms-relayed/libqmi-1.36.0" ] ||
  fail "uninstall left the managed version directory"
[ ! -e "$case_uninstall/root/etc/systemd/system/sms-relayed.service.d/qmicli.conf" ] ||
  fail "uninstall left the systemd drop-in"

case_uninstall_reload_rollback="$TEST_TMP/uninstall-reload-rollback"
make_fakes "$case_uninstall_reload_rollback"
FAKE_SERVICE_ACTIVE=0
export FAKE_SERVICE_ACTIVE
run_installer "$case_uninstall_reload_rollback"
FAKE_SERVICE_ACTIVE=1
FAKE_RELOAD_FAIL=1
export FAKE_SERVICE_ACTIVE FAKE_RELOAD_FAIL
if run_installer "$case_uninstall_reload_rollback" --uninstall; then
  fail "uninstall daemon-reload failure did not fail"
fi
[ -L "$case_uninstall_reload_rollback/root/opt/sms-relayed/libqmi" ] ||
  fail "uninstall daemon-reload failure did not restore the symlink"
[ -d "$case_uninstall_reload_rollback/root/opt/sms-relayed/libqmi-1.36.0" ] ||
  fail "uninstall daemon-reload failure did not restore the version"
[ -f "$case_uninstall_reload_rollback/root/etc/systemd/system/sms-relayed.service.d/qmicli.conf" ] ||
  fail "uninstall daemon-reload failure did not restore the drop-in"
unset FAKE_RELOAD_FAIL

case_uninstall_restart_rollback="$TEST_TMP/uninstall-restart-rollback"
make_fakes "$case_uninstall_restart_rollback"
FAKE_SERVICE_ACTIVE=0
export FAKE_SERVICE_ACTIVE
run_installer "$case_uninstall_restart_rollback"
FAKE_SERVICE_ACTIVE=1
FAKE_RESTART_FAIL=1
export FAKE_SERVICE_ACTIVE FAKE_RESTART_FAIL
if run_installer "$case_uninstall_restart_rollback" --uninstall; then
  fail "uninstall restart failure did not fail"
fi
[ -L "$case_uninstall_restart_rollback/root/opt/sms-relayed/libqmi" ] ||
  fail "uninstall restart failure did not restore the symlink"
[ -d "$case_uninstall_restart_rollback/root/opt/sms-relayed/libqmi-1.36.0" ] ||
  fail "uninstall restart failure did not restore the version"
[ -f "$case_uninstall_restart_rollback/root/etc/systemd/system/sms-relayed.service.d/qmicli.conf" ] ||
  fail "uninstall restart failure did not restore the drop-in"
unset FAKE_RESTART_FAIL

case_unowned="$TEST_TMP/unowned"
make_fakes "$case_unowned"
mkdir -p "$case_unowned/root/opt/sms-relayed/libqmi-1.36.0"
printf '%s\n' "operator data" > "$case_unowned/root/opt/sms-relayed/libqmi-1.36.0/keep"
FAKE_SERVICE_ACTIVE=0
export FAKE_SERVICE_ACTIVE
if run_installer "$case_unowned" --uninstall; then
  fail "uninstall accepted an unowned version directory"
fi
assert_contains "$case_unowned/root/opt/sms-relayed/libqmi-1.36.0/keep" "operator data"

case_unmanaged_binding="$TEST_TMP/unmanaged-binding"
make_fakes "$case_unmanaged_binding"
mkdir -p \
  "$case_unmanaged_binding/root/opt/sms-relayed" \
  "$case_unmanaged_binding/root/etc/systemd/system/sms-relayed.service.d"
ln -s "operator-libqmi" "$case_unmanaged_binding/root/opt/sms-relayed/libqmi"
printf '%s\n' "operator drop-in" > "$case_unmanaged_binding/root/etc/systemd/system/sms-relayed.service.d/qmicli.conf"
if run_installer "$case_unmanaged_binding"; then
  fail "installer overwrote an unmanaged binding"
fi
[ "$(readlink "$case_unmanaged_binding/root/opt/sms-relayed/libqmi")" = "operator-libqmi" ] ||
  fail "installer changed an unmanaged symlink"
assert_contains "$case_unmanaged_binding/root/etc/systemd/system/sms-relayed.service.d/qmicli.conf" "operator drop-in"

case_spoofed_drop_in="$TEST_TMP/spoofed-drop-in"
make_fakes "$case_spoofed_drop_in"
mkdir -p "$case_spoofed_drop_in/root/etc/systemd/system/sms-relayed.service.d"
printf '%s\n' \
  "# Managed by install-private-qmicli-debian.sh" \
  "[Service]" \
  "Environment=SMS_RELAYED_QMICLI_PATH=/operator/qmicli" \
  > "$case_spoofed_drop_in/root/etc/systemd/system/sms-relayed.service.d/qmicli.conf"
if run_installer "$case_spoofed_drop_in"; then
  fail "installer accepted a drop-in with only the ownership comment"
fi
assert_contains \
  "$case_spoofed_drop_in/root/etc/systemd/system/sms-relayed.service.d/qmicli.conf" \
  "SMS_RELAYED_QMICLI_PATH=/operator/qmicli"

printf '%s\n' "ok - private qmicli installer"
