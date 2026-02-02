# Example: Multi-File Monorepo

This example demonstrates rig's multi-file config resolution feature.

## Structure

```
example-monorepo/
├── rig.yaml              # Root config (imports all others)
├── infra.rig.yaml        # Infrastructure services (*.rig.yaml naming)
├── shared/
│   └── db/
│       └── rig.yaml      # Shared database services
├── backend/
│   └── rig.yaml          # Backend services (imports ../shared/db/rig.yaml)
└── frontend/
    └── rig.yaml          # Frontend services
```

## Features Demonstrated

1. **Imports** - Root config imports sub-configs to build the full service graph
2. **Shared configs** - `backend/rig.yaml` imports `shared/db/rig.yaml` for database services
3. **Deduplication** - When root imports both `shared/db` and `backend`, the shared config is only loaded once
4. **Path expansion** - Each config's paths are relative to its own location
5. **Cross-file dependencies** - Backend services depend on database services from another file
6. ***.rig.yaml naming** - `infra.rig.yaml` demonstrates organizing by concern without subdirectories

## Usage

### Run from monorepo root (all services available)

```bash
cd example-monorepo
rig ps              # Shows all services from all configs
rig up -d           # Starts everything
rig up -d api web   # Starts specific services
```

### Run from backend directory (backend + database only)

```bash
cd example-monorepo/backend
rig ps              # Shows backend + database services (via import)
rig up -d           # Starts backend and database services
```

### Discover new configs

```bash
cd example-monorepo
rig discover        # Scans for rig files and suggests imports
rig discover --yes  # Auto-accept new imports
```

## Groups

- `database` - PostgreSQL and Redis (from shared/db)
- `backend` - API and worker services
- `frontend` - Web and Storybook
- `infra` - Mailhog and LocalStack
- `root` - Monorepo-wide tasks
