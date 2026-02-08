#!/bin/sh
# rig installer
# Usage: curl -fsSL https://raw.githubusercontent.com/roelentless/rig/develop/install.sh | sh
#
# Checks what's missing, shows the install plan, asks before running.
# Pass -y to skip the prompt.

REPO_URL="https://github.com/roelentless/rig"
RIG_PACKAGE="jsr:@roelentless/rig"
WATCHEXEC_VERSION="2.2.1"
DENO_MIN_MAJOR=2
DENO_MIN_MINOR=5

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

# Show a command + description in the install plan
plan() { printf "    ${DIM}$%s ${RESET}%-40s %s\n" "$1" "$2" "$3"; }

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

maybe_sudo() {
  if [ "$(id -u)" -eq 0 ]; then
    "$@"
  elif has sudo; then
    sudo "$@"
  else
    err "Root privileges required. Run as root or install sudo."
    exit 1
  fi
}

source_deno_env() {
  DENO_INSTALL="${DENO_INSTALL:-$HOME/.deno}"
  export DENO_INSTALL
  export PATH="$DENO_INSTALL/bin:$PATH"
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
    x86_64|amd64)  ARCH="x86_64" ;;
    arm64|aarch64) ARCH="aarch64" ;;
    *)
      err "Unsupported architecture: $ARCH"
      err "See: ${REPO_URL}#install"
      exit 1
      ;;
  esac
}

detect_distro() {
  DISTRO=""
  PKG_MANAGER=""

  if [ "$PLATFORM" != "linux" ]; then return; fi

  if [ ! -f /etc/os-release ]; then
    err "Cannot detect Linux distribution (missing /etc/os-release)"
    err "rig is tested on Debian/Ubuntu, Fedora, and Arch Linux."
    err "See: ${REPO_URL}#install"
    exit 1
  fi

  . /etc/os-release

  case "$ID" in
    ubuntu|debian|pop|linuxmint|elementary|kali)
      DISTRO="debian"
      PKG_MANAGER="apt"
      ;;
    fedora|rhel|centos|rocky|alma)
      DISTRO="fedora"
      PKG_MANAGER="dnf"
      ;;
    arch|manjaro|endeavouros)
      DISTRO="arch"
      PKG_MANAGER="pacman"
      ;;
    *)
      err "Unsupported Linux distribution: ${ID:-unknown}"
      err "rig is tested on macOS, Debian/Ubuntu, Fedora, and Arch Linux."
      err "See: ${REPO_URL}#install"
      exit 1
      ;;
  esac
}

# ============================================================================
# Dependency check
# ============================================================================

# Parse deno version string, check >= DENO_MIN_MAJOR.DENO_MIN_MINOR
deno_version_ok() {
  VER=$(deno -v 2>/dev/null | head -1 | sed 's/deno //')
  MAJOR=$(echo "$VER" | cut -d. -f1)
  MINOR=$(echo "$VER" | cut -d. -f2)
  if [ "$MAJOR" -gt "$DENO_MIN_MAJOR" ] 2>/dev/null; then return 0; fi
  if [ "$MAJOR" -eq "$DENO_MIN_MAJOR" ] && [ "$MINOR" -ge "$DENO_MIN_MINOR" ] 2>/dev/null; then return 0; fi
  return 1
}

# Check if deno binary needs sudo to upgrade (not writable by current user)
deno_upgrade_needs_sudo() {
  DENO_PATH=$(command -v deno 2>/dev/null)
  [ -n "$DENO_PATH" ] && [ ! -w "$DENO_PATH" ]
}

check_deps() {
  HAS_DENO=false; DENO_OUTDATED=false
  HAS_TMUX=false; HAS_FD=false; HAS_WATCHEXEC=false

  if has deno; then
    if deno_version_ok; then
      HAS_DENO=true
    else
      DENO_OUTDATED=true
    fi
  fi
  has tmux                      && HAS_TMUX=true
  { has fd || has fdfind; }     && HAS_FD=true
  has watchexec                 && HAS_WATCHEXEC=true
}

