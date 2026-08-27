# METADATA
# scope: package
# title: No pgrep/pkill Full-Command-Line Matching (-f)
# authors: ["ds2-mods-rs agents"]
# custom:
#   severity: HIGH
#   id: DS2-MODS-BLOCK-PGREP-FULL-MATCH
#   description: >-
#     Deny `pgrep -f` / `pkill -f` (and `--full`) in agent Bash commands. Match the
#     process NAME with -x instead.
#
#     NOT A PORT of er-mods-rs's block_manual_pgrep, which bans pgrep OUTRIGHT. That
#     guard's premise is explicitly WSL: on that box Steam and the game are native
#     Windows processes pgrep cannot see, so `pgrep -x steam` is a false NEGATIVE and
#     only tasklist.exe tells the truth. THAT PREMISE IS FALSE HERE. This is native
#     CachyOS/Arch, Steam and DarkSoulsII.exe are ordinary Linux processes, and
#     `pgrep -x DarkSoulsII.exe` is exactly how the game was confirmed closed on
#     2026-08-26. Porting the ban would forbid a technique that is CORRECT on this
#     machine -- the textbook case of a guard carrying its authoring box's premises
#     into a repo where they do not hold.
#
#     The hazard here is the OTHER one, and it is the flag, not the tool. `-f`
#     matches the FULL COMMAND LINE of every process -- including the agent's own
#     shell, which is running a command that by construction contains the pattern
#     being searched for. Measured twice in a single session, 2026-08-26:
#
#       1. `pgrep -f 'DarkSoulsII.exe'` matched the agent's own command line and
#          returned a pid, and the agent reported the game ALIVE. It was not. A
#          fabricated runtime claim, produced by the check that was supposed to
#          prevent one.
#       2. `pkill -f 'ds2-run.py --probe neuter'` matched the agent's own shell and
#          killed it. Exit 144.
#
#     Both are self-matches. `-x` matches the executable name only and cannot match
#     the shell that is asking, which is why it is the correct instrument here and
#     why this guard steers rather than forbids.
#
#     There is deliberately NO escape hatch for an executing -f. A legitimate need
#     for full-command-line matching belongs in a committed helper script, where it
#     is reviewable and is not an agent Bash command. Text mentions are already safe
#     without an exemption: the flag test runs over commands.executed_unquoted_texts,
#     which DELETES quoted spans, so a bd issue or commit message quoting `pgrep -f`
#     is not an execution of it.
#   routing:
#     required_events: ["PreToolUse"]
#     required_tools: ["Bash"]
package cupcake.policies.claude.block_pgrep_full_match

import rego.v1

import data.cupcake.system.commands

steer := "Use `-x` (match the executable name) instead. `-f` matches every process's FULL command line, including the agent's own shell -- which is running a command that contains the pattern by construction. That self-match has produced a fabricated 'process is ALIVE' claim and has killed the agent's own shell (exit 144) in this repo. If full-command-line matching is genuinely required, put it in a committed helper script rather than an inline command."

deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"
	some text in executed_texts
	full_match_flag_used(text)

	decision := {
		"rule_id": "DS2-MODS-BLOCK-PGREP-FULL-MATCH",
		"severity": "HIGH",
		"reason": concat(" ", ["`pgrep -f` / `pkill -f` matches the agent's own command line.", steer]),
	}
}

# Fail closed on a payload this guard cannot read, gated on the text naming one of
# the two tools so an unrelated opaque wrapper is not answered with a pgrep denial.
deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"
	commands.unparsed_shell_payload(raw_command)
	tool_named

	decision := {
		"rule_id": "DS2-MODS-BLOCK-PGREP-FULL-MATCH",
		"severity": "HIGH",
		"reason": concat(" ", ["This command wraps a shell payload the guard cannot read (an unquoted or substituted `-c`/`eval` argument) while naming pgrep or pkill, so it cannot be shown not to use `-f`.", steer]),
	}
}

raw_command := object.get(input.tool_input, "command", "")

# executed_UNQUOTED_texts: quoted spans are DELETED rather than anchor-neutralised,
# which is what commands.rego documents it for ("for substring/flag tests"). Anchor
# blanking leaves spaces intact, so a quoted sentence mentioning `pgrep -f` still
# reads as a command to a space-anchored pattern. Deleting the span is what lets an
# issue or commit message describe this rule without tripping it.
executed_texts := commands.executed_unquoted_texts(raw_command)

tool_named if {
	regex.match(`(?i)\bp(grep|kill)\b`, raw_command)
}

# The tool token, then its options, then a short cluster containing `f` or the long
# `--full`. Case-sensitive on the flag letter on purpose: `-F` is `--pidfile`, an
# entirely different option that does not match command lines, and denying it would
# be a false positive. The cluster form catches `-af`, `-lf`, `-fl` and friends,
# which are the spellings that would otherwise walk straight past a naive `-f`
# search.
full_match_flag_used(text) if {
	regex.match(`(^|[[:space:];|&(])([^[:space:];|&()'"]*/)?p(grep|kill)([[:space:]]+[^[:space:];|&()]+)*[[:space:]]+-[a-zA-Z]*f[a-zA-Z]*([[:space:]]|$)`, text)
}

full_match_flag_used(text) if {
	regex.match(`(^|[[:space:];|&(])([^[:space:];|&()'"]*/)?p(grep|kill)([[:space:]]+[^[:space:];|&()]+)*[[:space:]]+--full([[:space:]=]|$)`, text)
}
