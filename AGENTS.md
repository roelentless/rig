# Agent Guidelines for rig

**Compose Spec Alignment**: Where it makes sense, we align with the [Compose Specification](https://github.com/compose-spec/compose-spec/blob/main/spec.md) for naming and behavior of new features.

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

## Code Structure

```
Types & Interfaces     → Data shapes
Constants              → Colors, config names
Utilities              → log(), buildEnvString()
Process Metrics        → getProcessTree(), getProcessMetrics()
Tmux Check             → checkTmux(), printTmuxInstallGuide()
Config                 → loadConfig(), findConfig()
SessionManager         → Class managing tmux sessions
Commands               → cmdStart(), cmdStop(), cmdPs(), cmdTop(), etc.
CLI                    → main(), printUsage()
```

## Common Pitfalls

1. **Orphan processes**: Old processes from different systems won't be in tmux. Port-based cleanup was removed - tmux handles lifecycle properly now.

2. **Path resolution**: `working_dir` in config is relative to config file location, not CWD.

3. **Raw mode stdin**: Intercepts Ctrl+C. Must check for byte 3 explicitly.

4. **lsof on macOS**: `-p` flag doesn't filter with `-i`. Parse output and filter by PID.

## Testing

Tests are organized by platform in the `test/` folder.

### Quick feedback loop (Mac only)

During development, test quickly with Mac-only tests:

```sh
deno task test:darwin
```

This is sufficient for rapid iteration and validation. Docker tests are slower.

### Validate all platforms after task completion

After completing any task, **always** run both platform tests to ensure cross-platform compatibility:

```sh
deno task test:darwin  # Fast Mac validation
deno task test:linux   # Slower Docker validation
```

The Linux tests run in Docker and take ~15-20s due to container build. Both must pass.

### Test maintenance

When modifying `rig.ts`:
- Update both `darwin.test.ts` and `linux.test.ts` with identical test logic
- Tests should have identical assertions (only platform detection differs)
- Validate both platforms pass before considering the task complete

### Test structure

```
test/
  _helpers.ts       # Shared utilities (rig runner, tmux helpers, config)
  darwin.test.ts    # macOS-specific tests
  linux.test.ts     # Linux-specific tests (run in Docker)
  Dockerfile        # Linux test environment
```

### Run all tests

```sh
deno task test  # Runs current platform only
```

## Future Considerations

- Auto-restart: tmux has `respawn-pane` but adds complexity
- Log persistence: `pipe-pane` can tee to files
- Health checks: Custom command per process
- Dependencies: `depends_on` ordering (keep simple for now)
