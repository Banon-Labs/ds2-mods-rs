package cupcake.policies.claude.no_unchecked_game_alive_claim_test

import data.cupcake.policies.claude.no_unchecked_game_alive_claim as policy
import rego.v1

# The signal speaking at all IS the violation: it stays silent unless the turn both claimed a live
# game and ran nothing that could have looked. So these tests pin the rule's half of the contract --
# that a spoken signal halts, that a silent one does not, and that the phrase reaches the correction
# intact even when it carries the characters the field parser splits on.

stop_with(signal) := {
	"hook_event_name": "Stop",
	"signals": {"last_assistant_game_alive": signal},
}

test_halts_on_a_claim_nothing_checked if {
	decisions := policy.halt with input as stop_with("GAMEALIVE|phrase=the game is up")
	count(decisions) == 1
	some d in decisions
	d.rule_id == "DS2-MODS-NO-UNCHECKED-GAME-ALIVE-CLAIM"
	d.severity == "HIGH"
}

# The phrase the user actually read, quoted back at the agent. A correction that cannot name what it
# is correcting is a correction the next turn argues with.
test_the_correction_quotes_the_phrase if {
	decisions := policy.halt with input as stop_with("GAMEALIVE|phrase=the game is up")
	some d in decisions
	contains(d.reason, "the game is up")
	contains(d.reason, "ds2-teardown.py --status")
}

# A phrase carrying the field separators must survive whole. The parser takes everything after the
# marker verbatim precisely so a quoted sentence cannot inject a second key into the fact map --
# two conflicting keys is an eval error, and an eval error in the WASM runtime is a silent allow.
test_a_phrase_holding_a_pipe_survives_intact if {
	decisions := policy.halt with input as stop_with("GAMEALIVE|phrase=press load | character from file=now")
	some d in decisions
	contains(d.reason, "press load | character from file=now")
}

# Silence is the clean turn: the signal only speaks when the conjunction holds.
test_an_empty_signal_does_not_halt if {
	count(policy.halt) == 0 with input as stop_with("")
}

test_a_missing_signal_does_not_halt if {
	count(policy.halt) == 0 with input as {"hook_event_name": "Stop", "signals": {}}
}

# A signal line with no phrase field is not a violation anyone can be told about, so it cannot halt:
# the correction would have nothing to quote and a guessed phrase would be a fabricated fact.
test_a_signal_with_no_phrase_does_not_halt if {
	count(policy.halt) == 0 with input as stop_with("GAMEALIVE|")
}

# Wrong event: this is a Stop guard and must not fire on a tool call.
test_it_does_not_fire_outside_stop if {
	count(policy.halt) == 0 with input as {
		"hook_event_name": "PreToolUse",
		"signals": {"last_assistant_game_alive": "GAMEALIVE|phrase=the game is up"},
	}
}

# Cupcake hands a signal back as either a bare string or {output: ...}; both must be read.
test_the_object_signal_shape_is_read if {
	decisions := policy.halt with input as stop_with({"output": "GAMEALIVE|phrase=the game is running"})
	count(decisions) == 1
	some d in decisions
	contains(d.reason, "the game is running")
}
