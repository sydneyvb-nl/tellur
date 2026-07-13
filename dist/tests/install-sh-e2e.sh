#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/assets" "$tmp/mock-bin" "$tmp/home" "$tmp/install"

cat >"$tmp/tellur" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >"$TELLUR_TEST_SETUP_LOG"
EOF
chmod +x "$tmp/tellur"
tar -czf "$tmp/assets/tellur-linux-x64.tar.gz" -C "$tmp" tellur
sha256sum "$tmp/assets/tellur-linux-x64.tar.gz" >"$tmp/assets/tellur-linux-x64.tar.gz.sha256"
printf 'vsix' >"$tmp/assets/tellur-vscode.vsix"
sha256sum "$tmp/assets/tellur-vscode.vsix" >"$tmp/assets/tellur-vscode.vsix.sha256"
printf 'jetbrains' >"$tmp/assets/tellur-jetbrains.zip"
sha256sum "$tmp/assets/tellur-jetbrains.zip" >"$tmp/assets/tellur-jetbrains.zip.sha256"

cat >"$tmp/mock-bin/uname" <<'EOF'
#!/usr/bin/env bash
[[ "${1:-}" == "-s" ]] && printf 'Linux\n' || printf 'x86_64\n'
EOF
cat >"$tmp/mock-bin/curl" <<'EOF'
#!/usr/bin/env bash
url="" output=""
while (($#)); do
  case "$1" in
    http*) url="$1"; shift ;;
    --output) output="$2"; shift 2 ;;
    *) shift ;;
  esac
done
cp "$TELLUR_TEST_ASSETS/${url##*/}" "$output"
EOF
cat >"$tmp/mock-bin/code" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >"$TELLUR_TEST_EDITOR_LOG"
EOF
chmod +x "$tmp/mock-bin/"*

PATH="$tmp/mock-bin:$PATH" \
HOME="$tmp/home" \
TELLUR_INSTALL_DIR="$tmp/install" \
TELLUR_TEST_ASSETS="$tmp/assets" \
TELLUR_TEST_SETUP_LOG="$tmp/setup.log" \
TELLUR_TEST_EDITOR_LOG="$tmp/editor.log" \
TELLUR_NONINTERACTIVE=1 \
bash "$root/install.sh"

test -x "$tmp/install/tellur"
grep -Fx 'setup --yes' "$tmp/setup.log"
grep -F -- '--install-extension' "$tmp/editor.log"

printf '%064d  tellur-linux-x64.tar.gz\n' 0 >"$tmp/assets/tellur-linux-x64.tar.gz.sha256"
if PATH="$tmp/mock-bin:$PATH" \
  HOME="$tmp/home" \
  TELLUR_INSTALL_DIR="$tmp/install" \
  TELLUR_TEST_ASSETS="$tmp/assets" \
  TELLUR_NONINTERACTIVE=1 \
  bash "$root/install.sh" >"$tmp/tamper.log" 2>&1; then
  echo "installer accepted a bad checksum" >&2
  exit 1
fi
grep -F 'checksum mismatch' "$tmp/tamper.log"
