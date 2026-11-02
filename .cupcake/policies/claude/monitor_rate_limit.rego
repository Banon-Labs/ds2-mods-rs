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
#     A second rule, DS2-MODS-MONITOR-RELOAD-MARKER, refuses a monitor on a
#     ds2-frida-watch log whose grep would drop the watcher's `agent.reloaded`
#     line, because the throttle's reload quiet window opens on that line.
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
#
# Interpreter flags that take no argument (`-u`, `-I`, `-B`, `-E`, `-O`, `-q`, `-s`,
# `-S`, alone or clustered) may sit between `python3` and the script path: they
# change buffering or import isolation, never which program runs. `python3 -u
# /abs/scripts/monitor-throttle.py 10` was refused on 2026-11-02 because the old
# pattern required the path to follow `python3` directly. Flags that take an
# argument stay refused -- `-m <mod>` and `-c <prog>` would run something else with
# the throttle path as a mere argv entry, and `-X`/`-W` are not needed here.
#
# After the path, in any order: the interval, and `--reload-quiet SECS` (or `=SECS`), the
# throttle's reload quiet window. The window only ever drops lines, so its value does not
# bear on the rate and any number is accepted, 0 included.
throttled_tail_pattern := `\|[[:space:]]*(uv run [^|]*)?python3?([[:space:]]+-[BEIOqsSu]+)*[[:space:]]+([^|[:space:]-][^|[:space:]]*)?scripts/monitor-throttle\.py([[:space:]]+([1-9][0-9]+(\.[0-9]+)?|--reload-quiet([[:space:]]+|=)[0-9]+(\.[0-9]+)?))*[[:space:]]*$`

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

# --- a Frida log monitor must let the reload marker through -------------------------
#
# `scripts/ds2-frida-watch.py` writes a line tagged `agent.reloaded` before every load of its
# agent, and `scripts/monitor-throttle.py` drops a reload's load-time lines for a quiet window
# that this marker opens. Measured 2026-11-02: the agent re-ran its load-time one-shots on every
# hot reload, one of them sent a line with an `error` key, and `tail -F <log> | grep -E
# '...|error|...' | monitor-throttle.py 10` notified once per edit. A grep in front of the
# throttle that does not keep the marker means the window never opens, so the throttle cannot
# help. A monitor with no grep passes the marker by construction.
#
# A Frida log is a `tail` of a `.jsonl` that is either the summon-sign log, under the watcher's
# default `er-frida/` directory, or in a command that names a scratchpad -- where every agent
# points `--log`, usually through `S=<scratchpad>; ... $S/<name>.jsonl`.
frida_log_tail if {
	regex.match(`(^|[;&|[:space:]])tail[[:space:]][^|]*\.jsonl`, norm_command)
	frida_log_location
}

frida_log_location if contains(norm_command, "summon-sign.jsonl")

frida_log_location if contains(norm_command, "scratchpad")

frida_log_location if contains(norm_command, "er-frida/")

filtering_grep if {
	regex.match(`(^|[;&|[:space:]])(e|f)?grep[[:space:]]`, norm_command)
}

filtering_grep if {
	regex.match(`(^|[;&|[:space:]])rg[[:space:]]`, norm_command)
}

# `agent\.reloaded` or `agent.reloaded`; the `agent\.reload` prefix also keeps it.
keeps_reload_marker if {
	regex.match(`agent\\?\.reload`, norm_command)
}

reload_marker_dropped if {
	frida_log_tail
	filtering_grep
	not keeps_reload_marker
}

block_reason_reload_marker := "🧁 Cupcake blocked a Frida log Monitor whose grep drops the reload marker. scripts/ds2-frida-watch.py writes a line tagged \"agent.reloaded\" before every agent load, and scripts/monitor-throttle.py silences a reload's load-time lines for a quiet window that this line opens. Your grep filters it out, so the window never opens and every load-time line that matches your pattern notifies once per edit -- measured 2026-11-02, when an `error` line from the agent's load-time one-shots fired on every hot reload. Add `agent\\.reloaded` to the grep alternation:\n\n    tail -n0 -F \"$S/summon-sign.jsonl\" | grep -E --line-buffered 'agent\\.reloaded|<your pattern>' | python3 -u scripts/monitor-throttle.py 10\n\nor drop the grep. The marker itself is never shown; a reload notifies only when a line inside its window matched error/exception/detached/Traceback, as one summary."

deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Monitor"
	not websocket_monitor
	reload_marker_dropped

	decision := {
		"rule_id": "DS2-MODS-MONITOR-RELOAD-MARKER",
		"severity": "HIGH",
		"reason": concat("", [block_reason_reload_marker, "\n\nSource: ", command]),
	}
}

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
