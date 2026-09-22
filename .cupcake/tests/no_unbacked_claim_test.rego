# OPA unit tests for no_unbacked_claim (Stop-event halt on a completion claim nothing backs).
# Run with:
#   opa test .cupcake/system/commands.rego \
#     .cupcake/policies/claude/no_unbacked_claim.rego \
#     .cupcake/tests/no_unbacked_claim_test.rego
package cupcake.policies.claude.no_unbacked_claim_test

import rego.v1

import data.cupcake.policies.claude.no_unbacked_claim as guard

stop_event(sig) := {
	"hook_event_name": "Stop",
	"signals": {"last_assistant_unbacked_claim": sig},
}

rule_ids(halts) := {d.rule_id | some d in halts}

test_halt_on_tagged_signal if {
	halts := guard.halt with input as stop_event("UNBACKED:I added the check to check.sh.")
	"DS2-MODS-NO-UNBACKED-CLAIM" in rule_ids(halts)
}

test_halt_on_untagged_signal if {
	halts := guard.halt with input as stop_event("I built the gate")
	"DS2-MODS-NO-UNBACKED-CLAIM" in rule_ids(halts)
}

test_halt_on_object_shaped_signal if {
	halts := guard.halt with input as {
		"hook_event_name": "Stop",
		"signals": {"last_assistant_unbacked_claim": {"output": "UNBACKED:I wired in the hook.", "exit_code": 0}},
	}
	"DS2-MODS-NO-UNBACKED-CLAIM" in rule_ids(halts)
}

# A clean turn emits nothing, and nothing must halt.
test_no_halt_on_empty_signal if {
	count(guard.halt) == 0 with input as stop_event("")
}

# The guard is Stop-only: a PreToolUse carrying the same signal must not halt.
test_no_halt_on_other_event if {
	count(guard.halt) == 0 with input as {
		"hook_event_name": "PreToolUse",
		"signals": {"last_assistant_unbacked_claim": "UNBACKED:I built the gate"},
	}
}

# The offending sentence is quoted back, so the agent knows what to fix or withdraw.
test_reason_quotes_the_claim if {
	halts := guard.halt with input as stop_event("UNBACKED:I created scripts/foo.py.")
	some d in halts
	contains(d.reason, "I created scripts/foo.py.")
}
