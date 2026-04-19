#!/usr/bin/env bash
set -euo pipefail

repo_owner="xycld"
repo_name="execmanager"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

cleanup_install_temp_dir() {
  local temp_dir_path="${1:-}"

  if [ -n "$temp_dir_path" ]; then
    rm -rf "$temp_dir_path"
  fi
}

parse_install_mode() {
  if [ "$#" -eq 0 ]; then
    printf 'release\n'
    return
  fi

  if [ "$#" -eq 1 ] && [ "$1" = "--snapshot" ]; then
    printf 'snapshot\n'
    return
  fi

  fail 'usage: install.sh [--snapshot]'
}

detect_os() {
  if [ -n "${INSTALL_OS:-}" ]; then
    case "$INSTALL_OS" in
      linux|macos)
        printf '%s\n' "$INSTALL_OS"
        return
        ;;
      *)
        fail "unsupported operating system"
        ;;
    esac
  fi

  case "$(uname -s)" in
    Linux) printf 'linux\n' ;;
    Darwin) printf 'macos\n' ;;
    *) fail "unsupported operating system" ;;
  esac
}

detect_arch() {
  if [ -n "${INSTALL_ARCH:-}" ]; then
    case "$INSTALL_ARCH" in
      x86_64|amd64) printf 'x86_64\n' ;;
      arm64|aarch64) printf 'aarch64\n' ;;
      *) fail "unsupported architecture" ;;
    esac
    return
  fi

  case "$(uname -m)" in
    x86_64|amd64) printf 'x86_64\n' ;;
    arm64|aarch64) printf 'aarch64\n' ;;
    *) fail "unsupported architecture" ;;
  esac
}

resolve_release_artifact_name() {
  local os="$1"
  local arch="$2"

  case "$os/$arch" in
    linux/x86_64) printf 'execmanager-linux-x86_64\n' ;;
    macos/aarch64) printf 'execmanager-macos\n' ;;
    *) fail "unsupported release artifact combination: ${os}/${arch}" ;;
  esac
}

resolve_snapshot_artifact_name() {
  local os="$1"
  local arch="$2"

  case "$os/$arch" in
    linux/x86_64) printf 'execmanager-linux-x86_64-snapshot\n' ;;
    macos/aarch64) printf 'execmanager-macos-snapshot\n' ;;
    *) fail "unsupported snapshot artifact combination: ${os}/${arch}" ;;
  esac
}

resolve_install_dir() {
  if [ -n "${INSTALL_DIR:-}" ]; then
    printf '%s\n' "$INSTALL_DIR"
    return
  fi

  if [ -z "${HOME:-}" ]; then
    fail 'HOME must be set to resolve the default install directory'
  fi

  printf '%s/.local/bin\n' "$HOME"
}

install_channel_marker_path() {
  local install_dir="$1"
  printf '%s/.execmanager-install-channel\n' "$install_dir"
}

download_asset() {
  local url="$1"
  local destination="$2"

  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$destination"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$destination" "$url"
  else
    fail 'curl or wget is required'
  fi
}

require_python3() {
  if ! command -v python3 >/dev/null 2>&1; then
    fail 'python3 is required for snapshot installs'
  fi
}

require_sha256_tool() {
  if command -v sha256sum >/dev/null 2>&1; then
    return
  fi

  if command -v shasum >/dev/null 2>&1; then
    return
  fi

  fail 'sha256sum or shasum is required for release installs'
}

compute_sha256() {
  local path="$1"

  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$path" | cut -d ' ' -f 1
    return
  fi

  shasum -a 256 "$path" | cut -d ' ' -f 1
}

verify_release_checksum() {
  local artifact_path="$1"
  local checksum_path="$2"
  local artifact_name="$3"
  local expected_checksum
  local actual_checksum
  local checksum_entry_name

  require_sha256_tool

  while IFS=' ' read -r expected_checksum checksum_entry_name _; do
    [ -n "$expected_checksum" ] || continue
    checksum_entry_name="${checksum_entry_name#\*}"

    if [ "$checksum_entry_name" = "$artifact_name" ]; then
      break
    fi

    expected_checksum=""
  done < "$checksum_path"

  [ -n "$expected_checksum" ] || fail 'failed to parse release checksum file'

  actual_checksum="$(compute_sha256 "$artifact_path")"

  if [ "$expected_checksum" != "$actual_checksum" ]; then
    fail 'downloaded release checksum verification failed'
  fi
}

