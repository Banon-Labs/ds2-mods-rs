package cupcake.policies.claude.no_stop_while_frida_attached_test

import data.cupcake.policies.claude.no_stop_while_frida_attached as policy
import rego.v1

# Which sessions make the signal speak is pinned by `python3 scripts/cupcake_frida_attached.py
# --selftest` (game up, this session's watcher attached, a closing that hands off nothing, nothing
# torn down) and end to end by scripts/test-cupcake-stop-guards.py. These pin the policy half: a
# spoken signal halts with the continue-now reason, and silence -- what the signal prints when the
# turn asked the user something, the game is down, no watcher is alive, the watcher is another
# session's, or the turn tore down -- never halts.

stop_with(signal) := {
	"hook_event_name": "Stop",
	"signals": {"frida_attached_turn_end": signal},
}

# 2026-11-02 19:51: the summon-sign tracer attached, the turn ended on a finding.
test_instance_halts if {
	decisions := policy.halt with input as stop_with("FRIDAATTACHED|game=766429|watcher=summon-sign-trace-frozen.js")
	count(decisions) == 1
	some d in decisions
	d.rule_id == "DS2-MODS-NO-STOP-WHILE-FRIDA-ATTACHED"
	d.severity == "HIGH"
	contains(d.reason, "DarkSoulsII.exe pid 766429")
	contains(d.reason, "(agent summon-sign-trace-frozen.js)")
	contains(d.reason, "Continue the investigation now, in this turn")
	contains(d.reason, "in one line")
	contains(d.reason, "destructive step")
	contains(d.reason, "python3 /home/banon/projects/ds2-mods-rs/scripts/ds2-teardown.py")
}

test_signal_output_object_halts if {
	decisions := policy.halt with input as {
		"hook_event_name": "Stop",
		"signals": {"frida_attached_turn_end": {"output": "FRIDAATTACHED|game=42|watcher=tick.js"}},
	}
	count(decisions) == 1
}

test_missing_watcher_name_still_halts if {
	decisions := policy.halt with input as stop_with("FRIDAATTACHED|game=42")
	some d in decisions
	contains(d.reason, "(DarkSoulsII.exe pid 42) with the Frida watcher this session started attached, and")
}

# The turn ended on a question to the user: the signal prints nothing.
test_question_to_user_allows if {
	count(policy.halt) == 0 with input as stop_with("")
}

# The game is not running, no watcher is alive, or the watcher is another session's: no signal key.
test_game_not_running_allows if {
	count(policy.halt) == 0 with input as {"hook_event_name": "Stop", "signals": {}}
}

test_no_watcher_allows if {
	count(policy.halt) == 0 with input as stop_with("\n")
}

# Teardown in the turn: the signal prints nothing.
test_teardown_allows if {
	count(policy.halt) == 0 with input as stop_with(" ")
}

test_non_numeric_pid_allows if {
	count(policy.halt) == 0 with input as stop_with("FRIDAATTACHED|game=abc|watcher=tick.js")
}

test_empty_pid_allows if {
	count(policy.halt) == 0 with input as stop_with("FRIDAATTACHED|game=|watcher=tick.js")
}

test_wrong_tag_allows if {
	count(policy.halt) == 0 with input as stop_with("IDLEGAME|game=42|watcher=tick.js")
}

test_other_event_allows if {
	count(policy.halt) == 0 with input as {
		"hook_event_name": "PreToolUse",
		"signals": {"frida_attached_turn_end": "FRIDAATTACHED|game=42|watcher=tick.js"},
	}
}
