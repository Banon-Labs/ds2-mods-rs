# OPA unit tests for monitor_rate_limit.
#
# Not loaded by the cupcake engine (which scans .cupcake/policies/<harness>/
# and .cupcake/system/ only). Run with:
#   opa test .cupcake/policies/claude/monitor_rate_limit.rego \
#            .cupcake/tests/monitor_rate_limit_test.rego
# End-to-end engine coverage lives in scripts/test-cupcake-policies.py.
package cupcake.policies.claude.monitor_rate_limit_test

import rego.v1

import data.cupcake.policies.claude.monitor_rate_limit as guard

RULE := "DS2-MODS-MONITOR-RATE-LIMIT"

monitor_event(cmd) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Monitor",
	"tool_input": {"command": cmd, "description": "test case", "timeout_ms": 300000},
}

ws_event(url) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Monitor",
	"tool_input": {"ws": {"url": url}, "description": "test case"},
}

rule_ids(denials) := {d.rule_id | some d in denials}

denied(event) if {
	denials := guard.deny with input as event
	RULE in rule_ids(denials)
}

# --- unthrottled streams are DENIED -----------------------------------------

# The exact shape that flooded the session on run br-20260912-202348-248f.
test_deny_tail_grep_without_throttle if {
	denied(monitor_event(`tail -f /home/x/run.log | grep -E --line-buffered "save-dest-picker|stats-text"`))
}

test_deny_bare_tail if {
	denied(monitor_event("tail -f /home/x/run.log"))
}

# A poll loop is no safer: the sleep is inside the loop, not on the emit.
test_deny_poll_loop if {
	denied(monitor_event("while true; do gh pr checks 123; sleep 30; done"))
}

# A throttle that is not the last stage does not bound what reaches the harness.
test_deny_throttle_not_final if {
	denied(monitor_event("tail -f a.log | python3 scripts/monitor-throttle.py 10 | grep --line-buffered ERROR"))
}

# Below the minimum, which is the whole point of the rule.
test_deny_interval_below_minimum if {
	denied(monitor_event("tail -f a.log | python3 scripts/monitor-throttle.py 5"))
}

test_deny_interval_single_digit_nine if {
	denied(monitor_event("tail -f a.log | python3 scripts/monitor-throttle.py 9"))
}

# A leading zero is not a two-digit interval; `01` is one second.
test_deny_interval_leading_zero if {
	denied(monitor_event("tail -f a.log | python3 scripts/monitor-throttle.py 01"))
}

# A similarly-named script is not the committed throttle.
test_deny_lookalike_script if {
	denied(monitor_event("tail -f a.log | python3 scripts/monitor-throttle-fake.py 10"))
}

# --- the self-throttling push helper is ALLOWED ---------------------------------

# scripts/er-push-watched.sh re-executes itself through the committed throttle and
# refuses to run when that file is missing, so the documented one-path form emits a
# throttled stream. Refusing it cost a push on 2026-09-14.
test_allow_er_push_watched_bare if {
	not denied(monitor_event("bash /home/banon/projects/er-mods-rs/scripts/er-push-watched.sh branch branch origin"))
}

# Queued behind another agent's gate run: the wait loop is in front, the script is
# still the last stage.
test_allow_er_push_watched_behind_a_wait_loop if {
	not denied(monitor_event("until flock -n -x /run/user/1000/er-mods-rs-check-sh.lock -c true; do sleep 20; done; bash /abs/scripts/er-push-watched.sh b b origin"))
}

# Belt and braces: the redundant second throttle is still accepted.
test_allow_er_push_watched_with_redundant_throttle if {
	not denied(monitor_event("bash /abs/scripts/er-push-watched.sh b b origin | python3 /abs/scripts/monitor-throttle.py 15"))
}

# ...but piping its output into something else is a stream this rule has not seen.
test_deny_er_push_watched_piped_elsewhere if {
	denied(monitor_event("bash /abs/scripts/er-push-watched.sh b b origin | grep --line-buffered stage"))
}

# --- throttled streams are ALLOWED ------------------------------------------

test_allow_throttled_tail_grep if {
	not denied(monitor_event(`tail -f /home/x/run.log | grep -E --line-buffered "PATTERN" | python3 scripts/monitor-throttle.py 10`))
}

# The bare form defaults to the 10s minimum inside the script itself.
test_allow_throttle_with_no_interval if {
	not denied(monitor_event("tail -f a.log | python3 scripts/monitor-throttle.py"))
}

test_allow_longer_interval if {
	not denied(monitor_event("tail -f a.log | python3 scripts/monitor-throttle.py 60"))
}

# An absolute path, so the rule does not depend on the working directory.
test_allow_absolute_path_to_throttle if {
	not denied(monitor_event("tail -f a.log | python3 /home/x/repo/scripts/monitor-throttle.py 10"))
}

# `python` as well as `python3`, and trailing whitespace.
test_allow_python2_spelling_and_trailing_space if {
	not denied(monitor_event("tail -f a.log | python scripts/monitor-throttle.py 15 "))
}

# Written across lines, which the live engine collapses before evaluation.
test_allow_multiline_command if {
	not denied(monitor_event("tail -f a.log \\\n  | grep --line-buffered X \\\n  | python3 scripts/monitor-throttle.py 10"))
}

# --- websocket monitors are DENIED outright ---------------------------------

test_deny_websocket_monitor if {
	denied(ws_event("wss://events.example.com/stream"))
}

# --- the guard stays out of everything else ---------------------------------

test_allow_other_tools if {
	not denied({
		"hook_event_name": "PreToolUse",
		"tool_name": "Bash",
		"tool_input": {"command": "tail -f a.log"},
	})
}

test_allow_other_events if {
	not denied({
		"hook_event_name": "PostToolUse",
		"tool_name": "Monitor",
		"tool_input": {"command": "tail -f a.log"},
	})
}
