#!/bin/sh
# rig installer
# Usage: curl -fsSL https://raw.githubusercontent.com/roelentless/rig/develop/install.sh | sh
#
# Downloads a prebuilt binary from GitHub Releases.
# Checks prerequisites, shows the install plan, asks before running.
# Safe to re-run for upgrades. Pass -y to skip the prompt.

REPO="roelentless/rig"
REPO_URL="https://github.com/${REPO}"

# --- Colors (only when terminal) ---

if [ -t 1 ]; then
  BOLD='\033[1m'
  DIM='\033[2m'
  RED='\033[31m'
  GREEN='\033[32m'
  YELLOW='\033[33m'
  CYAN='\033[36m'
  RESET='\033[0m'
else
  BOLD='' DIM='' RED='' GREEN='' YELLOW='' CYAN='' RESET=''
fi

ok()   { printf "  ${GREEN}✓${RESET}  %s\n" "$1"; }
miss() { printf "  ${RED}✗${RESET}  %-28s %s\n" "$1" "$2"; }
skip() { printf "  ${DIM}-  %-28s %s${RESET}\n" "$1" "$2"; }
info() { printf "  ${CYAN}→${RESET} %s\n" "$1"; }
warn() { printf "  ${YELLOW}!${RESET} %s\n" "$1"; }
err()  { printf "  ${RED}✗${RESET} %s\n" "$1" >&2; }

has() { command -v "$1" >/dev/null 2>&1; }

# curl is required to fetch releases - abort immediately if missing
if ! has curl; then
  printf "  curl is required but not installed.\n" >&2
  exit 1
fi

# Prompt Y/n - reads from /dev/tty when stdin is a pipe
prompt_yn() {
  if [ "${AUTO_YES}" = true ]; then return 0; fi
  if [ -t 0 ]; then
    printf "%s" "$1"
    read -r answer
  elif [ -e /dev/tty ]; then
    printf "%s" "$1"
    read -r answer < /dev/tty
  else
    return 0
  fi
  case "$answer" in
    n|N|no|No) return 1 ;;
    *) return 0 ;;
  esac
}

# ============================================================================
# Platform detection
# ============================================================================

detect_platform() {
  OS=$(uname -s)
  ARCH=$(uname -m)

  case "$OS" in
    Darwin) PLATFORM="macos" ;;
    Linux)  PLATFORM="linux" ;;
    *)
      err "Unsupported OS: $OS"
      err "rig supports macOS and Linux."
      err "See: ${REPO_URL}#install"
      exit 1
      ;;
  esac

  case "$ARCH" in
    x86_64|amd64)  ARCH_LABEL="amd64" ;;
    arm64|aarch64) ARCH_LABEL="arm64" ;;
    *)
      err "Unsupported architecture: $ARCH"
      err "See: ${REPO_URL}#install"
      exit 1
      ;;
  esac
}

# ============================================================================
# Install location
# ============================================================================

install_dir() {
  case "$PLATFORM" in
    linux) echo "$HOME/.local/bin" ;;
    macos) echo "/usr/local/bin" ;;
  esac
}

# ============================================================================
# Version check
# ============================================================================

get_latest_version() {
  curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null \
    | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/'
}

get_installed_version() {
  if has rig; then
    rig version 2>/dev/null | head -1 | sed 's/[^0-9.]*//'
  fi
}

# ============================================================================
# Dependency check
# ============================================================================

check_deps() {
  HAS_TMUX=false; HAS_WATCHEXEC=false; HAS_RIG=false

  has tmux                      && HAS_TMUX=true
  has watchexec                 && HAS_WATCHEXEC=true
  has rig                       && HAS_RIG=true
}

show_status() {
  printf "\n  Checking dependencies...\n\n"

  if $HAS_TMUX; then
    ok "$(tmux -V 2>/dev/null || echo tmux)"
  else
    miss "tmux" "(required) terminal multiplexer"
  fi

  if $HAS_WATCHEXEC; then
    ok "watchexec"
  else
    skip "watchexec" "(optional) file watcher for auto-restart"
  fi
}

# ============================================================================
# watchexec install
# ============================================================================

WE_REPO="watchexec/watchexec"


install_watchexec() {
  # macOS: prefer brew when available
  if [ "$PLATFORM" = "macos" ] && has brew; then
    info "Installing watchexec via Homebrew..."
    if brew install watchexec; then
      ok "watchexec installed via Homebrew"
      return 0
    else
      warn "brew install failed, falling back to webi..."
    fi
  fi

  info "Installing watchexec via webi..."
  if curl -fsSL https://webi.sh/watchexec | sh >/dev/null 2>&1; then
    ok "watchexec installed"
  else
    err "watchexec install failed."
    err "Install manually: https://github.com/${WE_REPO}/releases"
    return 1
  fi
}

maybe_install_watchexec() {
  if $HAS_WATCHEXEC; then return; fi

  echo ""
  if [ "$INSTALL_WATCHEXEC" = true ]; then
    install_watchexec
  elif [ "$AUTO_YES" != true ]; then
    if prompt_yn "  Install watchexec? (optional, enables file watching) [Y/n] "; then
      install_watchexec
    else
      info "Skipped. Install later:"
      if [ "$PLATFORM" = "macos" ] && has brew; then
        info "  brew install watchexec"
      else
        info "  https://github.com/${WE_REPO}/releases"
      fi
    fi
  fi
}

# ============================================================================
# Install
# ============================================================================

