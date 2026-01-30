<div align="center">
  <img src="https://raw.githubusercontent.com/roelentless/rig/develop/assets/rig.png" width="600"/>
</div>

# rig

A lightweight, tmux-based process manager for compose-like workflows without Docker. Built for speed and simplicity.

<div align="center">
  <img src="https://raw.githubusercontent.com/roelentless/rig/develop/assets/cli.gif" width="600"/>
</div>

## What it does

```bash
rig init      # Creates rig.yaml config
rig up        # Start all processes, stream logs (Ctrl+C stops all)
rig up -d     # Start in background (detached)
rig down      # Stop all processes (graceful)
rig kill      # Force kill with SIGKILL
rig ps        # Show status
rig logs -f   # Follow logs
```

Processes run in tmux sessions - they survive terminal close and can be reattached.

## Install

**Prerequisites:** tmux and deno

```bash
# macOS
brew install tmux deno

# Linux
sudo apt install tmux
curl -fsSL https://deno.land/install.sh | sh
```

**Install or upgrade rig:**

```bash
deno install -Agf -n rig --reload=https://jsr.io/@roelentless/rig jsr:@roelentless/rig
```

Ensure `~/.deno/bin` is in your PATH.

## Configuration

Create `rig.yaml` somewhere:

```yaml
group: myapp

services:
  db:
    command: docker run --rm -p 5432:5432 -v myapp-db:/var/lib/postgresql/data -e POSTGRES_PASSWORD=dev postgres:16
    working_dir: .
    healthcheck:
      grace_ms: 1000          # Wait for postgres to be ready

  api:
    command: deno run -A server.ts
    working_dir: ./backend
    environment:
      PORT: 3000
    depends_on: [db]          # Start after db

  web:
    command: npm run dev
    working_dir: ./frontend
```

See the [example/](example/) folder for a working configuration.

## Commands

```
rig - lightweight, tmux-based process manager

USAGE:
  rig <command> [options] [names...]

COMMANDS:
  init                      Create rig.yaml in current directory
  start/up [names...]       Start processes (foreground, streaming logs)
  start/up -d [names...]    Start processes in background (detached)
  stop/down [names...]      Stop processes (graceful)
  kill [names...]           Force kill with SIGKILL
  restart [names...]        Restart processes
  ps/list [-f|--full]       Show status (add -f for mem/cpu/ports)
  top                       Live dashboard with auto-refreshing metrics
  logs/tail [-f] [name]     Show logs (all or specific process)
  config [--raw|--json] [names...] Show tmux commands (--raw for YAML, --json for JSON)
  version                   Show version

OPTIONS:
  -v, --verbose             Enable verbose logging for debugging

EXAMPLES:
  rig up                    Start all processes
  rig up -d                 Start all in background
  rig start api worker      Start specific processes
  rig down                  Stop all processes (graceful)
  rig kill                  Force kill all processes
  rig kill api              Force kill specific process
  rig restart api           Restart single process
  rig ps                    Show status
  rig logs                  Dump all logs
  rig logs -f               Follow all logs (Ctrl+C to exit)
  rig logs api              Dump api logs
  rig logs -f api           Follow api logs
  rig config                Show all tmux commands
  rig config --raw          Show raw YAML config
  rig config --json         Show raw JSON config
  rig config api            Show command for specific process
  rig config --raw api      Show raw YAML for specific service
  rig config --json api     Show raw JSON for specific service

CONFIG:
  Looks for rig.yaml or rig.yml in current directory.
```

## Config reference

```yaml
group: myapp                    # Required. Prefix for tmux sessions

services:
  api:
    command: deno run -A app.ts  # Required. Command to run
    working_dir: ./backend       # Required. Working directory
    environment:                 # Optional. Environment variables
      PORT: 3000
      DEBUG: true
    env_file: ./api.env          # Optional. Load env from file
    color: cyan                  # Optional. Log color
    depends_on: [db, cache]      # Optional. Start after these services
    healthcheck:                 # Optional. Health check settings
      grace_ms: 500              # Wait before starting dependents
```

### env_file

Load environment variables from external files. Supports string or array format:

```yaml
# Simple form
env_file: ./app.env

# Array form with required flag
env_file:
  - path: ./default.env
    required: true   # default - error if missing
  - path: ./override.env
    required: false  # skip if missing
```

Files are processed in order. Later files override earlier. Inline `environment` values override `env_file` values.

Available colors: cyan, yellow, magenta, green, blue, orange, red, lavender, pink, teal, lime, coral, sky, gold, violet

## Uninstall

```bash
deno uninstall -g rig
```

## How it works

tmux is the source of truth - no state files.

- Start: `tmux new-session -d -s {group}-{name} -c {working_dir} '{command}'`
- Stop: `tmux kill-session -t {group}-{name}` (sends SIGHUP)
- Kill: `SIGKILL` to process tree, then cleanup tmux session
- Status: `tmux list-sessions` filtered by group prefix

## License

AGPL-3.0 - See LICENSE file.

---

Note: built to improve my personal workflow during development of [halebase.com](https://halebase.com) — don’t take it too seriously.

Generated with some LLM assistance.
