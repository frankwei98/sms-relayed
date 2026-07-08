#!/bin/sh
set -eu

REPO="frankwei98/sms-relayed"
VERSION="${SMS_RELAYED_VERSION:-latest}"
ROOT="${SMS_RELAYED_ROOT:-}"
BIN_DIR="${SMS_RELAYED_BIN_DIR:-/usr/bin}"
CONFIG_DIR="${SMS_RELAYED_CONFIG_DIR:-/etc/sms-relayed}"
START_SERVICE="${SMS_RELAYED_START:-1}"
CONFIG_ONLY="${SMS_RELAYED_CONFIG_ONLY:-0}"

target_path() {
  printf '%s%s\n' "$ROOT" "$1"
}

log() { printf '%s\n' "$*"; }
warn() { printf 'warning: %s\n' "$*" >&2; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

detect_suffix() {
  arch="$(uname -m)"
  case "$arch" in
    aarch64|arm64) printf '%s\n' "linux-musl-aarch64" ;;
    x86_64) printf '%s\n' "linux-musl-x64" ;;
    armv7l) printf '%s\n' "linux-musl-armv7l" ;;
    *) die "unsupported architecture: $arch" ;;
  esac
}

fetch_url() {
  url="$1"
  if have curl; then
    curl -fsSL "$url"
  elif have wget; then
    wget -qO- "$url"
  else
    die "curl or wget is required"
  fi
}

download_file() {
  url="$1"
  dest="$2"
  if have curl; then
    curl -fL "$url" -o "$dest"
  elif have wget; then
    wget -qO "$dest" "$url"
  else
    die "curl or wget is required"
  fi
}

resolve_asset_url_from_json() {
  suffix="$1"
  tr ',' '\n' |
    grep -o '"browser_download_url"[[:space:]]*:[[:space:]]*"[^"]*"' |
    sed 's/.*"browser_download_url"[[:space:]]*:[[:space:]]*"//; s/"$//' |
    grep "$suffix" |
    head -n 1
}

resolve_asset_url() {
  suffix="$1"
  if [ "$VERSION" = "latest" ]; then
    api="https://api.github.com/repos/$REPO/releases/latest"
  else
    api="https://api.github.com/repos/$REPO/releases/tags/$VERSION"
  fi
  fetch_url "$api" | resolve_asset_url_from_json "$suffix"
}

install_binary() {
  suffix="$(detect_suffix)"
  url="$(resolve_asset_url "$suffix")"
  [ -n "$url" ] || die "no release asset found for $suffix in version $VERSION"

  real_bin_dir="$(target_path "$BIN_DIR")"
  mkdir -p "$real_bin_dir"
  tmp="${TMPDIR:-/tmp}/sms-relayed.$$"
  download_file "$url" "$tmp"
  chmod +x "$tmp"
  mv "$tmp" "$real_bin_dir/sms-relayed"
  log "installed $real_bin_dir/sms-relayed"
}

write_openwrt_service() {
  init_dir="$(target_path /etc/init.d)"
  mkdir -p "$init_dir"
  cat > "$init_dir/sms-relayed" <<EOF
#!/bin/sh /etc/rc.common
START=99
USE_PROCD=1

start_service() {
  procd_open_instance
  procd_set_param command "$BIN_DIR/sms-relayed" run --config "$CONFIG_DIR/config.toml"
  procd_set_param respawn
  procd_close_instance
}
EOF
  chmod +x "$init_dir/sms-relayed"
}

write_systemd_service() {
  systemd_dir="$(target_path /etc/systemd/system)"
  mkdir -p "$systemd_dir"
  cat > "$systemd_dir/sms-relayed.service" <<EOF
[Unit]
Description=sms-relayed
After=network-online.target ModemManager.service
Wants=network-online.target

[Service]
Type=simple
ExecStart=$BIN_DIR/sms-relayed run --config $CONFIG_DIR/config.toml
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF
}

warn_environment() {
  have mmcli || warn "mmcli not found; install or enable ModemManager before expecting SMS forwarding to work"
}

run_setup_if_tty() {
  real_bin="$(target_path "$BIN_DIR")/sms-relayed"
  if [ ! -x "$real_bin" ]; then
    warn "binary not installed at $real_bin; skipping setup"
    return
  fi
  if [ -t 0 ] && [ -t 1 ]; then
    "$real_bin" setup --config "$CONFIG_DIR/config.toml"
  elif [ -c /dev/tty ] && : 2>/dev/null </dev/tty >/dev/tty; then
    "$real_bin" setup --config "$CONFIG_DIR/config.toml" </dev/tty >/dev/tty 2>/dev/tty
  else
    log "non-interactive shell detected; run: $BIN_DIR/sms-relayed setup --config $CONFIG_DIR/config.toml"
  fi
}

start_service_if_ready() {
  [ "$START_SERVICE" = "1" ] || return
  [ "$CONFIG_ONLY" != "1" ] || return
  real_config="$(target_path "$CONFIG_DIR")/config.toml"
  if [ ! -f "$real_config" ]; then
    warn "config missing at $real_config; not starting service"
    return
  fi
  if [ -x "$(target_path /etc/init.d/sms-relayed)" ]; then
    if [ -z "$ROOT" ]; then
      /etc/init.d/sms-relayed enable
      /etc/init.d/sms-relayed start
    else
      log "SMS_RELAYED_ROOT is set; service file generated but not started"
    fi
  elif have systemctl && [ -z "$ROOT" ]; then
    systemctl enable --now sms-relayed
  fi
}

main() {
  warn_environment
  mkdir -p "$(target_path "$CONFIG_DIR")"
  chmod 700 "$(target_path "$CONFIG_DIR")" 2>/dev/null || true

  if [ "$CONFIG_ONLY" != "1" ]; then
    install_binary
    if [ -d "$(target_path /etc/init.d)" ] && [ -f "$(target_path /etc/rc.common)" ]; then
      write_openwrt_service
    elif have systemctl || [ -n "$ROOT" ]; then
      write_systemd_service
      if [ -z "$ROOT" ] && have systemctl; then
        systemctl daemon-reload || true
      fi
    else
      warn "no supported service manager detected"
    fi
  fi

  run_setup_if_tty
  start_service_if_ready
}

if [ "${SMS_RELAYED_TEST:-0}" != "1" ]; then
  main "$@"
fi
