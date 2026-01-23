#!/usr/bin/env bash
set -e

# rig installer - https://github.com/roelentless/rig

RIG_REPO="${RIG_REPO:-$HOME/.rig/repo}"
REPO_URL="https://github.com/roelentless/rig.git"

info() { echo -e "\033[0;36m[rig]\033[0m $1"; }
success() { echo -e "\033[0;32m[rig]\033[0m $1"; }
error() { echo -e "\033[0;31m[rig]\033[0m $1"; exit 1; }

command -v tmux &>/dev/null || error "tmux not found. Install: brew install tmux (macOS) or apt install tmux (Linux)"
command -v deno &>/dev/null || error "deno not found. Install: curl -fsSL https://deno.land/install.sh | sh"
command -v git &>/dev/null || error "git not found"

info "Installing rig..."
mkdir -p "$(dirname "$RIG_REPO")"

if [ -d "$RIG_REPO" ]; then
  cd "$RIG_REPO"
  git fetch origin
  git reset --hard origin/$(git remote show origin 2>/dev/null | grep 'HEAD branch' | awk '{print $NF}' || echo develop)
else
  git clone "$REPO_URL" "$RIG_REPO"
  cd "$RIG_REPO"
fi

deno install -A -g -n rig --force rig.ts

export PATH="$HOME/.deno/bin:$PATH"
command -v rig &>/dev/null && success "Installed rig $(rig version)" || success "Installed. Add ~/.deno/bin to PATH"
