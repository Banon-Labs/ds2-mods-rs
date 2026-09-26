package cupcake.policies.claude.no_launch_deferral_test

import data.cupcake.policies.claude.no_launch_deferral as policy
import rego.v1

# The signal speaking is the violation; which turns make it speak is pinned by
# scripts/test-launch-deferral-signal.py. These pin the policy half: each kind halts with a
# correction that names ds2-run.py, and silence or a malformed line never halts.

stop_with(signal) := {
	"hook_event_name": "Stop",
	"signals": {"last_assistant_launch_deferral": signal},
}

test_prose_deferral_halts if {
	decisions := policy.halt with input as stop_with("LAUNCHDEFER|kind=prose|phrase=not launching, the game is running")
	count(decisions) == 1
	some d in decisions
	d.rule_id == "DS2-MODS-NO-LAUNCH-DEFERRAL"
	d.severity == "HIGH"
	contains(d.reason, "not launching, the game is running")
	contains(d.reason, "scripts/ds2-run.py")
}

test_agent_instruction_halts if {
	decisions := policy.halt with input as stop_with("LAUNCHDEFER|kind=agent|phrase=do not launch")
	some d in decisions
	contains(d.reason, "subagent")
	contains(d.reason, "do not launch")
	contains(d.reason, "scripts/ds2-run.py")
}

test_unrun_commit_halts if {
	decisions := policy.halt with input as stop_with("LAUNCHDEFER|kind=unrun|phrase=7f313a0")
	some d in decisions
	contains(d.reason, "Commit 7f313a0 touched game code")
	contains(d.reason, "scripts/ds2-run.py")
}

test_a_phrase_holding_separators_survives if {
	decisions := policy.halt with input as stop_with("LAUNCHDEFER|kind=prose|phrase=a | b=c until you're out")
	some d in decisions
	contains(d.reason, "a | b=c until you're out")
}

test_empty_signal_allows if {
	count(policy.halt) == 0 with input as stop_with("")
}

test_missing_signal_allows if {
	count(policy.halt) == 0 with input as {"hook_event_name": "Stop", "signals": {}}
}

test_unknown_kind_allows if {
	count(policy.halt) == 0 with input as stop_with("LAUNCHDEFER|kind=whatever|phrase=x")
}

test_missing_phrase_allows if {
	count(policy.halt) == 0 with input as stop_with("LAUNCHDEFER|kind=prose")
}

test_foreign_line_allows if {
	count(policy.halt) == 0 with input as stop_with("GAMEALIVE|kind=prose|phrase=the game is up")
}

test_not_outside_stop if {
	count(policy.halt) == 0 with input as {
		"hook_event_name": "PreToolUse",
		"signals": {"last_assistant_launch_deferral": "LAUNCHDEFER|kind=prose|phrase=x"},
	}
}

test_object_signal_shape_is_read if {
	decisions := policy.halt with input as stop_with({"output": "LAUNCHDEFER|kind=unrun|phrase=abc1234"})
	count(decisions) == 1
}
