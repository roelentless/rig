# noc - simple NO-Container service manager

A lightweight service manager using tmux. Think docker-compose without Docker.

**Goal: easy service management.**

## Features

- **Stateless**: tmux is the source of truth, no state files
- **Detachable**: services survive terminal close, re-attach anytime
- **Simple config**: YAML file defines services
- **Process metrics**: memory, CPU, ports via `ps -s` or `top`
- **Smart refresh**: high-CPU services refresh faster in `top`

## Installation

```bash
# Install globally
deno install -A -g -n noc https://raw.githubusercontent.com/your-repo/noc/main/noc.ts

# Or from local clone
deno task install

# Uninstall
deno task uninstall
```

Requires `tmux`:
```bash
brew install tmux  # macOS
```

## Usage

```bash
noc start              # Start all services (foreground, streaming logs)
noc start -d           # Start detached (background)
noc start api worker   # Start specific services
noc stop               # Stop all services
noc restart api        # Restart single service
noc ps                 # Quick status check (~30ms)
noc ps -s              # Status with mem/cpu/ports (~800ms)
noc top                # Live dashboard (q to exit)
noc logs api -f        # Follow logs
noc attach api         # Attach to tmux session (Ctrl+B, D to detach)
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
      DATABASE_URL: "postgres://localhost/myapp"
    color: cyan

  web:
    command: npm run dev
    cwd: ./frontend
    env:
      API_URL: "http://localhost:3000"
    color: green

  worker:
    command: python worker.py
    cwd: ./worker
    env:
      QUEUE_URL: "redis://localhost:6379"
    color: yellow

  db:
    command: postgres -D data
    cwd: ./database
    color: magenta
```

### Fields

|| Field | Required | Description |
|-------|----------|-------------|
| `group` | yes | Prefix for tmux sessions (e.g., `myapp-api`) |
| `services.*.command` | yes | Command to run |
| `services.*.cwd` | yes | Working directory (relative to config or absolute) |
| `services.*.env` | no | Environment variables |
| `services.*.color` | no | Output color: cyan, yellow, magenta, green, blue, orange, red, lavender, pink, teal |

## Architecture

```
noc.yaml          Config defining services
    ↓
SessionManager    Manages tmux sessions (start/stop/status)
    ↓
tmux sessions     Named {group}-{service}, e.g., myapp-api
```

**Key design decisions:**

1. **tmux as state**: No state file. Query `tmux list-sessions` to know what's running.
2. **Session naming**: `{group}-{service}` makes discovery simple.
3. **Process tree metrics**: `pgrep -P` finds children, `ps` sums memory/CPU.
4. **Smart refresh**: `top` refreshes high-CPU services more frequently.

## Development

```bash
# Run tests
deno task test

# Run locally
deno task dev start
```

## How It Works

### Starting a Service
1. Create tmux session: `tmux new-session -d -s {group}-{name} -c {cwd} '{env} exec {command}'`
2. Set `remain-on-exit on` to preserve crash output
3. Session auto-removes when killed

### Stopping a Service
1. `tmux kill-session -t {group}-{name}`
2. Process receives SIGHUP → exits → port released

### Querying Status
1. `tmux list-sessions -F '#{session_name}:#{pane_pid}:#{pane_dead}:#{pane_dead_status}'`
2. Filter by group prefix
3. `pane_dead=1` means process exited

### Process Metrics
1. Get root PID from tmux
2. Find children: `pgrep -P {pid}` recursively
3. Memory/CPU: `ps -o rss,%cpu -p {pids}`
4. Ports: `lsof -i -P -n`, filter by PID

---

## License

AGPL-3.0 - See LICENSE file for details.

## About

This tool was developed during the development of [halebase.com](https://halebase.com).
