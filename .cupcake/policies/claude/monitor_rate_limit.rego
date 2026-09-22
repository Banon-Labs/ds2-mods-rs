# METADATA
# scope: package
# title: Monitor event streams must be rate-limited to one notification per 10s
# authors: ["er-mods-rs agents", "ds2-mods-rs agents"]
# custom:
#   severity: HIGH
#   id: DS2-MODS-MONITOR-RATE-LIMIT
#   description: >-
#     Hard block on a `Monitor` call whose event stream can fire faster than once
#     per 10 seconds. Every stdout line a Monitor command emits becomes a
#     conversation message, and a `tail -f | grep` over a live game log has no
#     natural rate. On run br-20260912-202348-248f a single power-of-two-backed-off
#     log line produced notifications for minutes and the user interrupted the
#     session three times to stop it; the harness's own rate suppression only
#     engages after the flood has been delivered. A tighter grep is not a fix --
#     the agent cannot know in advance which pattern a live log will hammer.
#     So the rate is enforced structurally: the command's final pipeline stage
#     must be `scripts/monitor-throttle.py` with an interval of at least 10,
#     which coalesces a burst into one line carrying the suppressed count.
#     `ws:` monitors are refused outright: a pushed frame stream has no stage to
#     append a throttle to. Wrap it in a bash command through the throttle.
#   routing:
#     required_events: ["PreToolUse"]
#     required_tools: ["Monitor"]
package cupcake.policies.claude.monitor_rate_limit

import rego.v1

command := object.get(input.tool_input, "command", "")

# Whitespace-normalized, so a command written across several lines is matched the
# same way the live engine (which collapses whitespace) and `opa test` (which does
# not) both see it. Mirrors block_manual_pgrep's pgrep_norm_command for the same
# reason, and sticks to builtins proven to work in the wasm host.
norm_command := concat(" ", [word |
	some word in split(replace(replace(replace(command, "\t", " "), "\r", " "), "\n", " "), " ")
	word != ""
])

# The required final stage: a pipe into the committed throttle with an explicit
# interval of 10 or more seconds. The interval is matched as a literal rather than
# compared numerically because the engine evaluates these as wasm modules; two
# digits or more is >= 10 for any integer without a leading zero, and a bare
# `monitor-throttle.py` with no argument defaults to the 10s minimum in the script
# itself. A leading path is allowed (`python3 /abs/path/scripts/monitor-throttle.py`)
# so the rule does not depend on the working directory.
throttled_tail_pattern := `\|[[:space:]]*(uv run [^|]*)?python3?[[:space:]]+[^|[:space:]]*scripts/monitor-throttle\.py([[:space:]]+[1-9][0-9]+(\.[0-9]+)?)?[[:space:]]*$`

throttled if {
	regex.match(throttled_tail_pattern, norm_command)
}

# The other shape that cannot emit an unthrottled stream: a command whose last
# stage IS scripts/er-push-watched.sh. That script re-executes itself through the
# committed throttle and exits 2 when the throttle file is missing, so its stdout is
# throttled by construction and a second throttle appended by the caller only
# coalesces an already-coalesced stream. Its own header tells the caller to name one
# path and no second one; before this rule that documented command was refused, which
# cost a push on 2026-09-14 and teaches the next agent to bolt on a redundant stage.
# The pattern requires no pipe after the script, so piping its output somewhere else
# is still refused -- that would be a stream this rule has not seen.
self_throttling_tail_pattern := `scripts/er-push-watched\.sh[^|]*$`

throttled if {
	regex.match(self_throttling_tail_pattern, norm_command)
}

# A `ws:` monitor has no pipeline, so there is nowhere to put the throttle.
websocket_monitor if {
	object.get(input.tool_input, "ws", null) != null
}

block_reason_command := "🧁 Cupcake blocked an unthrottled Monitor. Every stdout line a Monitor emits becomes a conversation message, and a `tail -f | grep` over a live log has no natural rate -- on run br-20260912-202348-248f one backed-off log line notified for minutes and the user had to interrupt the session three times. A narrower grep is not the fix: you cannot know in advance which line a live log will hammer. End the pipeline with the committed throttle, which coalesces a burst into one line carrying the suppressed count:\n\n    tail -f <log> | grep -E --line-buffered \"<pattern>\" | python3 scripts/monitor-throttle.py 10\n\nThe interval must be 10 or more (a bare `monitor-throttle.py` defaults to 10). For a SINGLE notification -- 'tell me when X finishes' -- do not use Monitor at all: run a Bash command with run_in_background that exits when the condition is true."

block_reason_ws := "🧁 Cupcake blocked a `ws:` Monitor. A pushed frame stream has no pipeline stage to append a rate limit to, so it can notify without bound. Wrap it in a bash command that pipes the frames through the committed throttle instead: `websocat <url> | python3 scripts/monitor-throttle.py 10`."

deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Monitor"
	websocket_monitor

	decision := {
		"rule_id": "DS2-MODS-MONITOR-RATE-LIMIT",
		"severity": "HIGH",
		"reason": block_reason_ws,
	}
}

deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Monitor"
	not websocket_monitor
	not throttled

	decision := {
		"rule_id": "DS2-MODS-MONITOR-RATE-LIMIT",
		"severity": "HIGH",
		"reason": concat("", [block_reason_command, "\n\nSource: ", command]),
	}
}
