<div align="center">
  <img src="assets/rig.png" width="600"/>
</div>

# rig

A lightweight, tmux-based process manager for compose-like workflows without Docker. Built for speed and simplicity.

<div align="center">
  <img src="assets/cli.gif" width="600"/>
</div>

## What it does

```bash
rig init      # Creates rig.yaml config
rig up        # Start all processes, stream logs (Ctrl+C stops all)
rig up -d     # Start in background (detached)
rig down      # Stop all processes
rig ps        # Show status
rig logs -f   # Follow logs
```

Processes run in tmux sessions - they survive terminal close and can be reattached.

## Install

**1. Install tmux and deno:**

```bash
# macOS
brew install tmux
curl -fsSL https://deno.land/install.sh | sh

# Linux (Debian/Ubuntu)
sudo apt install tmux
curl -fsSL https://deno.land/install.sh | sh
```

**2. Install rig:**

```bash
curl -fsSL https://raw.githubusercontent.com/roelentless/rig/develop/install.sh | bash
```

Make sure `~/.deno/bin` is in your PATH.

## Configuration

Create `rig.yaml` in your project:

```yaml
group: myapp

services:
  api:
    command: deno run -A server.ts
    cwd: ./backend
    env:
      PORT: 3000

  web:
    command: npm run dev
    cwd: ./frontend
```

## Commands

```bash
rig init               # Create rig.yaml template
rig up                 # Start all (foreground, logs streaming)
rig up -d              # Start detached (background)
rig up api worker      # Start specific services
rig down               # Stop all
rig stop api           # Stop specific service
rig restart api        # Restart service
rig ps                 # Quick status
rig ps -f              # Status with memory/cpu/ports
rig top                # Live dashboard (q to exit)
rig logs               # Dump all logs
rig logs -f            # Follow all logs
rig logs api           # Logs for specific service
```

## How it works

tmux is the source of truth - no state files.

- Start: `tmux new-session -d -s {group}-{name} -c {cwd} '{command}'`
- Stop: `tmux kill-session -t {group}-{name}`
- Status: `tmux list-sessions` filtered by group prefix

## Config reference

```yaml
group: myapp                    # Required. Prefix for tmux sessions

services:
  api:
    command: deno run -A app.ts # Required. Command to run
    cwd: ./backend              # Required. Working directory
    env:                        # Optional. Environment variables
      PORT: 3000
      DEBUG: true
    color: cyan                 # Optional. Log color
```

Available colors: cyan, yellow, magenta, green, blue, orange, red, lavender, pink, teal, lime, coral, sky, gold, violet

## Uninstall

```bash
deno uninstall -g rig
rm -rf ~/.rig/repo
```

## License

AGPL-3.0 - See LICENSE file.

---

Built during development of [halebase.com](https://halebase.com). Generated with LLM assistance.
