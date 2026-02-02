# Dockerfile for running rig tests on Linux
FROM denoland/deno:debian

# Install tmux, procps (for ps, pgrep), and fd-find (for discover)
RUN apt-get update && apt-get install -y \
    tmux \
    procps \
    lsof \
    fd-find \
    && rm -rf /var/lib/apt/lists/* \
    && ln -s /usr/bin/fdfind /usr/bin/fd

WORKDIR /app

# Copy source files (skip lockfile for cross-version compatibility)
COPY rig.ts version.ts deno.json ./
COPY test/ ./test/

# Cache dependencies
RUN deno cache rig.ts test/rig.test.ts

# Run tests
CMD ["deno", "test", "-A", "test/rig.test.ts"]
