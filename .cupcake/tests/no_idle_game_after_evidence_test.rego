package cupcake.policies.claude.no_idle_game_after_evidence_test

import data.cupcake.policies.claude.no_idle_game_after_evidence as policy
import rego.v1

# Which sessions make the signal speak is pinned by `python3 scripts/cupcake_idle_game.py --selftest`
# (game alive and launched by this session, ds2-loader.log read since, no reason to keep it) and end
# to end by scripts/test-cupcake-stop-guards.py. These pin the policy half: a spoken signal halts
# with the bare teardown command, and silence -- what the signal prints when the user asked to
# leave the game running or the game is not running -- never halts.

stop_with(signal) := {
	"hook_event_name": "Stop",
	"signals": {"idle_game_after_evidence": signal},
}

# Game alive, evidence grepped, turn ending with no reason (2026-10-20 03:25:17).
test_game_alive_after_evidence_halts if {
	decisions := policy.halt with input as stop_with("IDLEGAME|pid=760735|launched=2026-10-20T03:21:27Z")
	count(decisions) == 1
	some d in decisions
	d.rule_id == "DS2-MODS-NO-IDLE-GAME-AFTER-EVIDENCE"
	d.severity == "HIGH"
	contains(d.reason, "DarkSoulsII.exe (pid 760735)")
	contains(d.reason, "at 2026-10-20T03:21:27Z")
	contains(d.reason, "python3 /home/banon/projects/ds2-mods-rs/scripts/ds2-teardown.py")
	contains(d.reason, "say so and why")
}

test_signal_output_object_halts if {
	decisions := policy.halt with input as {
		"hook_event_name": "Stop",
		"signals": {"idle_game_after_evidence": {"output": "IDLEGAME|pid=42|launched=2026-10-20T03:21:27Z"}},
	}
	count(decisions) == 1
}

test_missing_launch_time_still_halts if {
	decisions := policy.halt with input as stop_with("IDLEGAME|pid=42")
	some d in decisions
	contains(d.reason, "(pid 42) is the game this session launched with ds2-run.py, and")
}

# The user said to leave it running: the signal prints nothing.
test_user_said_leave_it_running_allows if {
	count(policy.halt) == 0 with input as stop_with("")
}

# The game is not running: the signal prints nothing.
test_game_not_running_allows if {
	count(policy.halt) == 0 with input as {"hook_event_name": "Stop", "signals": {}}
}

test_non_numeric_pid_allows if {
	count(policy.halt) == 0 with input as stop_with("IDLEGAME|pid=abc|launched=2026-10-20T03:21:27Z")
}

test_empty_pid_allows if {
	count(policy.halt) == 0 with input as stop_with("IDLEGAME|pid=|launched=2026-10-20T03:21:27Z")
}

test_wrong_tag_allows if {
	count(policy.halt) == 0 with input as stop_with("UNSPENTEVIDENCE|pid=42|launched=x")
}

test_other_event_allows if {
	count(policy.halt) == 0 with input as {
		"hook_event_name": "PreToolUse",
		"signals": {"idle_game_after_evidence": "IDLEGAME|pid=42|launched=x"},
	}
}
