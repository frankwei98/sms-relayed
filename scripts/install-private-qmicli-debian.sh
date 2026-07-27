#!/bin/sh
set -eu

LIBQMI_VERSION="1.36.0"
LIBQMI_SHA256="e254fafdd916a78a27126e6d72ae436662487f59c7de84d4d40286059af89093"
LIBQMI_URL="https://gitlab.freedesktop.org/mobile-broadband/libqmi/-/archive/$LIBQMI_VERSION/libqmi-$LIBQMI_VERSION.tar.gz"
LOGICAL_BASE="/opt/sms-relayed"
LOGICAL_VERSION_DIR="$LOGICAL_BASE/libqmi-$LIBQMI_VERSION"
LOGICAL_CURRENT="$LOGICAL_BASE/libqmi"
SERVICE="sms-relayed.service"
DROP_IN_LOGICAL="/etc/systemd/system/$SERVICE.d/qmicli.conf"
TEST_ROOT="${SMS_RELAYED_INSTALLER_TEST_ROOT:-}"
TEST_QMICLI="${SMS_RELAYED_INSTALLER_TEST_QMICLI:-}"
NO_RESTART=0
UNINSTALL=0
BUILD_WORK_DIR=""
STATE_DIR=""
BUILT_PAYLOAD=""
DEPLOYED_NEW=0
MARKER_NAME=".sms-relayed-private-qmicli"
MARKER_VALUE="sms-relayed-private-qmicli:$LIBQMI_VERSION"

log() { printf '%s\n' "$*"; }
warn() { printf 'warning: %s\n' "$*" >&2; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

cleanup() {
  if [ -n "$BUILD_WORK_DIR" ] && [ -d "$BUILD_WORK_DIR" ]; then
    rm -rf "$BUILD_WORK_DIR"
  fi
  if [ -n "$STATE_DIR" ] && [ -d "$STATE_DIR" ]; then
    rm -rf "$STATE_DIR"
  fi
}
trap cleanup EXIT HUP INT TERM

usage() {
  cat <<'EOF'
Usage: install-private-qmicli-debian.sh [--no-restart] [--uninstall]

Build and install a private libqmi/qmicli 1.36.0 for sms-relayed on Debian 12.
The system libqmi packages are not replaced.

  --no-restart  Install or remove the systemd binding without restarting service
  --uninstall   Remove only the private 1.36.0 install and managed drop-in
EOF
}

for argument in "$@"; do
  case "$argument" in
    --no-restart) NO_RESTART=1 ;;
    --uninstall) UNINSTALL=1 ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; die "unknown option: $argument" ;;
  esac
done

rooted() {
  printf '%s%s\n' "$TEST_ROOT" "$1"
}

DEST_BASE="$(rooted "$LOGICAL_BASE")"
VERSION_DIR="$(rooted "$LOGICAL_VERSION_DIR")"
CURRENT_LINK="$(rooted "$LOGICAL_CURRENT")"
DROP_IN="$(rooted "$DROP_IN_LOGICAL")"

require_supported_host() {
  if [ -n "$TEST_ROOT" ]; then
    [ -x "$TEST_QMICLI" ] || die "test qmicli is not executable"
    return
  fi
  [ "$(id -u)" = "0" ] || die "run this installer as root"
  [ -r /etc/os-release ] || die "/etc/os-release is missing"
  # shellcheck disable=SC1091
  . /etc/os-release
  [ "${ID:-}" = "debian" ] || die "this helper supports Debian only"
  [ "${VERSION_ID:-}" = "12" ] || die "this helper is pinned to Debian 12"
  command -v systemctl >/dev/null 2>&1 || die "systemctl is required"
}

service_is_active() {
  systemctl is-active --quiet "$SERVICE"
}

install_build_dependencies() {
  [ -n "$TEST_ROOT" ] && return
  export DEBIAN_FRONTEND=noninteractive
  apt-get update
  apt-get install -y --no-install-recommends \
    build-essential \
    ca-certificates \
    curl \
    libglib2.0-dev \
    meson \
    ninja-build \
    pkg-config \
    python3
}

write_wrapper() {
  payload="$1"
  mkdir -p "$payload/bin"
  cat > "$payload/bin/qmicli-wrapper" <<'EOF'
#!/bin/sh
set -eu
wrapper_dir="$(CDPATH= cd -- "$(dirname "$0")" && pwd -P)"
version_dir="$(dirname "$wrapper_dir")"
if [ -n "${LD_LIBRARY_PATH:-}" ]; then
  export LD_LIBRARY_PATH="$version_dir/lib:$LD_LIBRARY_PATH"
else
  export LD_LIBRARY_PATH="$version_dir/lib"
fi
exec "$version_dir/bin/qmicli" "$@"
EOF
  chmod 755 "$payload/bin/qmicli-wrapper"
}

