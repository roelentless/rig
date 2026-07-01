# Long-lived recipe for the cancellation tests. It first `touch`es $(READY) so the
# test has a race-free "recipe has started" gate (by the time READY exists, rig
# has already installed its signal handlers), then runs an endless multi-statement
# loop carrying a unique $(MARKER) in its argv so the recipe process is findable
# via `pgrep -f`. Both vars are supplied per run: rig run sleeper -- MARKER=.. READY=..
sleeper:
	@sh -c 'touch $(READY); : $(MARKER); while true; do sleep 0.2; done'

# Graceful-delivery recipe: traps TERM/INT, records the catchable signal by
# touching $(GRACEFUL), and exits cleanly — proving rig's cancellation delivered
# a real signal (not SIGKILL) before any hard sweep.
graceful:
	@sh -c 'trap "touch $(GRACEFUL); exit 0" TERM INT; touch $(READY); : $(MARKER); while true; do sleep 0.2; done'

# Stuck recipe: ignores TERM/INT entirely, so only rig's SIGKILL backstop (after
# the grace window) can reclaim it.
stubborn:
	@sh -c 'trap "" TERM INT; touch $(READY); : $(MARKER); while true; do sleep 0.2; done'
