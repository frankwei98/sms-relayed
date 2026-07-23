#!/bin/sh
set -eu

REPO="frankwei98/sms-relayed"
VERSION="${SMS_RELAYED_VERSION:-latest}"
ROOT="${SMS_RELAYED_ROOT:-}"
BIN_DIR="${SMS_RELAYED_BIN_DIR:-/usr/bin}"
CONFIG_DIR="${SMS_RELAYED_CONFIG_DIR:-/etc/sms-relayed}"
START_SERVICE="${SMS_RELAYED_START:-1}"
CONFIG_ONLY="${SMS_RELAYED_CONFIG_ONLY:-0}"
BINARY_INSTALLED=0
BINARY_UPDATED=0

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

detect_os() {
  os="$(uname -s)"
  case "$os" in
    Linux) printf '%s\n' "linux" ;;
    *) die "unsupported system: $os" ;;
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

verify_download_checksum() {
  binary="$1"
  checksum_file="$2"
  expected_name="$3"

  if ! have sha256sum; then
    printf '%s\n' "error: sha256sum is required to verify the release binary" >&2
    return 1
  fi
  if [ ! -s "$checksum_file" ]; then
    printf '%s\n' "error: release checksum file is empty" >&2
    return 1
  fi

  IFS=' ' read -r expected_checksum checksum_name extra < "$checksum_file" || true
  if [ -z "${expected_checksum:-}" ] || [ -z "${checksum_name:-}" ] || [ -n "${extra:-}" ]; then
    printf '%s\n' "error: release checksum file has an invalid format" >&2
    return 1
  fi
  case "$expected_checksum" in
    *[!0123456789abcdefABCDEF]*)
      printf '%s\n' "error: release checksum is not hexadecimal" >&2
      return 1
      ;;
  esac
  if [ "$(printf '%s' "$expected_checksum" | wc -c | tr -d ' ')" != "64" ]; then
    printf '%s\n' "error: release checksum has an invalid length" >&2
    return 1
  fi
  if [ "$checksum_name" != "$expected_name" ]; then
    printf '%s\n' "error: release checksum does not match $expected_name" >&2
    return 1
  fi

  expected_checksum="$(printf '%s' "$expected_checksum" | tr '[:upper:]' '[:lower:]')"
  actual_checksum="$(sha256sum "$binary" | awk '{print $1}')"
  if [ "$actual_checksum" != "$expected_checksum" ]; then
    printf '%s\n' "error: downloaded binary failed SHA-256 verification" >&2
    return 1
  fi
}

binary_version() {
  bin="$1"
  if [ ! -x "$bin" ]; then
    printf '%s\n' "not installed"
    return
  fi
  "$bin" -V 2>/dev/null || printf '%s\n' "unknown"
}

confirm_update() {
  installed_version="$1"
  release_version="$2"

  log "installed version: $installed_version"
  log "release version: $release_version"

  case "${SMS_RELAYED_UPDATE:-}" in
    1|yes|YES|true|TRUE)
      log "SMS_RELAYED_UPDATE=${SMS_RELAYED_UPDATE}; updating existing binary"
      return 0
      ;;
    0|no|NO|false|FALSE)
      log "SMS_RELAYED_UPDATE=${SMS_RELAYED_UPDATE}; skipping update"
      return 1
      ;;
  esac

  if [ -t 0 ] && [ -t 1 ]; then
    printf '%s' "Update sms-relayed? [y/N] "
    read -r answer
  elif [ -c /dev/tty ] && : 2>/dev/null </dev/tty >/dev/tty; then
    printf '%s' "Update sms-relayed? [y/N] " >/dev/tty
    read -r answer </dev/tty
  else
    warn "no interactive tty; updating existing binary. Set SMS_RELAYED_UPDATE=0 to skip."
    return 0
  fi

  case "$answer" in
    y|Y|yes|YES) return 0 ;;
    *) return 1 ;;
  esac
}

resolve_asset_url_from_json() {
  suffix="$1"
  tr ',' '\n' |
    grep -o '"browser_download_url"[[:space:]]*:[[:space:]]*"[^"]*"' |
    sed 's/.*"browser_download_url"[[:space:]]*:[[:space:]]*"//; s/"$//' |
    grep "$suffix$" |
    head -n 1
}

