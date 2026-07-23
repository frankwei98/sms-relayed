#!/bin/sh
set -eu

repo_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
test_dir=$(mktemp -d)
trap 'rm -rf "$test_dir"' EXIT HUP INT TERM

binary="$test_dir/sms-relayed-test-binary"
checksum="$test_dir/sms-relayed-test-binary.sha256"
printf '%s\n' 'known-good release binary' > "$binary"
(cd "$test_dir" && sha256sum "sms-relayed-test-binary" > "sms-relayed-test-binary.sha256")

SMS_RELAYED_TEST=1 . "$repo_dir/install.sh"

verify_download_checksum "$binary" "$checksum" "sms-relayed-test-binary"

printf '%s\n' 'tampered release binary' > "$binary"
if verify_download_checksum "$binary" "$checksum" "sms-relayed-test-binary" >/dev/null 2>&1; then
  echo "expected checksum verification to reject a modified binary" >&2
  exit 1
fi

install_root="$test_dir/installer"
install_bin_dir="$install_root/bin"
untrusted_execution="$test_dir/untrusted-execution"
mkdir -p "$install_bin_dir"
printf '%s\n' 'installed binary' > "$install_bin_dir/sms-relayed"
expected_checksum=$(printf '%s' 'expected release binary' | sha256sum | awk '{print $1}')

detect_os() { printf '%s\n' 'linux'; }
detect_suffix() { printf '%s\n' 'linux-musl-x64'; }
resolve_asset_urls() {
  printf '%s\n' 'test-release'
  printf '%s\n' 'https://example.test/sms-relayed'
  printf '%s\n' 'https://example.test/sms-relayed.sha256'
}
download_file() {
  case "$1" in
    https://example.test/sms-relayed)
      printf '%s' 'tampered release binary' > "$2"
      ;;
    https://example.test/sms-relayed.sha256)
      printf '%s  %s\n' "$expected_checksum" 'sms-relayed-test-release-linux-musl-x64' > "$2"
      ;;
  esac
}
binary_version() {
  if [ "$1" != "$install_bin_dir/sms-relayed" ]; then
    : > "$untrusted_execution"
  fi
  printf '%s\n' 'sms-relayed test'
}

BIN_DIR="$install_bin_dir"
TMPDIR="$test_dir"
VERSION='test-release'
if (install_binary) >/dev/null 2>&1; then
  echo "expected a tampered download to abort installation" >&2
  exit 1
fi
if [ -e "$untrusted_execution" ]; then
  echo "installer executed an unverified downloaded binary" >&2
  exit 1
fi

atomic_root="$test_dir/atomic-install"
atomic_bin_dir="$atomic_root/bin"
untrusted_tmp="$test_dir/untrusted-tmp"
staging_destinations="$test_dir/staging-destinations"
fixture_binary="$test_dir/fixture-binary"
mkdir -p "$atomic_bin_dir" "$untrusted_tmp"
printf '%s\n' 'verified release binary' > "$fixture_binary"
fixture_checksum=$(sha256sum "$fixture_binary" | awk '{print $1}')

download_file() {
  printf '%s\n' "$2" >> "$staging_destinations"
  case "$1" in
    https://example.test/sms-relayed)
      cp "$fixture_binary" "$2"
      ;;
    https://example.test/sms-relayed.sha256)
      printf '%s  %s\n' "$fixture_checksum" 'sms-relayed-test-release-linux-musl-x64' > "$2"
      ;;
  esac
}

BIN_DIR="$atomic_bin_dir"
TMPDIR="$untrusted_tmp"
BINARY_INSTALLED=0
if ! install_binary >/dev/null; then
  echo "expected a verified download to install successfully" >&2
  exit 1
fi
binary_stage=$(sed -n '1p' "$staging_destinations")
checksum_stage=$(sed -n '2p' "$staging_destinations")
if [ "$(dirname "$binary_stage")" != "$atomic_bin_dir" ] || [ "$(dirname "$checksum_stage")" != "$atomic_bin_dir" ]; then
  echo "installer must stage downloads beside the destination binary" >&2
  exit 1
fi
if [ -n "$(find "$atomic_bin_dir" -maxdepth 1 -name '.sms-relayed.*' -print -quit)" ]; then
  echo "installer left a staging file behind" >&2
  exit 1
fi
if [ -n "$(find "$untrusted_tmp" -mindepth 1 -print -quit)" ]; then
  echo "installer used an attacker-controlled temporary directory" >&2
  exit 1
fi
cmp "$fixture_binary" "$atomic_bin_dir/sms-relayed"
