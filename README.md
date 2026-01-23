<div align="center">
  <img src="assets/kitt.png" alt="Michael Knight managing services" width="600"/>
</div>

# noc - simple NO-Container service manager

A lightweight service manager using tmux. Think docker-compose without Docker, but with more leather jackets.

## Features

- **Stateless**: tmux is the source of truth, no state files
- **Detachable**: services survive terminal close, re-attach anytime
- **Simple config**: YAML file defines services
- **Process metrics**: memory, CPU, ports via `ps -a` or `top`

## Installation

**Dependencies:**

Requires [Deno](https://docs.deno.com/runtime/getting_started/installation/) and `tmux`:

```bash
# Install Deno
curl -fsSL https://deno.land/install.sh | sh  # macOS/Linux
# or
brew install deno  # macOS

# Install tmux
brew install tmux  # macOS
```

**Install noc:**

```bash
# From local clone
deno task install

# Uninstall
deno task uninstall
```

## Quick Start

```bash
noc init               # Create noc.yaml
noc start              # Start all services (foreground)
noc start -d           # Start detached (background)
noc stop               # Stop all services
```

## Usage

```bash
noc init               # Create noc.yaml in current directory
noc start              # Start all services (foreground, or reconnect if running)
noc start -d           # Start detached (background)
noc start api worker   # Start specific services
noc stop               # Stop all services
noc restart api        # Restart single service
noc ps                 # Quick status check (~30ms)
noc ps -a              # Status with mem/cpu/ports (~800ms)
noc list               # Alias for ps
noc top                # Live dashboard (q to exit)
noc logs               # Dump all logs (alias: tail)
noc logs -f            # Follow all logs (Ctrl+C to exit)
noc logs api           # Dump api logs
noc logs -f api        # Follow api logs
```

## Configuration

Create `noc.yaml` in your project root:

```yaml
group: myapp

services:
  api:
    command: deno run -A server.ts
    cwd: ./backend
    env:
      PORT: "3000"

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
  api:                          # Service name
    command: deno run -A app.ts # Required. Command to run
    cwd: ./backend              # Required. Working directory (relative or absolute)
    env:                        # Optional. Environment variables
      PORT: "3000"
      DATABASE_URL: "postgres://localhost/myapp"
    color: cyan                 # Optional. Log color (cyan, yellow, magenta, green, blue, orange, red, lavender, pink, teal, lime, coral, sky, gold, violet)
```

## License

AGPL-3.0 - See LICENSE file for details.

## About

This tool was developed during the development of [halebase.com](https://halebase.com).

---

Note: This code was generated and iterated on using LLM.
