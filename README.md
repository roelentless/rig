<div align="center">
  <img src="https://raw.githubusercontent.com/roelentless/rig/develop/assets/rig.png" width="600"/>
</div>

# rig

A lightweight compose-like dev workflow tool for services and tasks.

<div align="center">
  <img src="https://raw.githubusercontent.com/roelentless/rig/develop/assets/cli.gif" width="600"/>
</div>

## What it does

**Services** - long-running processes via tmux:
```bash
rig up        # Start all services, stream logs (Ctrl+C stops all)
rig up -d     # Start in background (detached)
rig down      # Stop all services (graceful)
rig ps        # Show status
rig logs -f   # Follow logs
```

**Tasks** - one-off commands:
```bash
rig tasks                       # List all tasks
rig run backend.build           # Run a group-level task
rig run backend.api.test        # Run a service-level task
rig run backend.api.test --watch  # Pass args to a task
```

Services run in tmux sessions - they survive terminal close and can be reattached.
Tasks execute directly - they pass through stdin/stdout and exit codes.

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
groups:
  backend:
    services:
      db:
        command: docker run --rm -p 5432:5432 postgres:16
        working_dir: .
        healthcheck:
          grace_ms: 1000

      api:
        command: deno run -A server.ts
        working_dir: ./backend
        environment:
          PORT: 3000
        depends_on: [db]
        tasks:                           # Service-level tasks
          test:
            command: deno test -A
          build:
            command: deno compile -A -o dist/api server.ts
            description: Compile to binary

    tasks:                               # Group-level tasks
      deploy:
        command: ./scripts/deploy.sh
        working_dir: .
        description: Deploy backend

  frontend:
    services:
      web:
        command: npm run dev
        working_dir: ./frontend
```

See the [example/](example/) folder for a working configuration.

## Commands

```
rig - lightweight dev workflow tool for services and tasks

USAGE:
  rig <command> [options] [services...]

SERVICES:
  init                      Create rig.yaml in current directory
  start/up [services...]    Start processes (foreground, streaming logs)
  start/up -d [services...] Start processes in background (detached)
  stop/down [services...]   Stop processes (graceful)
  kill [services...]        Force kill with SIGKILL
  restart [services...]     Restart processes
  ps/list [-f|--full]       Show status (add -f for mem/cpu/ports)
  top                       Live dashboard with auto-refreshing metrics
  logs/tail [-f] [--prev] [service]  Show logs (--prev for last run)
  config [--raw|--json] [services...] Show config (--raw for YAML, --json for JSON)

TASKS:
  tasks [--group <name>]       List all tasks
  run/task <path> [args...]    Run a task (group.name or group.service.name)

OTHER:
  version                   Show version
  help                      Show this help

OPTIONS:
  -g, --group <name>        Target entire group(s) instead of services
  -v, --verbose             Enable verbose logging for debugging

EXAMPLES:
  rig up                    Start all processes (all groups)
  rig up -d                 Start all in background
  rig start api worker      Start specific services
  rig start -g backend      Start all services in backend group
  rig down                  Stop all processes (graceful)
  rig stop -g backend       Stop all services in backend group
  rig kill                  Force kill all processes
  rig restart -g backend    Restart entire group
  rig ps                    Show status
  rig logs -f               Follow all logs
  rig logs --prev api       Show previous logs for api
  rig tasks                 List all tasks
  rig run backend.deploy    Run a group-level task
  rig run backend.api.build Run a service-level task
  rig run backend.api.test --watch  Pass args to a task
  rig config --json         Show raw JSON config

CONFIG:
  Looks for rig.yaml or rig.yml in current directory.
```

## Config reference

### Services

```yaml
groups:
  backend:                         # Group name (alphanumeric, hyphens, underscores)
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
        tasks:                       # Optional. Service-level tasks
          test:
            command: deno test -A
```

Service names must be unique across all groups.

### Tasks

Tasks can be defined at the group level or service level:

```yaml
groups:
  backend:
    services:
      api:
        # ... service config ...
        tasks:                       # Service-level: inherits service env/working_dir
          test:
            command: deno test -A
          build:
            command: deno compile -A -o dist/api server.ts
            environment:             # Optional: overrides/extends service env
              NODE_ENV: production
            description: Compile API

    tasks:                           # Group-level: standalone
      deploy:
        command: ./scripts/deploy.sh
        working_dir: .               # Required for group tasks
        description: Deploy backend
```

- **Service tasks** inherit `working_dir` and `environment` from their parent service
- **Group tasks** must specify `working_dir` (no parent to inherit from)
- Tasks execute directly (not via tmux) - stdin/stdout pass through
- Exit codes propagate - `rig run backend.build && rig run backend.deploy`

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

## Log files

Rig stores logs in `.rig/logs/{group}/{service}/`:

```
.rig/
  logs/
    backend/
      db/
        current.log    # Current run
        previous.log   # Previous run (rotated on restart)
      api/
        current.log
        previous.log
    frontend/
      web/
        current.log
        previous.log
```

Logs persist after processes stop - useful for debugging crashes. The `--prev` flag shows logs from the last run before the current one.

Rig automatically adds `.rig/` to your `.gitignore`.

## Uninstall

```bash
deno uninstall -g rig
```

## How it works

tmux is the source of truth for services - no state files.

- Start: `tmux new-session -d -s {group}-{name} -c {working_dir} '{command}'`
- Logs: `tmux pipe-pane` streams output to `.rig/logs/` files
- Stop: `tmux kill-session -t {group}-{name}` (sends SIGHUP)
- Kill: `SIGKILL` to process tree, then cleanup tmux session
- Status: `tmux list-sessions` filtered by group prefix

Tasks execute directly via `sh -c` with inherited stdin/stdout/stderr.

## License

AGPL-3.0 - See LICENSE file.

---

**Alpha Software** - This project is under active development. APIs and configuration formats may change between versions.

Note: built to improve my personal workflow during development of [halebase.com](https://halebase.com) — don't take it too seriously.

Generated with some LLM assistance.
