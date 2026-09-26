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
#     mentions are safe for the same reason as the pgrep guard: the test runs over
#     commands.executed_unquoted_texts, which deletes quoted spans.
#   routing:
#     required_events: ["PreToolUse"]
#     required_tools: ["Bash"]
package cupcake.policies.claude.bd_notes_append_only

import rego.v1

import data.cupcake.system.commands

steer := "Use `--append-notes` to add a finding. `--notes` replaces every note already on the issue, and `bd update` does not say what it discarded. If a replacement is genuinely intended, rebuild the full text from `bd history <id> --json` and say so in the note."

deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"
	some text in executed_texts
	notes_replaced(text)

	decision := {
		"rule_id": "DS2-MODS-BD-NOTES-APPEND-ONLY",
		"severity": "HIGH",
		"reason": concat(" ", ["`bd update --notes` replaces the issue's existing notes.", steer]),
	}
}

raw_command := object.get(input.tool_input, "command", "")

executed_texts := commands.executed_unquoted_texts(raw_command)

# The bd token (bare or by path), `update`, any arguments, then `--notes` as its own flag.
# `--append-notes` does not match: the flag must start at a word boundary with `--notes`.
notes_replaced(text) if {
	regex.match(`(^|[[:space:];|&(])([^[:space:];|&()'"]*/)?bd[[:space:]]+update([[:space:]]+[^[:space:];|&()]+)*[[:space:]]+--notes([[:space:]=]|$)`, text)
}