show_status() {
  printf "\n  Checking dependencies...\n\n"

  if $HAS_DENO; then
    ok "deno $(deno -v 2>/dev/null | head -1)"
  elif $DENO_OUTDATED; then
    miss "deno $(deno -v 2>/dev/null | head -1)" "(requires ${DENO_MIN_MAJOR}.${DENO_MIN_MINOR}+)"
  else
    miss "deno" "(required) JS/TS runtime"
  fi

  if $HAS_TMUX; then
    ok "$(tmux -V 2>/dev/null || echo tmux)"
  else
    miss "tmux" "(required) terminal multiplexer"
  fi

  if $HAS_FD; then
    ok "fd"
  else
    miss "fd" "(required) fast file finder"
  fi

  if $HAS_WATCHEXEC; then
    ok "watchexec"
  else
    miss "watchexec" "(required) file watcher"
  fi
}

# ============================================================================
# Install plan display
# ============================================================================

# Build the sudo prefix once
sudo_pfx() {
  if [ "$PLATFORM" = "linux" ] && [ "$(id -u)" -ne 0 ]; then
    echo "sudo "
  fi
}

show_plan() {
  S=$(sudo_pfx)
  HAS_PLAN=false

  printf "\n  ${BOLD}Install plan:${RESET}\n\n"

  # --- deno ---
  if $DENO_OUTDATED; then
    HAS_PLAN=true
    if deno_upgrade_needs_sudo; then
      plan " " "sudo deno upgrade" "Upgrade to deno ${DENO_MIN_MAJOR}.${DENO_MIN_MINOR}+"
    else
      plan " " "deno upgrade" "Upgrade to deno ${DENO_MIN_MAJOR}.${DENO_MIN_MINOR}+"
    fi
  elif ! $HAS_DENO; then
    HAS_PLAN=true
    plan " " "curl -fsSL https://deno.land/install.sh | sh" "JS/TS runtime"
  fi

  # --- tmux ---
  if ! $HAS_TMUX; then
    HAS_PLAN=true
    case "$PLATFORM" in
      macos) plan " " "brew install tmux" "Terminal multiplexer" ;;
      linux)
        case "$PKG_MANAGER" in
          apt)    plan " " "${S}apt-get install -y tmux" "Terminal multiplexer" ;;
          dnf)    plan " " "${S}dnf install -y tmux" "Terminal multiplexer" ;;
          pacman) plan " " "${S}pacman -S --noconfirm tmux" "Terminal multiplexer" ;;
        esac ;;
    esac
  fi

  # --- fd ---
  if ! $HAS_FD; then
    HAS_PLAN=true
    case "$PLATFORM" in
      macos) plan " " "brew install fd" "Fast file finder" ;;
      linux)
        case "$PKG_MANAGER" in
          apt)    plan " " "${S}apt-get install -y fd-find" "Fast file finder" ;;
          dnf)    plan " " "${S}dnf install -y fd-find" "Fast file finder" ;;
          pacman) plan " " "${S}pacman -S --noconfirm fd" "Fast file finder" ;;
        esac ;;
    esac
  fi

  # --- watchexec (see github.com/watchexec/watchexec/blob/main/doc/packages.md) ---
  if ! $HAS_WATCHEXEC; then
    HAS_PLAN=true
    case "$PLATFORM" in
      macos) plan " " "brew install watchexec" "File watcher for auto-restart" ;;
      linux)
        case "$PKG_MANAGER" in
          apt)    plan " " "watchexec v${WATCHEXEC_VERSION} .deb from GitHub" "File watcher for auto-restart" ;;
          dnf)    plan " " "watchexec v${WATCHEXEC_VERSION} .rpm from GitHub" "File watcher for auto-restart" ;;
          pacman) plan " " "${S}pacman -S --noconfirm watchexec" "File watcher for auto-restart" ;;
        esac ;;
    esac
  fi

  # --- rig ---
  HAS_PLAN=true
  plan " " "deno install -Agf -n rig ${RIG_PACKAGE}" "rig CLI"

  echo ""

  if [ -n "$S" ]; then
    warn "Some commands require sudo"
    echo ""
  fi
}

# ============================================================================
# Install functions
# ============================================================================

upgrade_deno() {
  info "Upgrading deno..."
  if deno_upgrade_needs_sudo; then
    maybe_sudo deno upgrade
  else
    deno upgrade
  fi

  if deno_version_ok; then
    ok "deno upgraded to $(deno -v 2>/dev/null | head -1 | sed 's/deno //')"
  else
    err "deno upgrade failed"
    exit 1
  fi
}

