<div align="center">
  <img src="assets/rig.png" width="600"/>
</div>

# rig

A lightweight, tmux-based process manager for compose-like workflows without Docker. No compose compatibility—built for speed and simplicity.

## Features

- **Stateless**: tmux is the source of truth, no state files
- **Detachable**: processes survive terminal close, re-attach anytime
- **Simple config**: YAML file defines processes to run
- **Metrics**: memory, CPU, ports via `ps -a` or `top`

## Installation

**1. Install dependencies:**

```bash
# Deno
curl -fsSL https://deno.land/install.sh | sh
# or: brew install deno

# tmux
brew install tmux  # macOS
# or: apt install tmux  # Linux
```

**2. Clone and install:**

```bash
git clone https://github.com/@roelentless/rig.git
cd rig
deno task install
```

This installs `rig` to `~/.deno/bin/`. Make sure it's in your PATH.

**Uninstall:**

```bash
deno task uninstall
```

## Quick Start

```bash
rig init               # Create rig.yaml
rig up                 # Start all processes (foreground)
rig up -d              # Start detached (background)
rig down               # Stop all processes
```

## Usage

```bash
rig init               # Create rig.yaml in current directory
rig up                 # Start all processes (foreground, or reconnect if running)
rig up -d              # Start detached (background)
rig start api worker   # Start specific processes
rig down               # Stop all processes
rig stop api           # Stop specific process
rig restart api        # Restart single process
rig ps                 # Quick status check (~30ms)
rig ps -f              # Status with mem/cpu/ports (~800ms)
rig list               # Alias for ps
rig top                # Live dashboard (q to exit)
rig logs               # Dump all logs (alias: tail)
rig logs -f            # Follow all logs (Ctrl+C to exit)
rig logs api           # Dump api logs
rig logs -f api        # Follow api logs
```

## Configuration

Create `rig.yaml` in your project root:

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

## How It Works

**tmux as state**: No state files. Query `tmux list-sessions` to know what's running.

- Start: `tmux new-session -d -s {group}-{name} -c {cwd} '{command}'`
- Stop: `tmux kill-session -t {group}-{name}` → SIGHUP → process exits
- Status: `tmux list-sessions` filtered by group prefix

## Config Reference

Full config with all options:

```yaml
group: myapp                    # Required. Prefix for tmux sessions

services:
  api:                          # Process name
    command: deno run -A app.ts # Required. Command to run
    cwd: ./backend              # Required. Working directory (relative or absolute)
    env:                        # Optional. Environment variables (values auto-converted to strings)
      PORT: 3000
      DEBUG: true
      DATABASE_URL: postgres://localhost/myapp
    color: cyan                 # Optional. Log color (cyan, yellow, magenta, green, blue, orange, red, lavender, pink, teal, lime, coral, sky, gold, violet)
```

## License

AGPL-3.0 - See LICENSE file for details.

## About

This tool was developed during the development of [halebase.com](https://halebase.com).

---

**Note:** rig provides a compose-like workflow for processes but does not aim for compatibility with compose files or features.

This code was generated and iterated on using LLM.
