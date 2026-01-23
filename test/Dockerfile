# Dockerfile for running rig tests on Linux
FROM denoland/deno:debian

# Install tmux and procps (for ps, pgrep)
RUN apt-get update && apt-get install -y \
    tmux \
    procps \
    lsof \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy source files (skip lockfile for cross-version compatibility)
COPY rig.ts deno.json ./
COPY test/ ./test/

# Cache dependencies
RUN deno cache rig.ts test/linux.test.ts

# Run Linux tests
CMD ["deno", "test", "-A", "test/linux.test.ts"]
