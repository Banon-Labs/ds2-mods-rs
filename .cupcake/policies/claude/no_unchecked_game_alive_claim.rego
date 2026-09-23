# METADATA
# scope: package
# title: Ban telling the user the game is running without having looked this turn
# authors: ["ds2-mods-rs agents"]
# custom:
#   severity: HIGH
#   id: DS2-MODS-NO-UNCHECKED-GAME-ALIVE-CLAIM
#   description: >-
#     User directive 2026-09-23, in their words: "Wait. The game is up? Prove it. If you can't
#     prove it, rego policy for bad behavior."
#
#     THE INSTANCE, from the turn immediately before that directive. The agent launched DARK SOULS
#     II, read the launcher's attach block, and then across four turns told the user the game was
#     up and to go press a row in it:
#
#         "The game is up on the new DLL with the rows armed -- press Load Character from File."
#
#     `scripts/ds2-teardown.py --status` answered `nothing running`. The process had crashed on its
#     own minutes earlier, with a `0xc0000005` in the save/load layer recorded by the crash logger.
#     Worse, the agent's own teardown in one of those turns had printed `0 session process(es) to
#     remove` and it read past that line to say the game was up.
#
#     WHAT IS BANNED IS THE TENSE, and the distinction is the whole rule. "I launched it" and "the
#     launcher reported the DLL attached" are claims about a moment that happened, and the launcher
#     proves them properly -- it prints its block only after reading the DLL's own log line. "The
#     game is up" is a claim about now. A process that attached, ran and died leaves the attach
#     block in the scrollback and nothing at all on the system, so the second claim does not follow
#     from the first no matter how recently the first was true.
#
#     WHY IT MATTERS MORE THAN AN ORDINARY WRONG SENTENCE. This claim is an instruction: the user
#     stops what they are doing, turns to a screen, and looks for a game that is not there. It is
#     the same failure the launch-banner rule in AGENTS.md exists for, one step later in time --
#     the banner promises a launch, this promises the launch is still alive.
#
#     THE WAY OUT IS ONE TOOL CALL. `ds2-teardown.py --status` classifies the session's processes
#     and is the direct answer; a `ds2-run.py` launch in the same turn has observed the process it
#     is talking about; `pgrep -x DarkSoulsII.exe` asks the question outright, and is the weakest of
#     the three because the game lives in a bwrap PID namespace where it can answer "nothing" about
#     a game that is on screen. Any of them in the turn's Bash calls and this rule never fires.
#
#     The signal emits ONE facts line -- GAMEALIVE|phrase=<the offending phrase> -- so the
#     OBSERVATION lives in the shell and the RULE lives here, where it is unit-testable. The
#     lexicon, the present-tense/past-tense split and the liveness-command list all live in
#     scripts/cupcake_game_alive.py so the signal, its unit test and any audit cannot drift into
#     three different answers. Empty signal -> no halt.
#
#     KNOWN GAP, the same one every Stop guard here has: an interrupted turn fires no Stop event, so
#     a claim the user cuts short is not caught.
#   routing:
#     required_events: ["Stop"]
#     required_signals: ["last_assistant_game_alive"]
package cupcake.policies.claude.no_unchecked_game_alive_claim

import rego.v1

# Enforcement: block turn-end when the closing prose says the game is alive and nothing looked.
halt contains decision if {
	input.hook_event_name == "Stop"
	phrase != ""
	decision := {
		"rule_id": "DS2-MODS-NO-UNCHECKED-GAME-ALIVE-CLAIM",
		"reason": reason,
		"severity": "HIGH",
	}
}

reason := msg if {
	msg := concat("", ["You told the user the game is running -- '", phrase, "' -- and nothing in this turn looked. Having launched it is not the same claim: a process that attached, ran and died leaves the launcher's block in the scrollback and nothing on the system, and the user acts on what you said by turning to a screen that has no game on it. Run `python3 scripts/ds2-teardown.py --status` and say what it answered, or drop the claim to the past tense you can actually support ('the launcher reported the DLL attached at <time>'). If it says `nothing running`, say THAT -- a dead session is information the user needs, and it is the thing you read past last time."])
}

# --- signal parsing ------------------------------------------------------------------------------
# GAMEALIVE|phrase=<the offending phrase>
#
# The phrase is taken verbatim from after the marker rather than through a field split, for the same
# reason the status-table guard does it: a quoted phrase can contain the `|` and `=` the field
# parser splits on, and two conflicting keys in a fact map is an eval error, which in the WASM
# runtime is an undefined decision, which is a silent ALLOW.
phrase_marker := "|phrase="

phrase_at := indexof(raw, phrase_marker)

phrase := p if {
	phrase_at >= 0
	p := trim(substring(raw, phrase_at + count(phrase_marker), -1), " \t\r\n")
} else := ""

raw := trim(matched_facts, " \t\r\n")

# Signal value tolerates both the bare-string and {output: ...} shapes cupcake may hand back.
matched_facts := p if {
	p := input.signals.last_assistant_game_alive
	is_string(p)
} else := p if {
	p := input.signals.last_assistant_game_alive.output
} else := ""