build_payload() {
  install_build_dependencies
  BUILD_WORK_DIR="$(mktemp -d "/tmp/sms-relayed-libqmi.XXXXXX")"
  stage="$BUILD_WORK_DIR/stage"

  if [ -n "$TEST_ROOT" ]; then
    payload="$stage$LOGICAL_VERSION_DIR"
    mkdir -p "$payload/bin"
    cp "$TEST_QMICLI" "$payload/bin/qmicli"
    chmod 755 "$payload/bin/qmicli"
  else
    archive="$BUILD_WORK_DIR/libqmi-$LIBQMI_VERSION.tar.gz"
    curl --connect-timeout 20 --max-time 300 -fL "$LIBQMI_URL" -o "$archive"
    printf '%s  %s\n' "$LIBQMI_SHA256" "$archive" | sha256sum -c -
    tar -xzf "$archive" -C "$BUILD_WORK_DIR"
    source_dir="$BUILD_WORK_DIR/libqmi-$LIBQMI_VERSION"
    build_dir="$BUILD_WORK_DIR/build"
    meson setup "$build_dir" "$source_dir" \
      --prefix="$LOGICAL_VERSION_DIR" \
      --libdir=lib \
      --buildtype=release \
      -Dcollection=full \
      -Dfirmware_update=false \
      -Dmbim_qmux=false \
      -Dmm_runtime_check=false \
      -Dqrtr=false \
      -Drmnet=false \
      -Dudev=false \
      -Dintrospection=false \
      -Dgtk_doc=false \
      -Dman=false \
      -Dbash_completion=false
    # Low-memory SD410 boards can become unresponsive when Ninja uses all cores.
    meson compile -C "$build_dir" -j 1
    DESTDIR="$stage" meson install -C "$build_dir"
    payload="$stage$LOGICAL_VERSION_DIR"
  fi

  write_wrapper "$payload"
  printf '%s\n' "$MARKER_VALUE" > "$payload/$MARKER_NAME"
  chmod 644 "$payload/$MARKER_NAME"
  BUILT_PAYLOAD="$payload"
}

validate_qmicli() {
  qmicli_path="$1"
  [ -x "$qmicli_path" ] || return 1
  help_output="$("$qmicli_path" --help-all)"
  printf '%s\n' "$help_output" | grep -F -- "--ims-get-ims-services-enabled-setting" >/dev/null
  printf '%s\n' "$help_output" | grep -F -- "--imsa-get-ims-registration-status" >/dev/null
  printf '%s\n' "$help_output" | grep -F -- "--imsa-get-ims-services-status" >/dev/null
}

is_managed_version_dir() {
  directory="$1"
  [ -f "$directory/$MARKER_NAME" ] &&
    [ "$(sed -n '1p' "$directory/$MARKER_NAME")" = "$MARKER_VALUE" ]
}

managed_drop_in_content() {
  printf '%s\n' \
    "# Managed by install-private-qmicli-debian.sh" \
    "[Service]" \
    "Environment=SMS_RELAYED_QMICLI_PATH=$LOGICAL_CURRENT/bin/qmicli-wrapper"
}

is_managed_drop_in() {
  [ -f "$DROP_IN" ] &&
    managed_drop_in_content | cmp -s - "$DROP_IN"
}

adopt_legacy_install() {
  if [ -d "$VERSION_DIR" ] &&
    ! is_managed_version_dir "$VERSION_DIR" &&
    [ -L "$CURRENT_LINK" ] &&
    [ "$(readlink "$CURRENT_LINK")" = "libqmi-$LIBQMI_VERSION" ] &&
    is_managed_drop_in &&
    validate_qmicli "$VERSION_DIR/bin/qmicli-wrapper"; then
    printf '%s\n' "$MARKER_VALUE" > "$VERSION_DIR/$MARKER_NAME"
    chmod 644 "$VERSION_DIR/$MARKER_NAME"
    log "adopted private qmicli installed by an earlier helper revision"
  fi
}

