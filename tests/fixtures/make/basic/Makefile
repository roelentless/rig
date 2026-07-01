.PHONY: ok boom streams greet loud quiet

## ok: succeeds cleanly
ok:
	@echo ok-ran

## boom: recipe fails with exit 7 (GNU make surfaces exit 2)
boom:
	@exit 7

## streams: writes to both stdout and stderr
streams:
	@echo out-line; echo err-line 1>&2

## greet: echoes a make variable set via passthrough args
greet:
	@echo hello $(NAME)

## loud: echoed recipe (no @) — make prints the command line
loud:
	echo loud-line

## quiet: silenced recipe (@) — only the output appears
quiet:
	@echo quiet-line
