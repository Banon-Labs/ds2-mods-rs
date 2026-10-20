# METADATA
# scope: package
# title: Do not end a turn with the game this session launched still up after its evidence was read
# authors: ["ds2-mods-rs agents"]
# custom:
#   severity: HIGH
#   id: DS2-MODS-NO-IDLE-GAME-AFTER-EVIDENCE
#   description: >-
#     User, 2026-10-19: "Is there a reason the game is still up? If not, we lack a rego policy to
#     tear it down as soon as evidence is collected". Earlier, 2026-09-26: "I just want the game to
#     go down when you're done collecting evidence."
#
#     The instance: commit d5e5404 on feat/config-hot-reload was launched with scripts/ds2-run.py, its
#     `live-config` lines were grepped out of ds2-loader.log, and the commit was then amended twice for
#     rustfmt and rustdoc. The teardown hook only fires on a push, and DS2-MODS-NO-UNSPENT-RUN-EVIDENCE
#     only speaks while the running build is a commit some worktree HEAD still holds, so the amend
#     silenced both and the game sat idle through a turn end until the next launch replaced it.
#
#     This halts the turn end on the run itself, not on a commit. The signal speaks only when this
#     session's transcript has a real ds2-run.py launch, a later tool call read ds2-loader.log, and
#     `pgrep -x DarkSoulsII.exe` finds a game that started within five minutes of that launch. It is
#     silent while the evidence is still being collected (no log read since the last launch), when
#     the user asked to leave the game running or said they are playing, when the user wrote about
#     the run after the launch within the last 30 minutes (a hand test), when the agent said in its
#     prose why it is keeping the game up ("leaving the game running until ..."), and while
#     DS2-MODS-NO-UNSPENT-RUN-EVIDENCE is already demanding the push that tears it down. The decision
#     is in scripts/cupcake_idle_game.py.
#
#     It halts instead of running the teardown: a kill from a Stop hook would land on a misjudged turn
#     with nobody watching. The command it hands over is the existing bare teardown, which
#     DS2-MODS-TEARDOWN-MUST-RELAUNCH allows as a whole command. It never asks for a launch to be
#     withheld, so it does not cut against "always launch over" (DS2-MODS-NO-LAUNCH-DEFERRAL).
#
#     Known gap, shared by every Stop guard here: an interrupted turn fires no Stop event.
#   routing:
#     required_events: ["Stop"]
#     required_signals: ["idle_game_after_evidence"]
package cupcake.policies.claude.no_idle_game_after_evidence

import rego.v1

teardown_command := "python3 /home/banon/projects/ds2-mods-rs/scripts/ds2-teardown.py"

halt contains decision if {
	input.hook_event_name == "Stop"
	pid != ""
	decision := {
		"rule_id": "DS2-MODS-NO-IDLE-GAME-AFTER-EVIDENCE",
		"reason": concat("", [
			"DarkSoulsII.exe (pid ", pid, ") is the game this session launched with ds2-run.py", launched_note,
			", and its evidence has already been read from ds2-loader.log. Tear it down now, as a command on its own:\n\n    ",
			teardown_command,
			"\n\nIf the run has to stay up (an experiment still in progress, or the user is about to use it), say so and why in your reply, e.g. \"leaving the game running until the reload line lands\".",
		]),
		"severity": "HIGH",
	}
}

launched_note := concat("", [" at ", launched]) if launched != "" else := ""

# --- signal parsing ------------------------------------------------------------------------------
# IDLEGAME|pid=<pid>|launched=<iso>
pid_marker := "|pid="

launched_marker := "|launched="

raw := trim(matched_facts, " \t\r\n")

pid_at := indexof(raw, pid_marker)

launched_at := indexof(raw, launched_marker)

pid := p if {
	startswith(raw, "IDLEGAME")
	pid_at >= 0
	launched_at > pid_at
	start := pid_at + count(pid_marker)
	p := substring(raw, start, launched_at - start)
	digits_only(p)
} else := p if {
	startswith(raw, "IDLEGAME")
	pid_at >= 0
	launched_at < 0
	p := substring(raw, pid_at + count(pid_marker), -1)
	digits_only(p)
} else := ""

launched := l if {
	launched_at >= 0
	l := trim(substring(raw, launched_at + count(launched_marker), -1), " \t\r\n")
} else := ""

# No regex: the rulebook's compiled-regex cache crashes the WASM runtime past a point (see
# teardown_must_relaunch.rego), and a crashed evaluation silences every policy at once.
digits_only(s) if {
	count(s) > 0
	stripped := trim(s, "0123456789")
	stripped == ""
}

matched_facts := p if {
	p := input.signals.idle_game_after_evidence
	is_string(p)
} else := p if {
	p := input.signals.idle_game_after_evidence.output
} else := ""