install_deno() {
  info "Installing deno..."

  # Ensure unzip is available (required by deno installer)
  if ! has unzip; then
    case "$PLATFORM" in
      linux)
        case "$PKG_MANAGER" in
          apt)    maybe_sudo apt-get update -qq && maybe_sudo apt-get install -y -qq unzip ;;
          dnf)    maybe_sudo dnf install -y -q unzip ;;
          pacman) maybe_sudo pacman -S --noconfirm unzip ;;
        esac ;;
    esac
  fi

  # Download and run non-interactively (temp file avoids nested-pipe issues)
  DENO_INSTALLER=$(mktemp)
  curl -fsSL https://deno.land/install.sh -o "$DENO_INSTALLER"
  sh "$DENO_INSTALLER" -y
  rm -f "$DENO_INSTALLER"

  source_deno_env

  if ! has deno; then
    err "deno installation failed"
    exit 1
  fi
  ok "deno installed"
}

install_tmux() {
  info "Installing tmux..."
  case "$PLATFORM" in
    macos) brew install tmux ;;
    linux)
      case "$PKG_MANAGER" in
        apt)    maybe_sudo apt-get update -qq && maybe_sudo apt-get install -y -qq tmux ;;
        dnf)    maybe_sudo dnf install -y -q tmux ;;
        pacman) maybe_sudo pacman -S --noconfirm tmux ;;
      esac ;;
  esac

  if ! has tmux; then
    err "tmux installation failed"
    exit 1
  fi
  ok "tmux installed"
}

install_fd() {
  info "Installing fd..."
  case "$PLATFORM" in
    macos) brew install fd ;;
    linux)
      case "$PKG_MANAGER" in
        apt)
          maybe_sudo apt-get install -y -qq fd-find
          if has fdfind && ! has fd; then
            maybe_sudo ln -sf "$(command -v fdfind)" /usr/local/bin/fd 2>/dev/null || true
          fi
          ;;
        dnf)    maybe_sudo dnf install -y -q fd-find ;;
        pacman) maybe_sudo pacman -S --noconfirm fd ;;
      esac ;;
  esac
  if has fd || has fdfind; then
    ok "fd installed"
  else
    err "fd installation failed"
    exit 1
  fi
}

install_watchexec() {
  info "Installing watchexec..."

  if [ "$ARCH" = "x86_64" ]; then WE_ARCH="x86_64"; else WE_ARCH="aarch64"; fi

  case "$PLATFORM" in
    macos) brew install watchexec ;;
    linux)
      case "$PKG_MANAGER" in
        apt)
          WE_URL="https://github.com/watchexec/watchexec/releases/download/v${WATCHEXEC_VERSION}/watchexec-${WATCHEXEC_VERSION}-${WE_ARCH}-unknown-linux-gnu.deb"
          WE_TMP=$(mktemp)
          curl -fsSL -o "$WE_TMP" "$WE_URL"
          maybe_sudo dpkg -i "$WE_TMP"
          rm -f "$WE_TMP"
          ;;
        dnf)
          WE_URL="https://github.com/watchexec/watchexec/releases/download/v${WATCHEXEC_VERSION}/watchexec-${WATCHEXEC_VERSION}-${WE_ARCH}-unknown-linux-gnu.rpm"
          WE_TMP=$(mktemp)
          curl -fsSL -o "$WE_TMP" "$WE_URL"
          maybe_sudo rpm -i "$WE_TMP"
          rm -f "$WE_TMP"
          ;;
        pacman) maybe_sudo pacman -S --noconfirm watchexec ;;
      esac ;;
  esac

  if has watchexec; then
    ok "watchexec installed"
  else
    err "watchexec installation failed"
    exit 1
  fi
}

install_rig() {
  info "Installing rig..."
  source_deno_env
  deno install -Agf -n rig --reload="https://jsr.io/@roelentless/rig" "$RIG_PACKAGE"

  DENO_BIN="${DENO_INSTALL:-$HOME/.deno}/bin"
  if [ -f "$DENO_BIN/rig" ]; then
    ok "rig installed"
  else
    err "rig installation failed"
    exit 1
  fi
}

# ============================================================================
# PATH verification
# ============================================================================

