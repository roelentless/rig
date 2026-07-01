# $$VAR reaches the recipe shell from the environment (rig group env + ambient).
# $(NAME) is a make variable, set via `rig run svc.greet -- NAME=world`.
greet:
	@echo "ANCESTOR=$$ANCESTOR_VAR GROUP=$$GROUP_VAR AMBIENT=$$AMBIENT_VAR NAME=$(NAME)"
