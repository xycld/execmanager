#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"

write_release_checksum() {
  local artifact_path="$1"

  python3 - "$artifact_path" <<'PY'
import hashlib
import pathlib
import sys

artifact_path = pathlib.Path(sys.argv[1])
checksum_path = artifact_path.with_name(f"{artifact_path.name}.sha256")
digest = hashlib.sha256(artifact_path.read_bytes()).hexdigest()
checksum_path.write_text(f"{digest}  {artifact_path.name}\n", encoding="utf-8")
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
write_release_checksum "$temp_dir/execmanager-linux-x86_64"

install_output="$(cd "$repo_root" && PATH='/usr/bin' INSTALL_OS=linux INSTALL_ARCH=x86_64 INSTALL_DIR="$temp_dir/bin" INSTALL_BASE_URL="file://$temp_dir" bash ./install.sh)"
test -x "$temp_dir/bin/execmanager"
test ! -e "$temp_dir/bin/.execmanager-install-channel"
printf '%s\n' "$install_output" | grep -Fx "installed execmanager to $temp_dir/bin/execmanager"
printf '%s\n' "$install_output" | grep -Fx "Add $temp_dir/bin to your PATH, then run:"
printf '%s\n' "$install_output" | grep -Fx '  execmanager'

"$temp_dir/bin/execmanager" | grep -Fx 'execmanager'

printf '#!/usr/bin/env bash\necho old-execmanager\n' > "$temp_dir/existing-execmanager"
chmod +x "$temp_dir/existing-execmanager"
mkdir -p "$temp_dir/bad-release"
mkdir -p "$temp_dir/protected-bin"
cp "$temp_dir/existing-execmanager" "$temp_dir/protected-bin/execmanager"

printf '#!/usr/bin/env bash\necho broken-execmanager\n' > "$temp_dir/bad-release/execmanager-linux-x86_64"
chmod +x "$temp_dir/bad-release/execmanager-linux-x86_64"
printf '0000000000000000000000000000000000000000000000000000000000000000  execmanager-linux-x86_64\n' > "$temp_dir/bad-release/execmanager-linux-x86_64.sha256"

if checksum_failure_output="$(cd "$repo_root" && PATH='/usr/bin' INSTALL_OS=linux INSTALL_ARCH=x86_64 INSTALL_DIR="$temp_dir/protected-bin" INSTALL_BASE_URL="file://$temp_dir/bad-release" bash ./install.sh 2>&1)"; then
  printf 'expected checksum verification failure\n' >&2
  exit 1
fi
printf '%s\n' "$checksum_failure_output" | grep -Fx 'downloaded release checksum verification failed'
"$temp_dir/protected-bin/execmanager" | grep -Fx 'old-execmanager'

snapshot_output="$(cd "$repo_root" && INSTALL_OS=linux INSTALL_ARCH=x86_64 DRY_RUN=1 bash ./install.sh --snapshot)"
printf '%s\n' "$snapshot_output" | grep -Fx 'snapshot asset: execmanager-linux-x86_64-snapshot'
printf '%s\n' "$snapshot_output" | grep -Fx 'download url: https://github.com/xycld/execmanager/releases/download/snapshot/execmanager-linux-x86_64-snapshot'

printf '#!/usr/bin/env bash\necho execmanager-snapshot\n' > "$temp_dir/execmanager-linux-x86_64-snapshot"
chmod +x "$temp_dir/execmanager-linux-x86_64-snapshot"
write_release_checksum "$temp_dir/execmanager-linux-x86_64-snapshot"

snapshot_install_output="$(cd "$repo_root" && PATH='/usr/bin' INSTALL_OS=linux INSTALL_ARCH=x86_64 INSTALL_DIR="$temp_dir/snapshot-bin" INSTALL_SNAPSHOT_BASE_URL="file://$temp_dir" bash ./install.sh --snapshot)"
test -x "$temp_dir/snapshot-bin/execmanager"
test -f "$temp_dir/snapshot-bin/.execmanager-install-channel"
grep -Fx 'snapshot' "$temp_dir/snapshot-bin/.execmanager-install-channel"
printf '%s\n' "$snapshot_install_output" | grep -Fx "installed execmanager to $temp_dir/snapshot-bin/execmanager"
printf '%s\n' "$snapshot_install_output" | grep -Fx "Add $temp_dir/snapshot-bin to your PATH, then run:"
printf '%s\n' "$snapshot_install_output" | grep -Fx '  execmanager'

"$temp_dir/snapshot-bin/execmanager" | grep -Fx 'execmanager-snapshot'