resolve_tag_from_json() {
  grep -o '"tag_name"[[:space:]]*:[[:space:]]*"[^"]*"' |
    head -n 1 |
    sed 's/.*"tag_name"[[:space:]]*:[[:space:]]*"//; s/"$//'
}

resolve_asset_urls() {
  suffix="$1"
  if [ "$VERSION" = "latest" ]; then
    api="https://api.github.com/repos/$REPO/releases/latest"
  else
    api="https://api.github.com/repos/$REPO/releases/tags/$VERSION"
  fi
  release_json="$(fetch_url "$api")"
  release_tag="$(printf '%s\n' "$release_json" | resolve_tag_from_json)"
  asset_url="$(printf '%s\n' "$release_json" | resolve_asset_url_from_json "$suffix")"
  checksum_url="$(printf '%s\n' "$release_json" | resolve_asset_url_from_json "$suffix.sha256")"
  printf '%s\n%s\n%s\n' "$release_tag" "$asset_url" "$checksum_url"
}

install_binary() {
  system="$(detect_os)"
  arch="$(uname -m)"
  suffix="$(detect_suffix)"
  release="$(resolve_asset_urls "$suffix")"
  release_tag="$(printf '%s\n' "$release" | sed -n '1p')"
  url="$(printf '%s\n' "$release" | sed -n '2p')"
  checksum_url="$(printf '%s\n' "$release" | sed -n '3p')"
  [ -n "$release_tag" ] || die "no release tag found for version $VERSION"
  [ -n "$url" ] || die "no release asset found for $suffix in version $VERSION"
  [ -n "$checksum_url" ] || die "no release checksum found for $suffix in version $VERSION"

  log "detected system: $system"
  log "detected architecture: $arch ($suffix)"
  log "github release tag: $release_tag"
  log "downloading asset: $url"

  real_bin_dir="$(target_path "$BIN_DIR")"
  mkdir -p "$real_bin_dir"
  real_bin="$real_bin_dir/sms-relayed"
  download_tmp="${TMPDIR:-/tmp}/sms-relayed.$$"
  checksum_tmp="${download_tmp}.sha256"
  download_file "$url" "$download_tmp"
  download_file "$checksum_url" "$checksum_tmp"
  if ! verify_download_checksum "$download_tmp" "$checksum_tmp" "sms-relayed-${release_tag}-${suffix}"; then
    rm -f "$download_tmp" "$checksum_tmp"
    die "release binary verification failed"
  fi
  rm -f "$checksum_tmp"
  chmod +x "$download_tmp"

  if [ -e "$real_bin" ]; then
    installed_version="$(binary_version "$real_bin")"
    release_version="$(binary_version "$download_tmp")"
    if ! confirm_update "$installed_version" "$release_version"; then
      rm -f "$download_tmp"
      log "skipped update for $real_bin"
      return
    fi
    BINARY_UPDATED=1
  fi

  mv "$download_tmp" "$real_bin"
  BINARY_INSTALLED=1
  log "installed $real_bin"
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
  have mmcli || warn "mmcli not found; SMS relay can still use ModemManager D-Bus, but Web modem status/control and /api/health modem diagnostics will report unknown"
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
      if [ "$BINARY_UPDATED" = "1" ]; then
        /etc/init.d/sms-relayed restart
      else
        /etc/init.d/sms-relayed start
      fi
    else
      log "SMS_RELAYED_ROOT is set; service file generated but not started"
    fi
  elif have systemctl && [ -z "$ROOT" ]; then
    if [ "$BINARY_UPDATED" = "1" ]; then
      systemctl enable sms-relayed
      systemctl restart sms-relayed
    else
      systemctl enable --now sms-relayed
    fi
  fi
}

main() {
  warn_environment
  mkdir -p "$(target_path "$CONFIG_DIR")"
  chmod 700 "$(target_path "$CONFIG_DIR")" 2>/dev/null || true

  if [ "$CONFIG_ONLY" != "1" ]; then
    install_binary
    if [ "$BINARY_INSTALLED" = "1" ]; then
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
  fi

  if [ "$BINARY_INSTALLED" = "1" ] || [ "$CONFIG_ONLY" = "1" ]; then
    run_setup_if_tty
    start_service_if_ready
  fi
}

if [ "${SMS_RELAYED_TEST:-0}" != "1" ]; then
  main "$@"
fi
