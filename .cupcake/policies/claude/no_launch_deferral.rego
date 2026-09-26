# METADATA
# scope: package
# title: Never withhold a DARK SOULS II launch because the game is running, or leave a build unrun
# authors: ["ds2-mods-rs agents"]
# custom:
#   severity: HIGH
#   id: DS2-MODS-NO-LAUNCH-DEFERRAL
#   description: >-
#     User directive 2026-09-26, verbatim: "if you ever stop runtime testing new builds while we
#     are running claude, I'm going to lobotomize you".
#
#     The instance. The main agent had a committed build (the PR #192 merge, 7f313a0) that the push
#     guard required to be run. It refused to launch it because the user's DS2 session was running
#     and "the user plans to go back to it", and told a subagent not to launch or attach at all. The
#     day before, the user had said "Please for the love of god, always launch over", and that was
#     recorded as bd memory `ds2-always-launch-over-running-game-2026-09-25`. A memory is advisory;
#     this is the enforcement.
#
#     A running game is never a reason to defer a launch: scripts/ds2-run.py tears the session down
#     and relaunches, and that is expected while agents work.
#
#     Three shapes halt the turn, all decided in scripts/cupcake_launch_deferral.py:
#       * prose -- a sentence that withholds a launch and gives the game being in use as the reason;
#       * agent -- a subagent prompt forbidding a launch or attach because the user is in the game;
#       * unrun -- a `git commit` this session that touched `crates/` or `scripts/ds2-run.py`, with
#         no ds2-run.py launch anywhere after it in the transcript. The push guard only speaks when a
#         push is attempted; an agent that commits and ends the turn never reaches it.
#     Past-tense reports of a launch that happened pass, a deferral followed by a real launch in the
#     same turn passes, and the user's own latest prompt saying not to launch silences the signal.
#
#     Known gap, shared by every Stop guard here: an interrupted turn fires no Stop event.
#   routing:
#     required_events: ["Stop"]
#     required_signals: ["last_assistant_launch_deferral"]
package cupcake.policies.claude.no_launch_deferral

import rego.v1

halt contains decision if {
	input.hook_event_name == "Stop"
	phrase != ""
	kind != ""
	decision := {
		"rule_id": "DS2-MODS-NO-LAUNCH-DEFERRAL",
		"reason": reason,
		"severity": "HIGH",
	}
}

launch_now := "Launch it now with `python3 scripts/ds2-run.py` (it tears the running game down and relaunches -- that is expected). A running or in-use game is never a reason to defer a launch; the user's directive of 2026-09-26 is that every new build gets runtime-tested while agents work."

reason := msg if {
	kind == "unrun"
	msg := concat("", ["Commit ", phrase, " touched game code this session and nothing launched it afterwards. ", launch_now])
} else := msg if {
	kind == "agent"
	msg := concat("", ["You told a subagent '", phrase, "' because the user is in the game. Do not withhold launches from agents for that reason. ", launch_now])
} else := msg if {
	msg := concat("", ["You withheld a launch because the game is running or in use -- '", phrase, "'. ", launch_now])
}

# --- signal parsing ------------------------------------------------------------------------------
# LAUNCHDEFER|kind=<prose|agent|unrun>|phrase=<text>
#
# The phrase is everything after its marker, verbatim, so a quoted sentence holding `|` or `=`
# cannot inject a second key; the kind sits before it and is read up to the phrase marker.
kind_marker := "|kind="

phrase_marker := "|phrase="

raw := trim(matched_facts, " \t\r\n")

phrase_at := indexof(raw, phrase_marker)

kind_at := indexof(raw, kind_marker)

phrase := p if {
	startswith(raw, "LAUNCHDEFER")
	phrase_at >= 0
	p := trim(substring(raw, phrase_at + count(phrase_marker), -1), " \t\r\n")
} else := ""

kind := k if {
	startswith(raw, "LAUNCHDEFER")
	kind_at >= 0
	phrase_at > kind_at
	start := kind_at + count(kind_marker)
	k := substring(raw, start, phrase_at - start)
	k in {"prose", "agent", "unrun"}
} else := ""

matched_facts := p if {
	p := input.signals.last_assistant_launch_deferral
	is_string(p)
} else := p if {
	p := input.signals.last_assistant_launch_deferral.output
} else := ""
