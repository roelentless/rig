# Agent Guidelines for rig

**Compose Spec Alignment**: Where it makes sense, we align with the [Compose Specification](https://github.com/compose-spec/compose-spec/blob/main/spec.md) for naming and behavior of new features.

## Off-Limits Directories

- `rfc/` - Contains work tasks and planning documents. Do not read unless specifically asked.

## Core Philosophy

**Stateless over stateful**: tmux IS the state. Don't add state files. Query reality.

**Idempotent operations**: Every command should be safe to run multiple times. `stop` loops through all processes even if none are running. `start` checks if already running.

**Let processes be independent**: When one process dies, don't kill others. User decides lifecycle (like docker-compose).

## Key Learnings

### tmux

- Session naming convention `{group}-{name}` enables discovery via prefix filtering
- `remain-on-exit on` preserves crash output for debugging
- `kill-session` sends SIGHUP → process exits → port released
- In raw mode, Ctrl+C is byte 3, not SIGINT - must handle explicitly

### Process Metrics

- `lsof -p PID -i` doesn't filter properly on macOS with `-i` flag - must filter output by PID column
- Process trees need recursive `pgrep -P` to find all children
- Sum RSS across tree for accurate memory (child processes matter)

### Performance

- `rig ps` should be instant (~30ms) - don't collect metrics by default
- `rig ps -f` for full metrics (~800ms acceptable)
- `rig top` uses smart refresh: high CPU processes refresh more often

### CLI

- Use `@std/cli/parse-args` for flag parsing
- Don't use `stopEarly: true` if flags can appear after the command

## Config Structure

Config uses multi-group format with services nested under groups:

```yaml
groups:
  backend:
    services:
      api: { command: ..., working_dir: ... }
      db:  { command: ..., working_dir: ... }
  frontend:
    services:
      web: { command: ..., working_dir: ... }
```

Key rules:
- Service names must be unique across all groups
- Groups are targeted with `-g/--group` flag: `rig start -g backend`
- Default CLI targets are services: `rig start api db`
- One SessionManager instance per group (tmux sessions: `{group}-{service}`)

## Code Structure

```
Types & Interfaces     → Data shapes (Config, GroupDef, ServiceDef, ResolvedService)
Constants              → Colors, config names
Utilities              → log(), buildEnvString()
Process Metrics        → getProcessTree(), getProcessMetrics()
Tmux Check             → checkTmux(), printTmuxInstallGuide()
Config                 → loadConfig(), buildServiceLookup(), resolveTargets()
SessionManager         → Class managing tmux sessions (one per group)
Commands               → cmdStart(), cmdStop(), cmdPs(), cmdTop(), etc.
CLI                    → main(), printUsage()
```

## Common Pitfalls

1. **Orphan processes**: Old processes from different systems won't be in tmux. Port-based cleanup was removed - tmux handles lifecycle properly now.

2. **Path resolution**: `working_dir` in config is relative to config file location, not CWD.

3. **Raw mode stdin**: Intercepts Ctrl+C. Must check for byte 3 explicitly.

4. **lsof on macOS**: `-p` flag doesn't filter with `-i`. Parse output and filter by PID.

## Local Development

Developers should set up local development as described in [CONTRIBUTING.md](CONTRIBUTING.md). Once set up, you can test changes locally without reinstalling.

The installed `rig` command is a wrapper script that runs:
```sh
exec deno run --allow-all --no-config 'file:///path/to/rig/rig.ts' "$@"
```

## Documentation

**Keep README.md in sync with command output**: When modifying commands or their output, always run `rig -h` and update the Commands section in README.md to match the exact output. The README should reflect what users see when they run the help command.

## Testing

Tests use a single unified test file that runs on both macOS and Linux.

### Quick feedback loop

During development, test locally on your platform:

```sh
deno task test
```

This runs the full test suite on your current platform and is sufficient for rapid iteration.

### Validate all platforms after task completion

After completing any task, **always** validate on Linux via Docker to ensure cross-platform compatibility:

```sh
deno task test         # Fast local validation
deno task test:docker  # Slower Docker validation (~15-20s)
```

Both must pass before considering the task complete.

### Test structure

```
test/
  _helpers.ts     # Shared utilities (rig runner, tmux helpers, config)
  rig.test.ts     # Unified tests (run on both macOS and Linux)
  Dockerfile      # Linux test environment for Docker
  tmp/            # Test artifacts (gitignored) - rig.yaml, .env files, etc.
```

Tests automatically detect the platform via `Deno.build.os` and prefix test names accordingly.

## Future Considerations

- Auto-restart: tmux has `respawn-pane` but adds complexity
- Multi-file config merging (`rig -f file1.yaml -f file2.yaml`)
- Group-level settings (shared env, working_dir defaults)
