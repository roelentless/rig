# Changelog

## Unreleased

### Added
- `rig run` with no arguments now lists all available tasks (same as `rig run -l`)
- Short task names: `rig run deploy` resolves automatically when the task name is unambiguous across all groups
  - Ambiguous names fail with a clear error listing all matches
