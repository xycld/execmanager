#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"

create_snapshot_fixture() {
  local fixture_dir="$1"

  python3 - "$fixture_dir" <<'PY'
import json
import pathlib
import sys
import zipfile

fixture_dir = pathlib.Path(sys.argv[1])
archive_path = fixture_dir / "execmanager-linux-x86_64-snapshot.zip"
binary_name = "execmanager-linux-x86_64-snapshot"

with zipfile.ZipFile(archive_path, "w") as archive:
    archive.writestr(binary_name, "#!/usr/bin/env bash\necho execmanager-snapshot\n")

(fixture_dir / "runs.json").write_text(json.dumps({
    "workflow_runs": [
        {
            "id": 4242,
            "head_branch": "main",
            "status": "completed",
            "conclusion": "success"
        }
    ]
}), encoding="utf-8")

(fixture_dir / "artifacts.json").write_text(json.dumps({
    "artifacts": [
        {
            "name": binary_name,
            "expired": False,
            "archive_download_url": archive_path.as_uri()
        }
    ]
}), encoding="utf-8")
PY
}

output="$(cd "$repo_root" && INSTALL_OS=linux INSTALL_ARCH=x86_64 DRY_RUN=1 bash ./install.sh)"
printf '%s\n' "$output" | grep -Fx 'release asset: execmanager-linux-x86_64'

default_home="/tmp/execmanager-home"
output="$(cd "$repo_root" && HOME="$default_home" INSTALL_OS=linux INSTALL_ARCH=x86_64 DRY_RUN=1 bash ./install.sh)"
printf '%s\n' "$output" | grep -Fx "install dir: ${default_home}/.local/bin"
printf '%s\n' "$output" | grep -Fx 'download url: https://github.com/xycld/execmanager/releases/latest/download/execmanager-linux-x86_64'

output="$(cd "$repo_root" && INSTALL_OS=linux INSTALL_ARCH=x86_64 INSTALL_DIR=/tmp/execmanager-bin DRY_RUN=1 bash ./install.sh)"
printf '%s\n' "$output" | grep -Fx 'install dir: /tmp/execmanager-bin'
printf '%s\n' "$output" | grep -Fx 'download url: https://github.com/xycld/execmanager/releases/latest/download/execmanager-linux-x86_64'

if missing_home_output="$(cd "$repo_root" && env -u HOME INSTALL_OS=linux INSTALL_ARCH=x86_64 DRY_RUN=1 bash ./install.sh 2>&1)"; then
  printf 'expected missing HOME failure\n' >&2
  exit 1
fi
printf '%s\n' "$missing_home_output" | grep -Fx 'HOME must be set to resolve the default install directory'

if unsupported_output="$(cd "$repo_root" && INSTALL_OS=windows INSTALL_ARCH=x86_64 DRY_RUN=1 bash ./install.sh 2>&1)"; then
  printf 'expected unsupported operating system failure\n' >&2
  exit 1
fi
printf '%s\n' "$unsupported_output" | grep -Fx 'unsupported operating system'

if unsupported_arch_output="$(cd "$repo_root" && INSTALL_OS=linux INSTALL_ARCH=sparc64 DRY_RUN=1 bash ./install.sh 2>&1)"; then
  printf 'expected unsupported architecture failure\n' >&2
  exit 1
fi
printf '%s\n' "$unsupported_arch_output" | grep -Fx 'unsupported architecture'

if unsupported_combo_output="$(cd "$repo_root" && INSTALL_OS=linux INSTALL_ARCH=aarch64 DRY_RUN=1 bash ./install.sh 2>&1)"; then
  printf 'expected unsupported release artifact combination failure\n' >&2
  exit 1
fi
printf '%s\n' "$unsupported_combo_output" | grep -Fx 'unsupported release artifact combination: linux/aarch64'

temp_dir="$(mktemp -d)"
trap 'rm -rf "$temp_dir"' EXIT

printf '#!/usr/bin/env bash\necho execmanager\n' > "$temp_dir/execmanager-linux-x86_64"
chmod +x "$temp_dir/execmanager-linux-x86_64"

install_output="$(cd "$repo_root" && PATH='/usr/bin' INSTALL_OS=linux INSTALL_ARCH=x86_64 INSTALL_DIR="$temp_dir/bin" INSTALL_BASE_URL="file://$temp_dir" bash ./install.sh)"
test -x "$temp_dir/bin/execmanager"
printf '%s\n' "$install_output" | grep -Fx "installed execmanager to $temp_dir/bin/execmanager"
printf '%s\n' "$install_output" | grep -Fx "Add $temp_dir/bin to your PATH, then run:"
printf '%s\n' "$install_output" | grep -Fx '  execmanager'

"$temp_dir/bin/execmanager" | grep -Fx 'execmanager'

create_snapshot_fixture "$temp_dir"

snapshot_output="$(cd "$repo_root" && INSTALL_OS=linux INSTALL_ARCH=x86_64 DRY_RUN=1 INSTALL_SNAPSHOT_RUNS_URL="file://$temp_dir/runs.json" INSTALL_SNAPSHOT_ARTIFACTS_URL="file://$temp_dir/artifacts.json" bash ./install.sh --snapshot)"
printf '%s\n' "$snapshot_output" | grep -Fx 'snapshot asset: execmanager-linux-x86_64-snapshot'
printf '%s\n' "$snapshot_output" | grep -Fx "download url: file://$temp_dir/execmanager-linux-x86_64-snapshot.zip"

snapshot_install_output="$(cd "$repo_root" && PATH='/usr/bin' INSTALL_OS=linux INSTALL_ARCH=x86_64 INSTALL_DIR="$temp_dir/snapshot-bin" INSTALL_SNAPSHOT_RUNS_URL="file://$temp_dir/runs.json" INSTALL_SNAPSHOT_ARTIFACTS_URL="file://$temp_dir/artifacts.json" bash ./install.sh --snapshot)"
test -x "$temp_dir/snapshot-bin/execmanager"
printf '%s\n' "$snapshot_install_output" | grep -Fx "installed execmanager to $temp_dir/snapshot-bin/execmanager"
printf '%s\n' "$snapshot_install_output" | grep -Fx "Add $temp_dir/snapshot-bin to your PATH, then run:"
printf '%s\n' "$snapshot_install_output" | grep -Fx '  execmanager'

"$temp_dir/snapshot-bin/execmanager" | grep -Fx 'execmanager-snapshot'
