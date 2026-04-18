#!/usr/bin/env bash
set -euo pipefail

repo_owner="xycld"
repo_name="execmanager"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
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

resolve_snapshot_download_url() {
  local artifact_name="$1"
  local api_base
  local runs_url
  local artifacts_url
  local work_dir
  local runs_json
  local artifacts_json
  local run_id
  local download_url

  require_python3

  api_base="${INSTALL_GITHUB_API_BASE_URL:-https://api.github.com/repos/${repo_owner}/${repo_name}}"
  runs_url="${INSTALL_SNAPSHOT_RUNS_URL:-${api_base}/actions/workflows/ci.yml/runs?branch=main&status=success&per_page=1}"

  work_dir="$(mktemp -d "${TMPDIR:-/tmp}/execmanager-snapshot-api.XXXXXX")"
  runs_json="${work_dir}/runs.json"
  artifacts_json="${work_dir}/artifacts.json"

  download_asset "$runs_url" "$runs_json"

  if ! run_id="$(python3 - "$runs_json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding='utf-8') as handle:
    payload = json.load(handle)

workflow_runs = payload.get('workflow_runs') or []
if not workflow_runs:
    sys.exit(1)

run_id = workflow_runs[0].get('id')
if run_id is None:
    sys.exit(1)

print(run_id)
PY
  )" || [ -z "$run_id" ]; then
    rm -rf "$work_dir"
    fail 'failed to resolve latest successful snapshot run'
  fi

  artifacts_url="${INSTALL_SNAPSHOT_ARTIFACTS_URL:-${api_base}/actions/runs/${run_id}/artifacts?per_page=100}"
  download_asset "$artifacts_url" "$artifacts_json"

  if ! download_url="$(python3 - "$artifacts_json" "$artifact_name" <<'PY'
import json
import sys

with open(sys.argv[1], encoding='utf-8') as handle:
    payload = json.load(handle)

target_name = sys.argv[2]
for artifact in payload.get('artifacts') or []:
    if artifact.get('name') == target_name and not artifact.get('expired', False):
        archive_download_url = artifact.get('archive_download_url')
        if archive_download_url:
            print(archive_download_url)
            sys.exit(0)

sys.exit(1)
PY
  )" || [ -z "$download_url" ]; then
    rm -rf "$work_dir"
    fail "failed to resolve snapshot artifact download URL: ${artifact_name}"
  fi

  rm -rf "$work_dir"
  printf '%s\n' "$download_url"
}

extract_snapshot_binary() {
  local archive_path="$1"
  local member_name="$2"
  local destination="$3"
  local status

  require_python3

  set +e
  python3 - "$archive_path" "$member_name" "$destination" <<'PY'
import pathlib
import sys
import zipfile

archive_path = pathlib.Path(sys.argv[1])
member_name = sys.argv[2]
destination = pathlib.Path(sys.argv[3])

with zipfile.ZipFile(archive_path) as archive:
    try:
        info = archive.getinfo(member_name)
    except KeyError:
        sys.exit(2)

    member_path = pathlib.PurePosixPath(info.filename)
    if member_path.is_absolute() or '..' in member_path.parts or member_path.name != member_name:
        sys.exit(3)

    destination.write_bytes(archive.read(info))
PY
  status="$?"
  set -e

  case "$status" in
    0) ;;
    2) fail "snapshot archive does not contain expected artifact: ${member_name}" ;;
    *) fail 'failed to extract snapshot artifact archive' ;;
  esac
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
  local asset_label

  install_mode="$(parse_install_mode "$@")"
  os="$(detect_os)"
  arch="$(detect_arch)"
  install_dir="$(resolve_install_dir)"
  target_path="${install_dir}/execmanager"

  case "$install_mode" in
    release)
      artifact_name="$(resolve_release_artifact_name "$os" "$arch")"
      download_url="${INSTALL_BASE_URL:-https://github.com/${repo_owner}/${repo_name}/releases/latest/download}/${artifact_name}"
      asset_label='release asset'
      ;;
    snapshot)
      artifact_name="$(resolve_snapshot_artifact_name "$os" "$arch")"
      download_url="$(resolve_snapshot_download_url "$artifact_name")"
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
  trap 'rm -rf "$temp_dir"' EXIT INT TERM HUP

  case "$install_mode" in
    release)
      temp_path="${temp_dir}/execmanager"
      download_asset "$download_url" "$temp_path"
      ;;
    snapshot)
      temp_path="${temp_dir}/execmanager"
      download_asset "$download_url" "${temp_dir}/artifact.zip"
      extract_snapshot_binary "${temp_dir}/artifact.zip" "$artifact_name" "$temp_path"
      ;;
  esac

  mv -f "$temp_path" "$target_path"
  chmod +x "$target_path"
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
