# METADATA
# scope: package
# title: Do not end a turn waiting on something nothing is watching
# authors: ["ds2-mods-rs agents"]
# custom:
#   severity: HIGH
#   id: DS2-MODS-NO-UNWATCHED-WAIT
#   description: >-
#     User, 2026-11-02: "Write a rego policy for when you say "I'm waiting" and you don't have a
#     monitor". Earlier the same day the agent's waits on CI were `until ... do sleep` loops with no
#     deadline ("What if it takes infinite time?"), and it ended turns on "Blocked on #368's checks".
#
#     A wait is real only when something will wake the agent: a backgrounded command, whose exit
#     re-invokes it; an async subagent, whose completion notifies; a Monitor. Ending a turn on "I'm
#     waiting for CI" with none of those live means nothing ever comes back, and the user has to
#     notice and ask. This halts that turn end. The fix is to start the watch (a backgrounded poll
#     with a deadline, or a Monitor) or to do the waited-on thing now.
#
#     The signal speaks only on the turn's closing prose, after its last tool call, and never on a
#     wait on the user ("waiting on you to review"), whose reply is the wake-up. The decision is in
#     scripts/cupcake_unwatched_wait.py.
#   routing:
#     required_events: ["Stop"]
#     required_signals: ["last_assistant_unwatched_wait"]
package cupcake.policies.claude.no_unwatched_wait

import rego.v1

halt contains decision if {
	input.hook_event_name == "Stop"
	some p in [phrase]
	decision := {
		"rule_id": "DS2-MODS-NO-UNWATCHED-WAIT",
		"reason": concat("", [
			"You ended the turn waiting ('", p,
			"') with nothing watching: no backgrounded command, async subagent or Monitor is live, so nothing will wake you when it happens. ",
			"Start the watch now -- a backgrounded poll with a deadline, or a Monitor -- or do the thing you are waiting on. ",
			"A wait on the user needs no watcher; say plainly that it is theirs.",
		]),
		"severity": "HIGH",
	}
}

phrase := p if {
	startswith(raw, "UNWATCHED:")
	p := trim(trim_prefix(raw, "UNWATCHED:"), " \t\r\n")
	p != ""
}

raw := trim(signal_value, " \t\r\n")

# Signal value tolerates both the bare-string and {output: ...} shapes cupcake may hand back.
signal_value := s if {
	s := input.signals.last_assistant_unwatched_wait
	is_string(s)
} else := s if {
	s := input.signals.last_assistant_unwatched_wait.output
	is_string(s)
} else := ""
