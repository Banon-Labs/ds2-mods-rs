# METADATA
# scope: package
# title: bd update appends notes, it never replaces them
# authors: ["ds2-mods-rs agents"]
# custom:
#   severity: HIGH
#   id: DS2-MODS-BD-NOTES-APPEND-ONLY
#   description: >-
#     Deny `bd update ... --notes` in agent Bash commands; `--append-notes` is the
#     spelling that adds to an issue.
#
#     `--notes` replaces the whole notes field. Measured 2026-09-26: a history scan
#     (`bd history <id> --json`) found earlier notes text missing from the current
#     notes of 29 issues, every one of them overwritten by an agent that meant to add
#     a finding and used `--notes`. The lost text included verified static findings
#     other issues depended on. Dolt history kept it, so it was recovered, but nothing
#     in the output of `bd update` says anything was discarded.
#
#     `bd create --notes` is untouched: a new issue has no notes to lose. Text
#     mentions are safe for the same reason as the pgrep guard: the invocation is
#     matched on each executed shell text with its quoted spans deleted
#     (commands.quotes_removed).
#
#     AN INTENDED REPLACEMENT (2026-11-02). The deny used to tell agents to "rebuild
#     the full text from `bd history <id> --json` and say so in the note", but it
#     denied every `--notes`, so that path did not exist. Measured: ds2-mods-rs-169u.3
#     had three misfiled copies of another issue's write-up appended to its notes, and
#     the agent cleaning them out was refused. The opt-in is a fixed first line of the
#     new value, `[notes rebuilt from bd history <id>]`, accepted in one narrow shape,
#     read from the RAW shell text (the marker sits inside the quoted value):
#       bd update <id> --notes "[notes rebuilt from bd history <id>]..."
#     with the same <id> in both places, the id directly after `update`, the marker
#     directly inside the opening quote, and `--notes` appearing exactly once in that
#     shell text, quoted prose included. The rest of the value may be `$(cat <file>)`
#     inside the same quotes, so a long rebuilt text need not be typed inline. The
#     marker is stored with the notes, so the replacement says so in the note itself.
#     It is a declaration, not proof that the history was read; what it rules out is
#     the accident this guard was written for.
#
#     VARIABLE HEADS (2026-11-02). The match used to need a literal `bd` token, so
#     `B=$HOME/.local/bin/bd; $B update X --notes "..."` passed; it was run that way
#     and not blocked. A `$NAME` or `${...}` command word now counts as bd, and so
#     does a quoted command word (`"$B"`, `"$HOME/.local/bin/bd"`), which quote
#     deletion leaves as an empty slot in command position right before `update`.
#     Both fail closed: such a command with `update ... --notes` is refused whether
#     or not the variable holds bd.
#   routing:
#     required_events: ["PreToolUse"]
#     required_tools: ["Bash"]
package cupcake.policies.claude.bd_notes_append_only

import rego.v1

import data.cupcake.system.commands

steer := "Use `--append-notes` to add a finding. `--notes` replaces every note already on the issue, and `bd update` does not say what it discarded. If a replacement is genuinely intended, rebuild the full text from `bd history <id> --json` and make the first line of the new value `[notes rebuilt from bd history <id>]`, in exactly this shape: `bd update <id> --notes \"[notes rebuilt from bd history <id>]<newline>$(cat <file>)\"` -- the same id in both places, the id right after `update`, the marker right inside the opening quote, and no other `--notes` anywhere in the command, quoted text included."

deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"
	some source in shell_sources
	unquoted := quotes_deleted(source)
	notes_replaced(unquoted)
	not rebuilt_replacement(source, unquoted)

	decision := {
		"rule_id": "DS2-MODS-BD-NOTES-APPEND-ONLY",
		"severity": "HIGH",
		"reason": concat(" ", ["`bd update --notes` replaces the issue's existing notes.", steer]),
	}
}

raw_command := object.get(input.tool_input, "command", "")

# The command and every shell-wrapper payload it runs, RAW. Each is read twice: with quoted spans
# deleted to find the invocation, and raw to read the opt-in marker, which lives inside the quoted
# value.
shell_sources := {raw_command} | commands.shell_payloads_deep(raw_command)

# Quoted spans deleted, with the quote style that opens first treated as the outer one.
#
# Not commands.quotes_removed, which always deletes `"` spans first and checks no parity. A lone `"`
# inside a single-quoted span then pairs with nothing, and every character after it is treated as
# quoted and deleted. Measured with `opa test` 2026-11-02:
#   echo 'see "x'; bd update a-1 --notes junk
# came out as `echo  ` and was allowed, by this rule as it stood before today too. Here each phase
# must split into balanced spans, double-outer first and single-outer as the fallback (the order
# commands.operands_blanked uses), and a text that balances neither way is scanned raw: that can
# only deny more.
quotes_deleted(text) := out if {
	out := spans_deleted(commands.escaped_quotes_stripped(text), `"`, "'")
} else := out if {
	out := spans_deleted(commands.escaped_quotes_stripped(text), "'", `"`)
} else := text

spans_deleted(text, outer, inner) := out if {
	first := split(text, outer)
	count(first) % 2 == 1
	phase_one := concat(" ", [part | some i; part := first[i]; i % 2 == 0])
	second := split(phase_one, inner)
	count(second) % 2 == 1
	out := concat(" ", [part | some j; part := second[j]; j % 2 == 0])
}

# The bd command word: bare or by path, or a variable (`$B`, `${B}`, `${B:-bd}`).
bd_head := `(([^[:space:];|&()'"]*/)?bd|\$[A-Za-z_][A-Za-z0-9_]*|\$\{[^}[:space:]]*\})`

# `update`, any arguments, then `--notes` as its own flag. `--append-notes` does not match: the
# flag must start at a word boundary with `--notes`.
update_then_notes := `update([[:space:]]+[^[:space:];|&()]+)*[[:space:]]+--notes([[:space:]=]|$)`

notes_replaced(text) if {
	regex.match(concat("", [`(^|[[:space:];|&(])`, bd_head, `[[:space:]]+`, update_then_notes]), text)
}

# A quoted command word (`"$B" update ...`) is deleted with the other quoted spans, which leaves
# `update` itself in command position.
notes_replaced(text) if {
	regex.match(concat("", [`(^|[;|&(\n])[ \t]*`, update_then_notes]), text)
}

rebuilt_marker_pattern := `update[[:space:]]+([A-Za-z0-9._-]+)[[:space:]]+--notes(=|[[:space:]]+)["']\[notes rebuilt from bd history ([A-Za-z0-9._-]+)\]`

rebuilt_replacement(source, unquoted) if {
	# One `--notes` in the whole raw text, quoted prose included, so a marker elsewhere in the
	# command cannot vouch for a second, unmarked replacement. (find_all_string_submatch_n, not
	# find_n: Cupcake's WASM runtime does not implement find_n, scripts/check-cupcake-wasm-builtins.py.)
	count(regex.find_all_string_submatch_n(`(^|[[:space:]'"])--notes([[:space:]=]|$)`, source, -1)) == 1

	invocation := regex.find_all_string_submatch_n(`update[[:space:]]+([A-Za-z0-9._-]+)[[:space:]]+--notes([[:space:]=]|$)`, unquoted, -1)
	count(invocation) == 1
	id := invocation[0][1]

	some marked in regex.find_all_string_submatch_n(rebuilt_marker_pattern, source, -1)
	marked[1] == id
	marked[3] == id
}