install_rig() {
  DEST_DIR=$(install_dir)
  DEST="$DEST_DIR/rig"

  LATEST=$(get_latest_version)
  if [ -z "$LATEST" ]; then
    err "Could not determine latest version from GitHub."
    err "Check your network connection or visit: ${REPO_URL}/releases"
    exit 1
  fi

  # Version check - skip if already up to date
  if $HAS_RIG; then
    CURRENT=$(get_installed_version)
    if [ "$CURRENT" = "$LATEST" ]; then
      printf "\n"
      ok "rig ${CURRENT} is already up to date"
      maybe_install_watchexec
      printf "\n"
      exit 0
    fi
    info "Upgrading rig ${CURRENT} → ${LATEST}"
  else
    info "Installing rig ${LATEST}"
  fi

  TARBALL="rig-${PLATFORM}-${ARCH_LABEL}.tar.gz"
  URL="${REPO_URL}/releases/download/${LATEST}/${TARBALL}"

  info "Downloading ${URL}"
  TMP_DIR=$(mktemp -d)
  TMP_TAR="$TMP_DIR/$TARBALL"

  if ! curl -fsSL -o "$TMP_TAR" "$URL"; then
    rm -rf "$TMP_DIR"
    err "Download failed: ${URL}"
    err "Check if a release exists for your platform: ${REPO_URL}/releases"
    exit 1
  fi

  tar xzf "$TMP_TAR" -C "$TMP_DIR"

  # Ensure destination directory exists
  mkdir -p "$DEST_DIR"

  # Install binary (may need elevated privileges on macOS /usr/local/bin)
  if [ -w "$DEST_DIR" ] || [ -w "$DEST" ] 2>/dev/null; then
    mv "$TMP_DIR/rig" "$DEST"
    chmod +x "$DEST"
  elif has sudo; then
    sudo mv "$TMP_DIR/rig" "$DEST"
    sudo chmod +x "$DEST"
  else
    err "Cannot write to ${DEST_DIR}. Run as root or install sudo."
    rm -rf "$TMP_DIR"
    exit 1
  fi

  rm -rf "$TMP_DIR"
  ok "rig ${LATEST} installed to ${DEST}"
}

# ============================================================================
# PATH verification
# ============================================================================

verify_path() {
  DEST_DIR=$(install_dir)

  # Already in PATH — nothing to do
  case ":$PATH:" in
    *":$DEST_DIR:"*) return ;;
  esac

  PATH_LINE="export PATH=\"${DEST_DIR}:\$PATH\""

  # Add to ~/.profile (POSIX login shell config, sourced by bash/sh/dash)
  add_to_profile "$HOME/.profile" "$PATH_LINE"

  # For zsh users, also add to ~/.zprofile (zsh doesn't source ~/.profile)
  CURRENT_SHELL=$(basename "${SHELL:-/bin/sh}")
  if [ "$CURRENT_SHELL" = "zsh" ]; then
    add_to_profile "$HOME/.zprofile" "$PATH_LINE"
  fi

  echo ""
  warn "Open a new terminal or run: source ~/.profile"
}

add_to_profile() {
  PROFILE_FILE="$1"
  LINE="$2"

  # Already present — skip
  if [ -f "$PROFILE_FILE" ] && grep -qF "$LINE" "$PROFILE_FILE"; then
    ok "PATH entry already in ${PROFILE_FILE}"
    return
  fi

  printf '\n%s\n' "$LINE" >> "$PROFILE_FILE"
  ok "Added PATH entry to ${PROFILE_FILE}"
}

# ============================================================================
# Main
# ============================================================================

main() {
  AUTO_YES=false
  INSTALL_WATCHEXEC=false
  for arg in "$@"; do
    case "$arg" in
      -y|--yes) AUTO_YES=true ;;
      --with-watchexec) INSTALL_WATCHEXEC=true ;;
      -h|--help)
        printf "rig installer\n\n"
        printf "Usage: curl -fsSL https://raw.githubusercontent.com/roelentless/rig/develop/install.sh | sh\n\n"
        printf "Options:\n"
        printf "  -y, --yes          Skip confirmation prompt\n"
        printf "  --with-watchexec   Also install watchexec (file watcher)\n"
        printf "  -h, --help         Show this help\n"
        exit 0
        ;;
    esac
  done

  printf "\n  ${BOLD}rig installer / upgrader${RESET}\n"

  # Platform
  detect_platform
  printf "\n  Platform: ${PLATFORM}/${ARCH_LABEL}\n"

  # Check dependencies
  check_deps
  show_status

  # tmux is required - hard error
  if ! $HAS_TMUX; then
    echo ""
    err "tmux is required but not installed."
    err "Install it first:"
    case "$PLATFORM" in
      macos) err "  brew install tmux" ;;
      linux) err "  sudo apt install tmux  (Debian/Ubuntu)"
             err "  sudo dnf install tmux  (Fedora)"
             err "  sudo pacman -S tmux    (Arch)" ;;
    esac
    exit 1
  fi

  # Show what we'll do
  printf "\n  ${BOLD}Install plan:${RESET}\n\n"
  DEST_DIR=$(install_dir)
  printf "    Download prebuilt binary from GitHub Releases\n"
  printf "    Install to ${DEST_DIR}/rig\n"
  echo ""

  if ! prompt_yn "  Proceed? [Y/n] "; then
    echo ""
    info "Aborted."
    info "Manual download: ${REPO_URL}/releases"
    echo ""
    exit 0
  fi

  echo ""
  install_rig
  verify_path
  maybe_install_watchexec

  printf "\n"
  ok "Done!"
  printf "\n"
  printf "  Get started:\n"
  printf "    rig init      Create a rig.yaml\n"
  printf "    rig --help    Show all commands\n"
  printf "\n"
  printf "  Docs: ${REPO_URL}\n"
  printf "  Skip prompt: curl -fsSL ...install.sh | sh -s -- -y\n"
  printf "\n"
}

main "$@"
