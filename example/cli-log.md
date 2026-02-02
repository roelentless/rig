# rig CLI Examples

Real command output from this example folder.

## Basic Commands

### List all services
```
$ rig ps
GROUP          SERVICE        STATUS       UPTIME
───────────────────────────────────────────────────────
backend        db             stopped      -
backend        api            stopped      -
backend        worker         stopped      -
frontend       web            stopped      -
infra          mail           stopped      -
```

### Start all services
```
$ rig start -d
11:32:08.653 rig          Starting 5 process(es)...
11:32:08.688 rig          Started db (pid 29413)
11:32:08.716 rig          Started web (pid 29425)
11:32:08.742 rig          Started mail (pid 29437)
11:32:09.276 rig          Started api (pid 29489)
11:32:09.306 rig          Started worker (pid 29513)
11:32:09.306 rig          Processes started in background
```

### Check running services
```
$ rig ps
GROUP          SERVICE        STATUS       UPTIME
───────────────────────────────────────────────────────
backend        db             running      3s
backend        api            running      2s
backend        worker         running      2s
frontend       web            running      3s
infra          mail           running      3s
```

### Full status with metrics
```
$ rig ps -f
GROUP          SERVICE        STATUS       MEM    CPU  PORTS            UPTIME      PID
───────────────────────────────────────────────────────────────────────────────────────────────
backend        db             running        47M    0%  -                8s         29413
backend        api            running        55M    0%  -                7s         29489
backend        worker         running        47M    0%  -                8s         29513
frontend       web            running        46M    0%  -                9s         29425
infra          mail           running        48M    0%  -                9s         29437
```

### View logs
```
$ rig logs api --lines 5
11:32:16.836 api          API server starting on port 3000...
11:32:16.836 api          Connected to database
11:32:16.836 api          API ready at http://localhost:3000
11:32:16.836 api          POST /orders - 13ms
11:32:16.836 api          GET /products - 24ms
```

### Stop all services
```
$ rig stop
11:32:43.897 rig          Stopping db...
11:32:43.914 rig          Stopping api...
11:32:43.924 rig          Stopping worker...
11:32:43.933 rig          Stopping web...
11:32:43.939 rig          All processes stopped
```

## Targeting Groups

### Start only backend group
```
$ rig start -g backend -d
11:32:48.893 rig          Starting 3 process(es)...
11:32:48.929 rig          Started db (pid 31416)
11:32:49.483 rig          Started api (pid 31429)
11:32:49.519 rig          Started worker (pid 31455)
11:32:49.519 rig          Processes started in background

$ rig ps
GROUP          SERVICE        STATUS       UPTIME
───────────────────────────────────────────────────────
backend        db             running      2s
backend        api            running      1s
backend        worker         running      1s
frontend       web            stopped      -
infra          mail           stopped      -
```

## Running Tasks

### Group-level task
```
$ rig run backend.build
Check api.ts
Compile api.ts to dist/api
...
```

### Service-level task
```
$ rig run backend.api.check
Check file:///Users/roel/projects/rig/example/api.ts
```

## Show Config

```
$ rig config
backend      db           deno run -A db.ts working_dir=. grace_ms=500
backend      api          deno run -A api.ts working_dir=. PORT=3000 depends_on=[db]
backend      worker       deno run -A worker.ts working_dir=. depends_on=[db]
frontend     web          deno run -A web.ts working_dir=.
infra        mail         deno run -A mail.ts working_dir=infra/. grace_ms=300
```

## Multi-File Config: Running from Subdirectory

When running from `infra/`, only that config is loaded:

```
$ cd infra
$ rig ps
GROUP          SERVICE        STATUS       UPTIME
───────────────────────────────────────────────────────
infra          mail           stopped      -
```
