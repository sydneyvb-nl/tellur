#!/usr/bin/env bash
set -euo pipefail

REPO="sydneyvb-nl/tellur"
INSTALL_DIR="${TELLUR_INSTALL_DIR:-$HOME/.local/bin}"
VERSION="${TELLUR_VERSION:-latest}"

say() { printf '%s\n' "$*"; }
die() { say "Tellur install failed: $*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "required command '$1' was not found"; }

need curl
need tar

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) asset="tellur-mac-arm64.tar.gz" ;;
  Darwin-x86_64) asset="tellur-mac-x64.tar.gz" ;;
  Linux-x86_64) asset="tellur-linux-x64.tar.gz" ;;
  Linux-aarch64|Linux-arm64) asset="tellur-linux-arm64.tar.gz" ;;
  *) die "no prebuilt binary for $(uname -s) $(uname -m)" ;;
esac

if [[ "$VERSION" == "latest" ]]; then
  base="https://github.com/$REPO/releases/latest/download"
else
  base="https://github.com/$REPO/releases/download/v${VERSION#v}"
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

download() { curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location "$1" --output "$2"; }
verify() {
  local file="$1" sidecar="$2" expected actual
  expected="$(awk '{print $1}' "$sidecar")"
  if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "$file" | awk '{print $1}')"
  else
    actual="$(shasum -a 256 "$file" | awk '{print $1}')"
  fi
  [[ "$actual" == "$expected" ]] || die "checksum mismatch for $(basename "$file")"
}

say "Installing Tellur CLI…"
download "$base/$asset" "$tmp/$asset"
download "$base/$asset.sha256" "$tmp/$asset.sha256"
verify "$tmp/$asset" "$tmp/$asset.sha256"
mkdir -p "$INSTALL_DIR"
tar -xzf "$tmp/$asset" -C "$INSTALL_DIR"
chmod +x "$INSTALL_DIR/tellur"

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    profile="$HOME/.profile"
    [[ "${SHELL:-}" == */zsh ]] && profile="$HOME/.zprofile"
    marker="# Tellur installer PATH"
    if ! grep -Fq "$marker" "$profile" 2>/dev/null; then
      printf '\n%s\nexport PATH="%s:$PATH"\n' "$marker" "$INSTALL_DIR" >>"$profile"
    fi
    export PATH="$INSTALL_DIR:$PATH"
    say "Added $INSTALL_DIR to PATH in $profile."
    ;;
esac

install_vsix() {
  if ! download "$base/tellur-vscode.vsix" "$tmp/tellur-vscode.vsix" 2>/dev/null; then
    say "Editor package is not present in this release; continuing with CLI setup."
    return
  fi
  download "$base/tellur-vscode.vsix.sha256" "$tmp/tellur-vscode.vsix.sha256"
  verify "$tmp/tellur-vscode.vsix" "$tmp/tellur-vscode.vsix.sha256"
  local editor executable installed=0
  for editor in code cursor windsurf; do
    executable="$(command -v "$editor" 2>/dev/null || true)"
    if [[ -z "$executable" && "$(uname -s)" == "Darwin" ]]; then
      case "$editor" in
        code) executable="/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code" ;;
        cursor) executable="/Applications/Cursor.app/Contents/Resources/app/bin/cursor" ;;
        windsurf) executable="/Applications/Windsurf.app/Contents/Resources/app/bin/windsurf" ;;
      esac
      [[ -x "$executable" ]] || executable=""
    fi
    if [[ -n "$executable" ]]; then
      say "Installing Tellur extension in ${editor}..."
      "$executable" --install-extension "$tmp/tellur-vscode.vsix" --force >/dev/null
      installed=1
    fi
  done
  [[ "$installed" == 1 ]] || say "No VS Code-compatible editor CLI detected; setup will still prepare its settings."
}

install_jetbrains() {
  command -v unzip >/dev/null 2>&1 || { say "Skipping JetBrains package: 'unzip' is unavailable."; return; }
  download "$base/tellur-jetbrains.zip" "$tmp/tellur-jetbrains.zip" 2>/dev/null || return
  download "$base/tellur-jetbrains.zip.sha256" "$tmp/tellur-jetbrains.zip.sha256"
  verify "$tmp/tellur-jetbrains.zip" "$tmp/tellur-jetbrains.zip.sha256"
  local roots=() product installed=0
  if [[ "$(uname -s)" == "Darwin" ]]; then
    roots+=("$HOME/Library/Application Support/JetBrains")
  else
    roots+=("${XDG_DATA_HOME:-$HOME/.local/share}/JetBrains")
  fi
  for root in "${roots[@]}"; do
    [[ -d "$root" ]] || continue
    while IFS= read -r -d '' product; do
      mkdir -p "$product/plugins"
      rm -rf "$product/plugins/tellur-jetbrains"
      unzip -q "$tmp/tellur-jetbrains.zip" -d "$product/plugins"
      installed=1
    done < <(find "$root" -mindepth 1 -maxdepth 1 -type d -print0)
  done
  [[ "$installed" == 1 ]] && say "Installed the Tellur plugin for detected JetBrains products." || say "No installed JetBrains product was detected."
}

install_vsix
install_jetbrains

say "Starting Tellur setup…"
if [[ "${TELLUR_NONINTERACTIVE:-0}" != "1" && -r /dev/tty && -w /dev/tty ]]; then
  "$INSTALL_DIR/tellur" setup </dev/tty
else
  "$INSTALL_DIR/tellur" setup --yes
fi

say "Tellur is installed. Run 'tellur setup status' to inspect the result."
