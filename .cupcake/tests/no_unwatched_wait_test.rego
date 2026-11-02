# OPA unit tests for no_unwatched_wait (the Stop halt on a wait nothing is watching).
# Run with:
#   opa test .cupcake/system/commands.rego \
#     .cupcake/policies/claude/no_unwatched_wait.rego \
#     .cupcake/tests/no_unwatched_wait_test.rego
package cupcake.policies.claude.no_unwatched_wait_test

import rego.v1

import data.cupcake.policies.claude.no_unwatched_wait as guard

stop_event(sig) := {
	"hook_event_name": "Stop",
	"signals": {"last_assistant_unwatched_wait": sig},
}

rule_ids(halts) := {d.rule_id | some d in halts}

test_halt_on_tagged_signal if {
	halts := guard.halt with input as stop_event("UNWATCHED:Blocked on")
	"DS2-MODS-NO-UNWATCHED-WAIT" in rule_ids(halts)
}

test_halt_on_object_signal if {
	halts := guard.halt with input as stop_event({"output": "UNWATCHED:I'm waiting\n", "exit_code": 0})
	"DS2-MODS-NO-UNWATCHED-WAIT" in rule_ids(halts)
}

test_reason_names_the_phrase if {
	some d in guard.halt with input as stop_event("UNWATCHED:waiting for")
	contains(d.reason, "'waiting for'")
}

test_no_halt_on_empty_signal if {
	count(guard.halt) == 0 with input as stop_event("")
}

test_no_halt_on_missing_signal if {
	count(guard.halt) == 0 with input as {"hook_event_name": "Stop", "signals": {}}
}

test_no_halt_on_untagged_value if {
	count(guard.halt) == 0 with input as stop_event("something else")
}

test_no_halt_on_empty_phrase if {
	count(guard.halt) == 0 with input as stop_event("UNWATCHED:  ")
}

test_no_halt_on_other_events if {
	count(guard.halt) == 0 with input as {
		"hook_event_name": "UserPromptSubmit",
		"signals": {"last_assistant_unwatched_wait": "UNWATCHED:Blocked on"},
	}
}
