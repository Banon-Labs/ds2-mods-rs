# METADATA
# scope: package
# title: Ban ending a turn on a promise nothing is going to keep
# authors: ["ds2-mods-rs agents"]
# custom:
#   severity: HIGH
#   id: DS2-MODS-NO-UNEXECUTED-PROMISE
#   description: >-
#     PORTED FROM er-mods-rs (.cupcake/policies/claude/no_unexecuted_promise.rego, rule id
#     ER-EFFECTS-NO-UNEXECUTED-PROMISE). The rule, the four-fact conjunction and the correction text
#     are that file's; what is local here is the rule id, and the fact that this is this repo's FIRST
#     Stop guard -- so it ships with the WASM-inertness gate that repo learned the hard way already
#     wired into scripts/check.sh. Fixes to the classification belong upstream too, or the two copies
#     start disagreeing about the same turn.
#
#     User directive 2026-08-22, in their words: "How can I prevent you from ever saying 'I'll
#     <statement of future action>' and then landing on no shells running, no monitors, and no
#     directive to me to explain why a user is required to re-initiate the task."
#
#     The instance: a turn ended "I'll re-record the directive with the shell metacharacters escaped
#     rather than leave it unsaved" -- and then stopped. No tool call, no background task, no
#     statement that the user had to do anything. The work simply evaporated and the user had to
#     notice and re-ask. A promise made in prose is enforceable by nothing; only a Stop hook that
#     refuses the stop closes this.
#
#     THE VIOLATION IS A CONJUNCTION OF FOUR FACTS, computed in the signal:
#       1. the turn's FINAL prose block commits the agent to a CONCRETE action ("I'll re-run the
#          gate", "I'm going to patch it", "let me check the offsets");
#       2. no tool_use follows that prose in the turn -- nothing executed it;
#       3. no background work is live -- no backgrounded Bash still awaiting its result, no async
#          subagent that has not notified, no detached shell / Monitor / SendMessage in the turn;
#       4. the message does not hand the obligation to the user -- no question, no blocker
#          statement, no "once you've X", no "I'll need you to Y", no "next session".
#     Any one of the four missing means the turn is fine, and the signal stays silent.
#
#     BIASED HARD TOWARD NOT FIRING. A guard that cries wolf gets ignored, which is worse than no
#     guard, so: only the closing prose block is scanned (a mid-turn "I'll check X" followed by the
#     tool call that checks X is the correct shape and is never touched); the committed verb must be
#     on a concrete-action allowlist, which excludes stance verbs ("I'll keep that in mind"), verbs
#     the message itself fulfils ("I'll summarise"), hedges ("I'll try to") and negations ("I'll
#     never"); and quoted/backticked/fenced spans are stripped so quoting the ban cannot trip it.
#
#     The signal emits PROMISE:<clause> with the offending sentence, which the halt quotes back so
#     the agent knows exactly which sentence it has to make true or withdraw.
#   routing:
#     required_events: ["Stop"]
#     required_signals: ["last_assistant_unexecuted_promise"]
package cupcake.policies.claude.no_unexecuted_promise

import rego.v1

# Enforcement: block turn-end when the just-emitted turn closed on a promise nothing will keep.
halt contains decision if {
	input.hook_event_name == "Stop"
	some p in [phrase]
	decision := {
		"rule_id": "DS2-MODS-NO-UNEXECUTED-PROMISE",
		"reason": reason_for(p),
		"severity": "HIGH",
	}
}

reason_for(p) := msg if {
	msg := concat("", ["You ended the turn on a promise nothing is going to keep: '", p, "'. No tool call executed it, no background task or shell is carrying it, and you did not tell the user the ball is theirs -- so that work evaporates and they have to notice and re-ask. Pick one, now, before stopping: (a) EXECUTE IT in this turn with the tool call that does it; (b) START IT as a background task (a backgrounded Bash or a subagent) so something real is carrying it; or (c) REWRITE the sentence to say plainly that the user must re-initiate it AND why you cannot -- name the blocker. Deleting the promise and staying silent is not an option: if the work matters, say who does it next."])
}

# Parse the tagged signal into the offending clause. Untagged-but-non-empty falls back to the raw
# value so a bare/crafted signal still halts. Empty -> phrase undefined -> no halt.
phrase := p if {
	startswith(raw, "PROMISE:")
	p := trim(trim_prefix(raw, "PROMISE:"), " \t\r\n")
} else := p if {
	raw != ""
	p := raw
}

raw := trim(matched_phrase, " \t\r\n")

# Signal value tolerates both the bare-string and {output: ...} shapes cupcake may hand back.
matched_phrase := p if {
	p := input.signals.last_assistant_unexecuted_promise
	is_string(p)
} else := p if {
	p := input.signals.last_assistant_unexecuted_promise.output
} else := ""