preflight_managed_state() {
  adopt_legacy_install
  if [ -e "$VERSION_DIR" ] && ! is_managed_version_dir "$VERSION_DIR"; then
    die "$VERSION_DIR is not managed by this helper; leaving it unchanged"
  fi
  if [ -e "$CURRENT_LINK" ] || [ -L "$CURRENT_LINK" ]; then
    [ -L "$CURRENT_LINK" ] ||
      die "$CURRENT_LINK exists and is not a managed symlink"
    [ "$(readlink "$CURRENT_LINK")" = "libqmi-$LIBQMI_VERSION" ] ||
      die "$CURRENT_LINK points to unmanaged state; leaving it unchanged"
    is_managed_version_dir "$VERSION_DIR" ||
      die "$CURRENT_LINK has no helper-managed version directory"
  fi
  if [ -e "$DROP_IN" ] && ! is_managed_drop_in; then
    die "$DROP_IN is not managed by this helper; leaving it unchanged"
  fi
}

deploy_version() {
  if is_managed_version_dir "$VERSION_DIR" &&
    [ -x "$VERSION_DIR/bin/qmicli-wrapper" ] &&
    validate_qmicli "$VERSION_DIR/bin/qmicli-wrapper"; then
    log "private qmicli $LIBQMI_VERSION is already installed"
    return
  fi
  [ ! -e "$VERSION_DIR" ] ||
    die "$VERSION_DIR already exists but failed validation; remove it explicitly before retrying"
  build_payload
  mkdir -p "$DEST_BASE"
  mv "$BUILT_PAYLOAD" "$VERSION_DIR"
  DEPLOYED_NEW=1
}

replace_current_link() {
  target="$1"
  temporary_link="$DEST_BASE/.libqmi-link.$$"
  rm -f "$temporary_link" || return 1
  ln -s "$target" "$temporary_link" || return 1
  if ! mv -Tf "$temporary_link" "$CURRENT_LINK" 2>/dev/null; then
    rm -f "$CURRENT_LINK" || return 1
    mv -f "$temporary_link" "$CURRENT_LINK" || return 1
  fi
}

write_drop_in() {
  drop_in_dir="$(dirname "$DROP_IN")"
  mkdir -p "$drop_in_dir" || return 1
  temporary_drop_in="$drop_in_dir/.qmicli.conf.$$"
  if ! managed_drop_in_content > "$temporary_drop_in"; then
    return 1
  fi
  chmod 644 "$temporary_drop_in" || return 1
  mv "$temporary_drop_in" "$DROP_IN" || return 1
}

restore_previous_state() {
  old_link="$1"
  old_drop_in="$2"
  if [ -n "$old_link" ]; then
    replace_current_link "$old_link"
  else
    rm -f "$CURRENT_LINK"
  fi
  if [ -n "$old_drop_in" ]; then
    cp "$old_drop_in" "$DROP_IN"
  else
    rm -f "$DROP_IN"
  fi
  systemctl daemon-reload >/dev/null 2>&1 || true
}

remove_new_version_after_failure() {
  if [ "$DEPLOYED_NEW" = "1" ] && is_managed_version_dir "$VERSION_DIR"; then
    rm -rf "$VERSION_DIR"
  fi
}

rollback_install() {
  old_link="$1"
  old_drop_in="$2"
  restore_previous_state "$old_link" "$old_drop_in"
  remove_new_version_after_failure
}

install_private_qmicli() {
  preflight_managed_state
  was_active=0
  service_is_active && was_active=1
  old_link=""
  [ ! -L "$CURRENT_LINK" ] || old_link="$(readlink "$CURRENT_LINK")"
  old_drop_in=""
  if [ -f "$DROP_IN" ]; then
    STATE_DIR="$(mktemp -d "/tmp/sms-relayed-qmicli-state.XXXXXX")"
    old_drop_in="$STATE_DIR/qmicli.conf"
    cp "$DROP_IN" "$old_drop_in"
  fi

  deploy_version
  if ! validate_qmicli "$VERSION_DIR/bin/qmicli-wrapper"; then
    remove_new_version_after_failure
    die "installed qmicli does not expose the required IMS actions"
  fi
  if ! replace_current_link "libqmi-$LIBQMI_VERSION"; then
    rollback_install "$old_link" "$old_drop_in"
    die "failed to activate the private qmicli symlink"
  fi
  if ! write_drop_in; then
    rollback_install "$old_link" "$old_drop_in"
    die "failed to write the systemd qmicli binding"
  fi
  if ! systemctl daemon-reload; then
    rollback_install "$old_link" "$old_drop_in"
    die "systemd daemon-reload failed; restored the previous qmicli binding"
  fi

  if [ "$was_active" = "1" ] && [ "$NO_RESTART" = "0" ]; then
    if ! systemctl restart "$SERVICE"; then
      warn "service restart failed; restoring the previous qmicli binding"
      rollback_install "$old_link" "$old_drop_in"
      systemctl restart "$SERVICE" >/dev/null 2>&1 || true
      return 1
    fi
    log "installed private qmicli $LIBQMI_VERSION and restarted $SERVICE"
  elif [ "$was_active" = "1" ]; then
    log "installed private qmicli $LIBQMI_VERSION; restart skipped"
  else
    log "installed private qmicli $LIBQMI_VERSION; $SERVICE remains inactive"
  fi
}

