# OPA unit tests for no_restating_user_own_rule (the Stop-event halt on a turn that hands the user
# back a rule the user themselves authored).
# Run with:
#   opa test .cupcake/system/commands.rego \
#     .cupcake/policies/claude/no_restating_user_own_rule.rego \
#     .cupcake/tests/no_restating_user_own_rule_test.rego
#
# The split of duty, same as the sibling guards. This file pins the RULE: which combination of facts
# halts a turn and what the correction says. The prose classification that produces those facts --
# which sentences restate a user-owned rule, and which are exempt -- lives in the SIGNAL and is
# pinned by `scripts/cupcake_user_own_rule.py --selftest` against the verbatim failure.
package cupcake.policies.claude.no_restating_user_own_rule_test

import rego.v1

import data.cupcake.policies.claude.no_restating_user_own_rule as guard

# The clause the signal lifts out of the verbatim turn that prompted this guard (2026-09-23,
# closing a turn that had just opened draft pull request #481).
verbatim := "undrafting is yours"

facts(clause, command, guardevent) := concat("", [
	"OWNRULEFACTS|clause=", clause,
	"|ruleid=ownership",
	"|command=", command,
	"|guardevent=", guardevent,
])

stop_event(sig) := {
	"hook_event_name": "Stop",
	"signals": {"last_assistant_user_own_rule": sig},
}

stop_event_object_signal(sig) := {
	"hook_event_name": "Stop",
	"signals": {"last_assistant_user_own_rule": {"output": sig, "exit_code": 0}},
}

# --- the failure this guard exists for -----------------------------------------------------------

test_verbatim_restatement_halts if {
	count(guard.halt) == 1 with input as stop_event(facts(verbatim, "0", "0"))
}

test_halt_names_the_clause_back if {
	some decision in guard.halt with input as stop_event(facts(verbatim, "0", "0"))
	contains(decision.reason, verbatim)
}

test_halt_carries_the_rule_id if {
	some decision in guard.halt with input as stop_event(facts(verbatim, "0", "0"))
	decision.rule_id == "DS2-MODS-NO-RESTATING-USER-OWN-RULE"
}

# --- the two exemptions --------------------------------------------------------------------------

# `AGENTS.md` requires handing the user the pasteable command for work the agent may not do.
test_handing_over_the_command_is_allowed if {
	count(guard.halt) == 0 with input as stop_event(facts(verbatim, "1", "0"))
}

# A guard that fired this turn is an event the user was not watching for.
test_reporting_a_guard_event_is_allowed if {
	count(guard.halt) == 0 with input as stop_event(facts(verbatim, "0", "1"))
}

# --- clean turns ---------------------------------------------------------------------------------

test_empty_signal_does_not_halt if {
	count(guard.halt) == 0 with input as stop_event("")
}

test_missing_signal_does_not_halt if {
	count(guard.halt) == 0 with input as {"hook_event_name": "Stop", "signals": {}}
}

test_other_event_does_not_halt if {
	count(guard.halt) == 0 with input as {
		"hook_event_name": "PreToolUse",
		"signals": {"last_assistant_user_own_rule": facts(verbatim, "0", "0")},
	}
}

# --- signal shape --------------------------------------------------------------------------------

test_object_shaped_signal_is_read if {
	count(guard.halt) == 1 with input as stop_event_object_signal(facts(verbatim, "0", "0"))
}

# A degraded signal that names a clause but omits the exemption fields must fail CLOSED, matching
# how every sibling guard treats an untagged non-empty value.
test_degraded_signal_fails_closed if {
	count(guard.halt) == 1 with input as stop_event(concat("", ["OWNRULEFACTS|clause=", verbatim]))
}
