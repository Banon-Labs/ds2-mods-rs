package cupcake.policies.claude.no_unspent_run_evidence_test

import data.cupcake.policies.claude.no_unspent_run_evidence as policy
import rego.v1

# Which machine states make the signal speak (game alive, clean attached build, unpushed, game code)
# is pinned by `python3 scripts/cupcake_unspent_evidence.py --selftest`. These pin the policy half:
# a spoken signal halts with a push instruction, and silence or a malformed line never halts.

stop_with(signal) := {
	"hook_event_name": "Stop",
	"signals": {"unspent_run_evidence": signal},
}

test_unspent_evidence_halts if {
	decisions := policy.halt with input as stop_with("UNSPENTEVIDENCE|branch=estus-max-default-on|sha=2f2d0a5b4bd52163cf2b1d173463ef719b730c11")
	count(decisions) == 1
	some d in decisions
	d.rule_id == "DS2-MODS-NO-UNSPENT-RUN-EVIDENCE"
	d.severity == "HIGH"
	contains(d.reason, "unpushed commit 2f2d0a5b4bd5 on estus-max-default-on")
	contains(d.reason, "Push it now")
	contains(d.reason, "teardown hook then closes the game")
}

test_signal_output_object_halts if {
	decisions := policy.halt with input as {
		"hook_event_name": "Stop",
		"signals": {"unspent_run_evidence": {"output": "UNSPENTEVIDENCE|branch=x|sha=2f2d0a5"}},
	}
	count(decisions) == 1
}

test_missing_branch_still_halts if {
	decisions := policy.halt with input as stop_with("UNSPENTEVIDENCE|sha=2f2d0a5")
	some d in decisions
	contains(d.reason, "unpushed commit 2f2d0a5 and its run evidence")
}

test_empty_signal_allows if {
	count(policy.halt) == 0 with input as stop_with("")
}

test_missing_signal_allows if {
	count(policy.halt) == 0 with input as {"hook_event_name": "Stop", "signals": {}}
}

test_not_a_sha_allows if {
	count(policy.halt) == 0 with input as stop_with("UNSPENTEVIDENCE|branch=x|sha=not-a-sha")
}

test_wrong_tag_allows if {
	count(policy.halt) == 0 with input as stop_with("SOMETHING|branch=x|sha=2f2d0a5")
}

test_other_event_allows if {
	count(policy.halt) == 0 with input as {
		"hook_event_name": "PreToolUse",
		"signals": {"unspent_run_evidence": "UNSPENTEVIDENCE|branch=x|sha=2f2d0a5"},
	}
}