uninstall_private_qmicli() {
  preflight_managed_state
  was_active=0
  service_is_active && was_active=1
  staged_link="$DEST_BASE/.libqmi-link.uninstall.$$"
  staged_version="$DEST_BASE/.libqmi-$LIBQMI_VERSION.uninstall.$$"
  drop_in_dir="$(dirname "$DROP_IN")"
  staged_drop_in="$drop_in_dir/.qmicli.conf.uninstall.$$"
  [ ! -e "$staged_link" ] && [ ! -L "$staged_link" ] ||
    die "temporary uninstall path already exists: $staged_link"
  [ ! -e "$staged_version" ] ||
    die "temporary uninstall path already exists: $staged_version"
  [ ! -e "$staged_drop_in" ] ||
    die "temporary uninstall path already exists: $staged_drop_in"

  staged_link_present=0
  staged_drop_in_present=0
  staged_version_present=0
  rollback_uninstall() {
    if [ "$staged_version_present" = "1" ] && [ ! -e "$VERSION_DIR" ]; then
      mv "$staged_version" "$VERSION_DIR" || true
    fi
    if [ "$staged_drop_in_present" = "1" ] && [ ! -e "$DROP_IN" ]; then
      mv "$staged_drop_in" "$DROP_IN" || true
    fi
    if [ "$staged_link_present" = "1" ] &&
      [ ! -e "$CURRENT_LINK" ] && [ ! -L "$CURRENT_LINK" ]; then
      mv "$staged_link" "$CURRENT_LINK" || true
    fi
    systemctl daemon-reload >/dev/null 2>&1 || true
    if [ "$was_active" = "1" ] && [ "$NO_RESTART" = "0" ]; then
      systemctl restart "$SERVICE" >/dev/null 2>&1 || true
    fi
  }

  if is_managed_version_dir "$VERSION_DIR" &&
    [ -L "$CURRENT_LINK" ] &&
    [ "$(readlink "$CURRENT_LINK")" = "libqmi-$LIBQMI_VERSION" ]; then
    if ! mv "$CURRENT_LINK" "$staged_link"; then
      die "failed to stage the private qmicli symlink for uninstall"
    fi
    staged_link_present=1
  fi
  if is_managed_drop_in; then
    if ! mv "$DROP_IN" "$staged_drop_in"; then
      rollback_uninstall
      die "failed to stage the systemd qmicli binding for uninstall"
    fi
    staged_drop_in_present=1
  fi
  if is_managed_version_dir "$VERSION_DIR"; then
    if ! mv "$VERSION_DIR" "$staged_version"; then
      rollback_uninstall
      die "failed to stage the private qmicli version for uninstall"
    fi
    staged_version_present=1
  fi
  if ! systemctl daemon-reload; then
    rollback_uninstall
    die "systemd daemon-reload failed; restored the private qmicli binding"
  fi
  if [ "$was_active" = "1" ] && [ "$NO_RESTART" = "0" ] &&
    ! systemctl restart "$SERVICE"; then
    warn "service restart failed; restoring the private qmicli binding"
    rollback_uninstall
    return 1
  fi

  [ "$staged_link_present" = "0" ] || rm -f "$staged_link"
  [ "$staged_drop_in_present" = "0" ] || rm -f "$staged_drop_in"
  if [ "$staged_version_present" = "1" ]; then
    is_managed_version_dir "$staged_version" ||
      die "staged version lost its ownership marker: $staged_version"
    rm -rf "$staged_version"
  fi
  log "removed private qmicli $LIBQMI_VERSION binding"
}

require_supported_host
if [ "$UNINSTALL" = "1" ]; then
  uninstall_private_qmicli
else
  install_private_qmicli
fi