verify_path() {
  DENO_BIN="${DENO_INSTALL:-$HOME/.deno}/bin"

  path_ok=false
  for profile in \
    "$HOME/.bashrc" "$HOME/.bash_profile" \
    "$HOME/.zshrc" "$HOME/.zshenv" \
    "$HOME/.profile" "$HOME/.deno/env"
  do
    if [ -f "$profile" ] && grep -q "\.deno" "$profile" 2>/dev/null; then
      path_ok=true
      break
    fi
  done

  if $path_ok; then
    ok "PATH includes ${DENO_BIN}"
    return
  fi

  echo ""
  warn "~/.deno/bin may not be in your PATH"
  warn "Add to your shell profile:"
  echo ""
  CURRENT_SHELL=$(basename "${SHELL:-/bin/sh}")
  case "$CURRENT_SHELL" in
    zsh)  printf "    echo 'export PATH=\"\$HOME/.deno/bin:\$PATH\"' >> ~/.zshrc\n" ;;
    bash) printf "    echo 'export PATH=\"\$HOME/.deno/bin:\$PATH\"' >> ~/.bashrc\n" ;;
    fish) printf "    fish_add_path ~/.deno/bin\n" ;;
    *)    printf "    export PATH=\"\$HOME/.deno/bin:\$PATH\"\n" ;;
  esac
}

# ============================================================================
# Main
# ============================================================================

main() {
  AUTO_YES=false
  for arg in "$@"; do
    case "$arg" in
      -y|--yes) AUTO_YES=true ;;
      -h|--help)
        printf "rig installer\n\n"
        printf "Usage: curl -fsSL https://raw.githubusercontent.com/roelentless/rig/develop/install.sh | sh\n\n"
        printf "Options:\n"
        printf "  -y, --yes    Skip confirmation prompt\n"
        printf "  -h, --help   Show this help\n"
        exit 0
        ;;
    esac
  done

  printf "\n  ${BOLD}rig installer${RESET}\n"

  # Platform
  detect_platform
  detect_distro
  printf "\n  Platform: ${PLATFORM}/${ARCH}${DISTRO:+ (${DISTRO})}\n"

  # macOS requires brew for system packages
  if [ "$PLATFORM" = "macos" ] && ! has brew; then
    echo ""
    err "Homebrew is required on macOS for system packages."
    err "Install from: https://brew.sh"
    err "Then re-run this installer."
    exit 1
  fi

  # Check
  check_deps
  show_status

  # All deps present and up to date → just install/upgrade rig
  if $HAS_DENO && ! $DENO_OUTDATED && $HAS_TMUX && $HAS_FD && $HAS_WATCHEXEC; then
    printf "\n  All dependencies satisfied.\n"
    echo ""
    install_rig
    verify_path
    printf "\n"
    ok "Done!"
    printf "\n"
    return
  fi

  # Show plan and ask
  show_plan

  if ! prompt_yn "  Proceed? [Y/n] "; then
    echo ""
    info "Aborted. Run the commands above manually, then:"
    printf "    deno install -Agf -n rig %s\n" "$RIG_PACKAGE"
    echo ""
    exit 0
  fi

  echo ""

  # Execute
  if $DENO_OUTDATED; then upgrade_deno
  elif ! $HAS_DENO; then install_deno
  fi
  source_deno_env

  if ! $HAS_TMUX; then install_tmux; fi
  if ! $HAS_FD; then install_fd; fi
  if ! $HAS_WATCHEXEC; then install_watchexec; fi

  echo ""
  install_rig
  verify_path

  printf "\n"
  ok "Installation complete!"
  printf "\n"
  printf "  To activate in this shell:\n"
  CURRENT_SHELL=$(basename "${SHELL:-/bin/sh}")
  case "$CURRENT_SHELL" in
    zsh)  printf "    source ~/.zshrc\n" ;;
    bash) printf "    source ~/.bashrc\n" ;;
    fish) printf "    source ~/.config/fish/config.fish\n" ;;
    *)    printf "    exec \$SHELL\n" ;;
  esac
  printf "\n"
  printf "  Then get started:\n"
  printf "    rig init      Create a rig.yaml\n"
  printf "    rig --help    Show all commands\n"
  printf "\n"
  printf "  Docs: ${REPO_URL}\n"
  printf "  Skip prompt: curl -fsSL ...install.sh | sh -s -- -y\n"
  printf "\n"
}

main "$@"
