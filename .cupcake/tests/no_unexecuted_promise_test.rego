# OPA unit tests for no_unexecuted_promise (the Stop-event halt on a turn that ended with a
# first-person promise nothing is going to keep).
# Run with:
#   opa test .cupcake/system/commands.rego \
#     .cupcake/policies/claude/no_unexecuted_promise.rego \
#     .cupcake/tests/no_unexecuted_promise_test.rego
#
# These pin the POLICY half: signal tag -> halt, and the correction text. The four false-positive
# carve-outs (contingent on the user, directive to the user, executed in-turn, covered by a live
# background task) are decided in the SIGNAL, and are pinned by scripts/test-unexecuted-promise-signal.py
# against crafted transcripts -- an empty signal here stands for every one of them.
package cupcake.policies.claude.no_unexecuted_promise_test

import rego.v1

import data.cupcake.policies.claude.no_unexecuted_promise as guard

stop_event(sig) := {
	"hook_event_name": "Stop",
	"signals": {"last_assistant_unexecuted_promise": sig},
}

stop_event_object_signal(sig) := {
	"hook_event_name": "Stop",
	"signals": {"last_assistant_unexecuted_promise": {"output": sig, "exit_code": 0}},
}

rule_ids(halts) := {d.rule_id | some d in halts}

# A tagged promise hit halts the turn-end.
test_halt_on_promise_tagged_signal if {
	halts := guard.halt with input as stop_event("PROMISE:I'll re-record the directive with the shell metacharacters escaped.")
	"DS2-MODS-NO-UNEXECUTED-PROMISE" in rule_ids(halts)
}

# Bare/untagged non-empty value (backward compat with crafted signal shapes) still halts.
test_halt_on_bare_string_signal if {
	halts := guard.halt with input as stop_event("I'll re-run the gate.")
	"DS2-MODS-NO-UNEXECUTED-PROMISE" in rule_ids(halts)
}

# Object-shaped signal ({output: ...}) is handled too.
test_halt_on_object_signal if {
	halts := guard.halt with input as stop_event_object_signal("PROMISE:I'll patch the offset.")
	"DS2-MODS-NO-UNEXECUTED-PROMISE" in rule_ids(halts)
}

# The correction quotes the offending sentence back, the way idle_hold's does.
test_reason_quotes_the_offending_phrase if {
	halts := guard.halt with input as stop_event("PROMISE:I'll re-record the directive with the shell metacharacters escaped.")
	some d in halts
	contains(d.reason, "I'll re-record the directive with the shell metacharacters escaped.")
}

# The correction names all THREE ways out, concretely.
test_reason_offers_three_ways_out if {
	halts := guard.halt with input as stop_event("PROMISE:I'll re-run the gate.")
	some d in halts
	contains(d.reason, "EXECUTE IT")
	contains(d.reason, "background task")
	contains(d.reason, "REWRITE")
	contains(d.reason, "re-initiate")
}

# Severity is carried so the halt is reported at the same weight as the sibling Stop guards.
test_halt_is_high_severity if {
	halts := guard.halt with input as stop_event("PROMISE:I'll re-run the gate.")
	some d in halts
	d.severity == "HIGH"
}

# Clean turn (the signal found no violation, or found one of the four exemptions) -> no halt.
test_no_halt_on_clean_turn if {
	halts := guard.halt with input as stop_event("")
	count(halts) == 0
}

# Whitespace-only signal is treated as clean.
test_no_halt_on_whitespace_signal if {
	halts := guard.halt with input as stop_event("   \n")
	count(halts) == 0
}

# Missing signal entirely (routing not satisfied) -> no halt, no evaluation error.
test_no_halt_when_signal_absent if {
	halts := guard.halt with input as {"hook_event_name": "Stop", "signals": {}}
	count(halts) == 0
}

# The halt only applies to Stop events, not other events that might carry the signal.
test_no_halt_on_non_stop_event if {
	halts := guard.halt with input as {
		"hook_event_name": "PreToolUse",
		"signals": {"last_assistant_unexecuted_promise": "PROMISE:I'll re-run the gate."},
	}
	count(halts) == 0
}
