package cupcake.policies.claude.no_user_property_grant_test

import data.cupcake.policies.claude.no_user_property_grant as policy
import rego.v1

# The signal speaking at all IS the violation: it stays silent unless the closing prose granted the
# user something already theirs. So these tests pin the rule's half of the contract -- that a
# spoken signal halts, that a silent one does not, and that the phrase reaches the correction
# intact even when it carries the characters the field parser splits on.

stop_with(signal) := {
	"hook_event_name": "Stop",
	"signals": {"last_assistant_property_grant": signal},
}

test_halts_on_a_spoken_grant if {
	decisions := policy.halt with input as stop_with("PROPERTYGRANT|phrase=yours to end")
	count(decisions) == 1
	some d in decisions
	d.rule_id == "DS2-MODS-NO-USER-PROPERTY-GRANT"
	d.severity == "HIGH"
}

# The phrase the user actually read, quoted back at the agent. A correction that cannot name what
# it is correcting is a correction the next turn argues with.
test_the_correction_quotes_the_phrase if {
	decisions := policy.halt with input as stop_with("PROPERTYGRANT|phrase=yours to end")
	some d in decisions
	contains(d.reason, "yours to end")
	contains(d.reason, "the DLL is built; I have not relaunched")
}

# A phrase carrying the field separators must survive whole. The parser takes everything after the
# marker verbatim precisely so a quoted sentence cannot inject a second key into the fact map --
# two conflicting keys is an eval error, and an eval error in the WASM runtime is a silent allow.
test_a_phrase_holding_a_pipe_survives_intact if {
	decisions := policy.halt with input as stop_with("PROPERTYGRANT|phrase=up to you | feel free=to")
	some d in decisions
	contains(d.reason, "up to you | feel free=to")
}

# Silence is the clean turn: the signal only speaks when the conjunction holds.
test_an_empty_signal_does_not_halt if {
	count(policy.halt) == 0 with input as stop_with("")
}

test_a_missing_signal_does_not_halt if {
	count(policy.halt) == 0 with input as {"hook_event_name": "Stop", "signals": {}}
}

# A signal line with no phrase field is not a violation anyone can be told about, so it cannot
# halt: the correction would have nothing to quote and a guessed phrase would be a fabricated fact.
test_a_signal_with_no_phrase_does_not_halt if {
	count(policy.halt) == 0 with input as stop_with("PROPERTYGRANT|")
}

# Wrong event: this is a Stop guard and must not fire on a tool call.
test_it_does_not_fire_outside_stop if {
	count(policy.halt) == 0 with input as {
		"hook_event_name": "PreToolUse",
		"signals": {"last_assistant_property_grant": "PROPERTYGRANT|phrase=yours to end"},
	}
}

# Cupcake hands a signal back as either a bare string or {output: ...}; both must be read.
test_the_object_signal_shape_is_read if {
	decisions := policy.halt with input as stop_with({"output": "PROPERTYGRANT|phrase=feel free to"})
	count(decisions) == 1
	some d in decisions
	contains(d.reason, "feel free to")
}
