# Agent Guidelines for noc

## Core Philosophy

**Stateless over stateful**: tmux IS the state. Don't add state files. Query reality.

**Idempotent operations**: Every command should be safe to run multiple times. `stop` loops through all services even if none are running. `start` checks if already running.

**Let services be independent**: When one service dies, don't kill others. User decides lifecycle (like docker-compose).

## Key Learnings

### tmux

- Session naming convention `{group}-{service}` enables discovery via prefix filtering
- `remain-on-exit on` preserves crash output for debugging
- `kill-session` sends SIGHUP → process exits → port released
- In raw mode, Ctrl+C is byte 3, not SIGINT - must handle explicitly

### Process Metrics

- `lsof -p PID -i` doesn't filter properly on macOS with `-i` flag - must filter output by PID column
- Process trees need recursive `pgrep -P` to find all children
- Sum RSS across tree for accurate memory (child processes matter)

### Performance

- `noc ps` should be instant (~30ms) - don't collect metrics by default
- `noc ps -s` for full metrics (~800ms acceptable)
- `noc top` uses smart refresh: high CPU services refresh more often

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

2. **Path resolution**: `cwd` in config is relative to config file location, not CWD.

3. **Raw mode stdin**: Intercepts Ctrl+C. Must check for byte 3 explicitly.

4. **lsof on macOS**: `-p` flag doesn't filter with `-i`. Parse output and filter by PID.

## Future Considerations

- Auto-restart: tmux has `respawn-pane` but adds complexity
- Log persistence: `pipe-pane` can tee to files
- Health checks: Custom command per service
- Dependencies: `depends_on` ordering (keep simple for now)