path_contains_dir() {
  local target_dir="$1"
  local path_entry
  local old_ifs="$IFS"

  IFS=':'
  for path_entry in ${PATH:-}; do
    if [ "$path_entry" = "$target_dir" ]; then
      IFS="$old_ifs"
      return 0
    fi
  done
  IFS="$old_ifs"

  return 1
}

main() {
  local install_mode
  local os
  local arch
  local artifact_name
  local install_dir
  local download_url
  local target_path
  local temp_dir
  local temp_path
  local checksum_url
  local checksum_path
  local asset_label
  local install_channel_marker

  install_mode="$(parse_install_mode "$@")"
  os="$(detect_os)"
  arch="$(detect_arch)"
  install_dir="$(resolve_install_dir)"
  target_path="${install_dir}/execmanager"
  install_channel_marker="$(install_channel_marker_path "$install_dir")"

  case "$install_mode" in
    release)
      artifact_name="$(resolve_release_artifact_name "$os" "$arch")"
      download_url="${INSTALL_BASE_URL:-https://github.com/${repo_owner}/${repo_name}/releases/latest/download}/${artifact_name}"
      checksum_url="${download_url}.sha256"
      asset_label='release asset'
      ;;
    snapshot)
      artifact_name="$(resolve_snapshot_artifact_name "$os" "$arch")"
      download_url="${INSTALL_SNAPSHOT_BASE_URL:-https://github.com/${repo_owner}/${repo_name}/releases/download/snapshot}/${artifact_name}"
      checksum_url="${download_url}.sha256"
      asset_label='snapshot asset'
      ;;
    *)
      fail 'unsupported install mode'
      ;;
  esac

  if [ "${DRY_RUN:-0}" = "1" ]; then
    printf '%s: %s\n' "$asset_label" "$artifact_name"
    printf 'install dir: %s\n' "$install_dir"
    printf 'download url: %s\n' "$download_url"
    exit 0
  fi

  mkdir -p "$install_dir"

  temp_dir="$(mktemp -d "${install_dir}/execmanager.tmp.XXXXXX")"
  trap 'cleanup_install_temp_dir "${temp_dir:-}"' EXIT INT TERM HUP

  case "$install_mode" in
    release)
      temp_path="${temp_dir}/execmanager"
      checksum_path="${temp_dir}/execmanager.sha256"
      download_asset "$download_url" "$temp_path"
      [ -s "$temp_path" ] || fail 'downloaded release asset is empty'
      download_asset "$checksum_url" "$checksum_path"
      verify_release_checksum "$temp_path" "$checksum_path" "$artifact_name"
      chmod +x "$temp_path"
      ;;
    snapshot)
      temp_path="${temp_dir}/execmanager"
      checksum_path="${temp_dir}/execmanager.sha256"
      download_asset "$download_url" "$temp_path"
      [ -s "$temp_path" ] || fail 'downloaded snapshot asset is empty'
      download_asset "$checksum_url" "$checksum_path"
      verify_release_checksum "$temp_path" "$checksum_path" "$artifact_name"
      chmod +x "$temp_path"
      ;;
  esac

  mv -f "$temp_path" "$target_path"

  case "$install_mode" in
    release)
      rm -f "$install_channel_marker"
      ;;
    snapshot)
      printf 'snapshot\n' > "$install_channel_marker"
      ;;
  esac

  rm -rf "$temp_dir"
  trap - EXIT INT TERM HUP

  printf 'installed execmanager to %s\n' "$target_path"

  if path_contains_dir "$install_dir"; then
    printf 'Run:\n'
    printf '  execmanager\n'
  else
    printf 'Add %s to your PATH, then run:\n' "$install_dir"
    printf '  execmanager\n'
  fi
}

main "$@"
