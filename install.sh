#!/bin/sh
# Fluxon installer.
#
#   curl -fsSL https://raw.githubusercontent.com/fluxon-lang/fluxon/master/install.sh | sh
#
# Downloads the latest (or a pinned) release binary for your platform from
# GitHub Releases, verifies its SHA-256 checksum, and installs it to a bin
# directory on your PATH. POSIX sh — no bash-isms — so it runs the same on
# Linux and macOS.
#
# Env knobs:
#   FLUXON_VERSION   pin a version, e.g. v0.1.0 (default: latest release)
#   FLUXON_INSTALL_DIR   install target (default: /usr/local/bin if writable,
#                        else ~/.local/bin)
set -eu

REPO="fluxon-lang/fluxon"
BIN_NAME="fluxon"

# --- pretty output (no color when not a tty) --------------------------------
if [ -t 1 ]; then
  bold=$(printf '\033[1m'); dim=$(printf '\033[2m'); red=$(printf '\033[31m')
  green=$(printf '\033[32m'); reset=$(printf '\033[0m')
else
  bold=''; dim=''; red=''; green=''; reset=''
fi
info()  { printf '%s\n' "${dim}fluxon${reset} $*"; }
ok()    { printf '%s\n' "${green}✓${reset} $*"; }
err()   { printf '%s\n' "${red}error:${reset} $*" >&2; }
die()   { err "$@"; exit 1; }

need() { command -v "$1" >/dev/null 2>&1 || die "'$1' is required but was not found."; }

# We need a downloader and a tar. curl preferred, wget as fallback.
if command -v curl >/dev/null 2>&1; then
  DL="curl -fsSL"
  DL_OUT="curl -fsSL -o"
elif command -v wget >/dev/null 2>&1; then
  DL="wget -qO-"
  DL_OUT="wget -qO"
else
  die "either 'curl' or 'wget' is required."
fi
need tar

# --- detect platform --------------------------------------------------------
os="$(uname -s)"
case "$os" in
  Linux)  os_name="linux" ;;
  Darwin) os_name="macos" ;;
  *) die "unsupported OS '$os'. On Windows use install.ps1 instead." ;;
esac

arch="$(uname -m)"
case "$arch" in
  x86_64|amd64)   arch_name="x86_64" ;;
  arm64|aarch64)  arch_name="aarch64" ;;
  *) die "unsupported architecture '$arch'." ;;
esac

# --- resolve version --------------------------------------------------------
version="${FLUXON_VERSION:-}"
if [ -z "$version" ]; then
  info "resolving the latest release…"
  # The redirect target of /releases/latest ends in /tag/<version> — read it
  # without depending on jq.
  version="$(
    $DL "https://api.github.com/repos/$REPO/releases/latest" \
      | grep '"tag_name"' | head -n1 | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/'
  )"
  [ -n "$version" ] || die "could not determine the latest release. Set FLUXON_VERSION=vX.Y.Z and retry."
fi
# Normalize: tags are published as vX.Y.Z; accept a bare X.Y.Z too.
case "$version" in v*) tag="$version" ;; *) tag="v$version" ;; esac

asset="${BIN_NAME}-${tag}-${os_name}-${arch_name}.tar.gz"
base_url="https://github.com/$REPO/releases/download/$tag"

info "installing ${bold}fluxon ${tag}${reset} (${os_name}/${arch_name})"

# --- download into a temp dir we always clean up ----------------------------
tmp="$(mktemp -d 2>/dev/null || mktemp -d -t fluxon)"
trap 'rm -rf "$tmp"' EXIT INT TERM

info "downloading $asset"
$DL_OUT "$tmp/$asset" "$base_url/$asset" \
  || die "download failed. Is '$tag' published for ${os_name}/${arch_name}? See https://github.com/$REPO/releases"

# --- verify checksum (best-effort: skip cleanly if SHA tools are absent) ----
if $DL_OUT "$tmp/SHA256SUMS.txt" "$base_url/SHA256SUMS.txt" 2>/dev/null; then
  if command -v sha256sum >/dev/null 2>&1; then
    sha_cmd="sha256sum"
  elif command -v shasum >/dev/null 2>&1; then
    sha_cmd="shasum -a 256"
  else
    sha_cmd=""
  fi
  if [ -n "$sha_cmd" ]; then
    expected="$(grep " $asset\$" "$tmp/SHA256SUMS.txt" | awk '{print $1}')"
    if [ -n "$expected" ]; then
      actual="$( (cd "$tmp" && $sha_cmd "$asset") | awk '{print $1}')"
      [ "$expected" = "$actual" ] || die "checksum mismatch for $asset — refusing to install."
      ok "checksum verified"
    fi
  fi
fi

# --- unpack -----------------------------------------------------------------
tar -xzf "$tmp/$asset" -C "$tmp"
[ -f "$tmp/$BIN_NAME" ] || die "archive did not contain the '$BIN_NAME' binary."
chmod +x "$tmp/$BIN_NAME"

# --- choose install dir -----------------------------------------------------
install_dir="${FLUXON_INSTALL_DIR:-}"
if [ -z "$install_dir" ]; then
  if [ -w /usr/local/bin ] 2>/dev/null; then
    install_dir="/usr/local/bin"
  else
    install_dir="$HOME/.local/bin"
  fi
fi
mkdir -p "$install_dir"

# Use sudo for a system dir we cannot write to directly.
if [ -w "$install_dir" ]; then
  mv "$tmp/$BIN_NAME" "$install_dir/$BIN_NAME"
elif command -v sudo >/dev/null 2>&1; then
  info "installing to $install_dir (requires sudo)"
  sudo mv "$tmp/$BIN_NAME" "$install_dir/$BIN_NAME"
else
  die "cannot write to $install_dir and 'sudo' is unavailable. Set FLUXON_INSTALL_DIR to a writable directory."
fi

ok "installed ${bold}$BIN_NAME${reset} to $install_dir/$BIN_NAME"

# --- PATH hint --------------------------------------------------------------
case ":$PATH:" in
  *":$install_dir:"*) ;;
  *)
    printf '\n'
    info "${bold}$install_dir is not on your PATH.${reset} Add it:"
    printf '    %sexport PATH="%s:$PATH"%s\n' "$dim" "$install_dir" "$reset"
    printf '  (put that line in your ~/.bashrc, ~/.zshrc, or ~/.profile)\n'
    ;;
esac

printf '\n'
ok "run ${bold}$BIN_NAME --help${reset} to get started"
