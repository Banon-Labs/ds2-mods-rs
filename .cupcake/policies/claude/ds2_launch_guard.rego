# METADATA
# scope: package
# title: Launch DARK SOULS II Only Through the Testimony-Gated Launcher
# authors: ["ds2-mods-rs agents"]
# custom:
#   severity: HIGH
#   id: DS2-MODS-LAUNCH-GUARD
#   description: >-
#     Deny a RAW launch of appid 335300 -- `steam -applaunch 335300`, a `steam://run/`
#     or `steam://rungameid/` URL, or a wine/proton invocation naming DarkSoulsII.exe
#     -- issued as an agent Bash command. Launches go through scripts/ds2-run.py.
#
#     NOT A PORT of er-mods-rs's bash_elden_ring_launch_guard, which forbids launching
#     that game through Steam AT ALL because of EAC bans. DS2 SOTFS has no EAC and
#     `steam -applaunch 335300` is exactly how this repo's own launcher starts the
#     game, so the ER guard ported verbatim would block scripts/ds2-run.py itself.
#     Same intent, inverted mechanism: not "never launch" but "never launch bare".
#
#     WHY: ds2-run.py is testimony-gated. It stages dinput8.dll into the Game dir,
#     records both the BUILT and the STAGED sha256 so a stale DLL is visible rather
#     than silent, and prints its success block ONLY after reading the DLL's own log
#     line out of <Game>/ds2-loader.log. A bare applaunch does none of that, and the
#     result is a launch that LOOKS like it proved something and did not -- an agent
#     reporting "the DLL loaded" with no line of testimony behind it. That failure
#     shape is the one this repo spends the most effort refusing to produce.
#
#     ds2-run.py needs no exemption here. It runs `steam -applaunch` from inside a
#     file on disk, and a file on disk is never an agent Bash command, so this policy
#     never sees it. Same structural escape hatch the sibling repo grants
#     scripts/steam-running.sh.
#   routing:
#     required_events: ["PreToolUse"]
#     required_tools: ["Bash"]
package cupcake.policies.claude.ds2_launch_guard

import rego.v1

import data.cupcake.system.commands

launcher_hint := "Launch DARK SOULS II through `python3 scripts/ds2-run.py` instead. It stages the DLL, records the built and staged sha256, and prints its success block only after reading the DLL's own log line -- a bare launch proves nothing about what was loaded."

deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"
	some text in executed_texts
	raw_steam_launch(lower(text))

	decision := {
		"rule_id": "DS2-MODS-LAUNCH-GUARD",
		"severity": "HIGH",
		"reason": concat(" ", ["This launches appid 335300 through Steam directly, bypassing the testimony gate.", launcher_hint]),
	}
}

deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"
	some text in executed_texts
	raw_wine_launch(lower(text))

	decision := {
		"rule_id": "DS2-MODS-LAUNCH-GUARD",
		"severity": "HIGH",
		"reason": concat(" ", ["This runs DarkSoulsII.exe through wine/proton directly, bypassing the testimony gate.", launcher_hint]),
	}
}

# Fail closed on a wrapper this guard cannot read, exactly as the main-push guard
# does. `bash -c $CMD` hands a shell a program that is not in the command text, so
# no pattern can rule out a launch. Gated on the text naming BOTH a launch channel
# and the appid so an unrelated opaque wrapper is not answered with a launch
# denial; a payload naming neither is outside any text policy's reach, and saying
# so is better than pretending otherwise.
deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"
	opaque_launch_payload

	decision := {
		"rule_id": "DS2-MODS-LAUNCH-GUARD",
		"severity": "HIGH",
		"reason": concat(" ", ["This command wraps a shell payload the guard cannot read (an unquoted or substituted `-c`/`eval` argument) while naming Steam and appid 335300, so it cannot be shown not to launch the game bare.", launcher_hint]),
	}
}

# executed_UNQUOTED_texts, not executed_texts. Both decompose shell wrappers, so
# `bash -c 'steam -applaunch 335300'` is caught either way. They differ in what they
# do to a quoted span, and that difference decides this policy:
#
#   executed_texts        blanks ANCHORS inside quotes -- separators, parens, the
#                         quotes themselves -- but NOT spaces. So a mid-sentence
#                         mention inside a quoted operand still carries a space
#                         before `steam`, still satisfies this pattern's anchor
#                         class, and still denies. Measured while writing this
#                         policy: `bd create --description "Deny a raw steam
#                         -applaunch 335300 ..."` was denied by its own rule, which
#                         would make the guard impossible to document or file
#                         issues about.
#
#   executed_unquoted_texts  DELETES quoted spans outright. The mention disappears;
#                         a real bare launch, which is not quoted, does not.
#
# LIMIT, stated rather than papered over: quoting the appid itself
# (`steam -applaunch "335300"`) deletes it from the scanned text and defeats this.
# That is a deliberate-evasion spelling, not an accidental one. A text-scanning
# guard cannot beat someone who is trying to get past it; this one exists to stop
# the accidental bare launch, and the opaque-payload rule below is what refuses to
# guess when the text genuinely cannot be read.
executed_texts := commands.executed_unquoted_texts(input.tool_input.command)

lower_command := lower(object.get(input.tool_input, "command", ""))

# `steam -applaunch 335300`, with or without a path prefix, and tolerating the
# options Steam accepts between the binary and the flag.
raw_steam_launch(text) if {
	regex.match(`(^|[[:space:];|&(])([^[:space:];|&()'"]*/)?steam([[:space:]]+-{1,2}[a-z][a-z0-9-]*)*[[:space:]]+-{1,2}applaunch[[:space:]]+335300([^0-9]|$)`, text)
}

# The URL forms, however they are handed to the desktop: `xdg-open`, `steam`,
# `gio open`, a bare paste. The scheme itself is the launch, so the pattern does
# not care which opener carries it.
raw_steam_launch(text) if {
	regex.match(`steam://(run|rungameid)/335300([^0-9]|$)`, text)
}

# wine/proton naming the game executable. Deliberately gated on a LAUNCHER verb
# rather than on the exe name alone: `objdump -d .../DarkSoulsII.exe`,
# `strings`, and `xxd` all name the same path, and static RE on that binary is
# the single most encouraged activity in this repo. Denying it would be the guard
# fighting the work instead of protecting it.
raw_wine_launch(text) if {
	regex.match(`(^|[[:space:];|&(])([^[:space:];|&()'"]*/)?(wine|wine64|wineconsole|proton|protontricks)([[:space:]]|$)`, text)
	contains(text, "darksoulsii.exe")
}

# Plain substrings, not word tokens: the payload is unreadable precisely because
# it is a variable, and `bash -c $LAUNCH_CMD` carries no standalone token to find.
opaque_launch_payload if {
	commands.unparsed_shell_payload(object.get(input.tool_input, "command", ""))
	contains(lower_command, "335300")
	launch_channel_named
}

launch_channel_named if {
	contains(lower_command, "steam")
}

launch_channel_named if {
	contains(lower_command, "applaunch")
}
