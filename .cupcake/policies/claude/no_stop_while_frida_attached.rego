# METADATA
# scope: package
# title: Do not end a turn on unfinished work while the game is up with this session's Frida watcher attached
# authors: ["ds2-mods-rs agents"]
# custom:
#   severity: HIGH
#   id: DS2-MODS-NO-STOP-WHILE-FRIDA-ATTACHED
#   description: >-
#     User, 2026-11-02: "I didn't ask you to do anything except for ensure that I never have to ask
#     you to keep going when a project isn't completed while the game is up and Frida is attached".
#     Before that: "Yes, it is. And your point? Why did you pause? To make me mad?"
#
#     The instance: DarkSoulsII.exe was running and this session had attached
#     scripts/ds2-frida-watch.py (a backgrounded `uv run --with frida python3
#     scripts/ds2-frida-watch.py --agent $S/summon-sign-trace-frozen.js ...`) to find out why an NPC
#     summon sign is invisible. The turn ended on a finding and nothing else: "Your position is pinned
#     down, and it isn't guesswork. ... So the last sign really was made 1.5 m from you, and it's
#     invisible for some reason other than where it is." No question, no request, no blocker, and
#     every tool to continue was in reach. The user had to say keep going.
#
#     The signal speaks only when the game is alive (`pgrep -x DarkSoulsII.exe`), a ds2-frida-watch.py
#     process is alive (read from /proc/*/cmdline), this session's own transcript (the Stop event's
#     `transcript_path`) has a Bash call that ran that watcher with the same `--agent` file, the
#     closing message neither hands off nor names a blocker (scripts/cupcake_handoff.py, shared with
#     DS2-MODS-NO-DESCRIBED-NEXT-STEP, plus a bare in-game imperative such as "walk to the sign"), the
#     turn neither tore down nor stopped the watcher, no other background work of the session is
#     still running, and the user did not open the turn by saying stop, wait or that they are playing.
#     The decision is in scripts/cupcake_frida_attached.py.
#
#     A finished investigation is not an exemption on its own, on purpose: when the work is done the
#     watcher has no business staying attached to the game, so the way to end that turn is to stop it
#     or tear down, which the signal then sees.
#
#     Known gaps, shared by every Stop guard here: an interrupted turn fires no Stop event, and the
#     reply to a halt is judged one Stop later, once its prose is in the transcript.
#   routing:
#     required_events: ["Stop"]
#     required_signals: ["frida_attached_turn_end"]
package cupcake.policies.claude.no_stop_while_frida_attached

import rego.v1

halt contains decision if {
	input.hook_event_name == "Stop"
	game != ""
	decision := {
		"rule_id": "DS2-MODS-NO-STOP-WHILE-FRIDA-ATTACHED",
		"reason": concat("", [
			"The game is up (DarkSoulsII.exe pid ", game, ") with the Frida watcher this session started attached",
			watcher_note,
			", and the work is not done: you ended the turn on a statement, with no question to the user, no request and no blocker. ",
			"Continue the investigation now, in this turn: make the next tool call (read the watcher's log, edit the agent, take the next measurement). ",
			"Stop only to ask the user, in one line, for something only they can supply (an in-game action or what is on their screen), ",
			"before a destructive step that needs their approval, or after tearing down (stop the watcher, or run python3 /home/banon/projects/ds2-mods-rs/scripts/ds2-teardown.py) because the work is finished.",
		]),
		"severity": "HIGH",
	}
}

watcher_note := concat("", [" (agent ", watcher, ")"]) if watcher != ""

else := ""

# --- signal parsing ------------------------------------------------------------------------------
# FRIDAATTACHED|game=<pid>|watcher=<agent file name>
game_marker := "|game="

watcher_marker := "|watcher="

raw := trim(matched_facts, " \t\r\n")

game_at := indexof(raw, game_marker)

watcher_at := indexof(raw, watcher_marker)

game := g if {
	startswith(raw, "FRIDAATTACHED")
	game_at >= 0
	watcher_at > game_at
	start := game_at + count(game_marker)
	g := substring(raw, start, watcher_at - start)
	digits_only(g)
} else := g if {
	startswith(raw, "FRIDAATTACHED")
	game_at >= 0
	watcher_at < 0
	g := substring(raw, game_at + count(game_marker), -1)
	digits_only(g)
} else := ""

watcher := w if {
	watcher_at >= 0
	w := trim(substring(raw, watcher_at + count(watcher_marker), -1), " \t\r\n")
} else := ""

# No regex: the rulebook's compiled-regex cache crashes the WASM runtime past a point (see
# teardown_must_relaunch.rego), and a crashed evaluation silences every policy at once.
digits_only(s) if {
	count(s) > 0
	stripped := trim(s, "0123456789")
	stripped == ""
}

matched_facts := p if {
	p := input.signals.frida_attached_turn_end
	is_string(p)
} else := p if {
	p := input.signals.frida_attached_turn_end.output
} else := ""
