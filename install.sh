#!/bin/sh
# plaud installer — macOS only.
#
#   curl -fsSL https://raw.githubusercontent.com/leegonzales/plaud-cli/main/install.sh | sh
#
# Downloads the prebuilt binary for your Mac from the latest GitHub release and
# installs it to ~/.local/bin. Falls back to `cargo install` from source if no
# prebuilt binary is available. Override the install dir with PLAUD_INSTALL_DIR.
set -eu

REPO="leegonzales/plaud-cli"
BIN="plaud"
INSTALL_DIR="${PLAUD_INSTALL_DIR:-$HOME/.local/bin}"

say()  { printf '%s\n' "$*"; }
err()  { printf 'error: %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

# --- platform check: macOS only -------------------------------------------
os="$(uname -s)"
[ "$os" = "Darwin" ] || err "plaud is only supported on macOS (detected: $os).
For Linux you can try building from source: cargo install --git https://github.com/$REPO"

case "$(uname -m)" in
  arm64|aarch64) target="aarch64-apple-darwin" ;;
  x86_64)        target="x86_64-apple-darwin" ;;
  *)             err "unsupported macOS architecture: $(uname -m)" ;;
esac

have curl || err "curl is required"

# --- try the prebuilt binary ----------------------------------------------
asset="${BIN}-${target}.tar.gz"
url="https://github.com/${REPO}/releases/latest/download/${asset}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

install_prebuilt() {
  say "Downloading ${BIN} (${target})..."
  if ! curl -fsSL "$url" -o "$tmp/$asset"; then
    return 1
  fi
  tar -xzf "$tmp/$asset" -C "$tmp" || return 1
  [ -f "$tmp/$BIN" ] || return 1
  mkdir -p "$INSTALL_DIR"
  install -m 0755 "$tmp/$BIN" "$INSTALL_DIR/$BIN"
  return 0
}

# --- fall back to building from source -------------------------------------
install_from_source() {
  have cargo || err "no prebuilt binary available and cargo is not installed.
Install Rust from https://rustup.rs and re-run, or download a release manually:
https://github.com/${REPO}/releases/latest"
  say "No prebuilt binary found — building from source with cargo..."
  cargo install --git "https://github.com/${REPO}" --root "${INSTALL_DIR%/bin}" "$BIN"
}

if install_prebuilt; then
  say "Installed ${BIN} -> ${INSTALL_DIR}/${BIN}"
else
  install_from_source
fi

# --- PATH guidance ---------------------------------------------------------
say ""
"$INSTALL_DIR/$BIN" --version 2>/dev/null || true
case ":$PATH:" in
  *":$INSTALL_DIR:"*) : ;;
  *)
    say ""
    say "Note: $INSTALL_DIR is not on your PATH. Add it:"
    say "  echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.zshrc && exec zsh"
    ;;
esac
say ""
say "Next: run 'plaud login' to sign in to Plaud."
