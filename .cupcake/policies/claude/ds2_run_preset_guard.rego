# METADATA
# scope: package
# title: Refuse a ds2-run.py Launch That Would Overwrite the User's Lighting Presets
# authors: ["ds2-mods-rs agents"]
# custom:
#   severity: HIGH
#   id: DS2-MODS-REFUSE-PRESET-WIPING-LAUNCH
#   description: >-
#     Deny a Bash command that launches a scripts/ds2-run.py whose file lacks
#     ENGINE_SHIPPED_PRESETS. --dry-run, --selftest and --help stage nothing and
#     are allowed.
#
#     WHY (2026-09-28): the user's DS2 Lighting Engine atmosphere presets,
#     Game/ds2le_atmosphere_presets/atmospheres_extended.ini, saved from the
#     engine's F1 menu, were overwritten with Second Sin's stock copy for the
#     second time. main already keeps them (d694947: a preset file is reinstalled
#     only while it still holds bytes the engine shipped). The launches that wiped
#     it ran ds2-run.py from worktrees whose branches predate that commit, and
#     that copy reinstalls Second Sin's presets over whatever is there. There is
#     one game install; whichever launcher copy the command names is the one that
#     writes into it.
#
#     The file the command runs is resolved by the ds2_run_presets signal
#     (scripts/cupcake_ds2_run_presets.py), which follows a leading cd, bash -c
#     payloads, shell heredocs and absolute paths into .claude/worktrees/*.
#     A launch the signal cannot resolve, and a launch-shaped command the signal
#     never answered for, are refused: this guard fails closed.
#   routing:
#     required_events: ["PreToolUse"]
#     required_tools: ["Bash"]
#     required_signals: ["ds2_run_presets"]
package cupcake.policies.claude.ds2_run_preset_guard

import rego.v1

import data.cupcake.system.commands

merge_hint := "Merge origin/main into that branch first (`git fetch origin && git merge origin/main` in that worktree), then launch its ds2-run.py. main's launcher keeps presets saved from the lighting engine's F1 menu (ENGINE_SHIPPED_PRESETS, d694947); a launcher without it reinstalls Second Sin's atmospheres_extended.ini over the user's own, which happened twice on 2026-09-28."

deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"
	some script in stale_launches

	decision := {
		"rule_id": "DS2-MODS-REFUSE-PRESET-WIPING-LAUNCH",
		"severity": "HIGH",
		"reason": concat("", ["This launches ", script, ", which predates ENGINE_SHIPPED_PRESETS and will overwrite the user's F1-saved lighting presets. ", merge_hint]),
	}
}

deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"
	some pair in unknown_launches

	decision := {
		"rule_id": "DS2-MODS-REFUSE-PRESET-WIPING-LAUNCH",
		"severity": "HIGH",
		"reason": concat("", ["This launches ", pair[0], " and the guard cannot tell which file that is (", pair[1], "), so it cannot show that the launcher keeps the user's F1-saved lighting presets. Spell the script as a plain path. ", merge_hint]),
	}
}

deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"
	not signal_checked
	some text in executed_texts
	launch_shaped(text)

	decision := {
		"rule_id": "DS2-MODS-REFUSE-PRESET-WIPING-LAUNCH",
		"severity": "HIGH",
		"reason": concat(" ", ["This launches ds2-run.py and the ds2_run_presets signal did not answer, so the guard cannot show that the launcher keeps the user's F1-saved lighting presets.", merge_hint]),
	}
}

executed_texts := commands.executed_unquoted_texts(object.get(input.tool_input, "command", ""))

# Only consulted when the signal is silent. A ds2-run.py path in command position or as a python
# interpreter's script, with no flag that makes it stage nothing.
launch_shaped(text) if {
	regex.match(`(^|[;|&(][[:space:]]*|(python|pypy)[0-9.]*[[:space:]]+(-[^[:space:]]+[[:space:]]+)*)[^[:space:];|&()'"]*ds2-run\.py([[:space:];|&)]|$)`, text)
	not regex.match(`(^|[[:space:]])(--dry-run|--selftest|--help|-h)([[:space:];|&)]|$)`, text)
}

stale_launches contains script if {
	some line in signal_lines
	parts := split(line, "\t")
	count(parts) == 3
	parts[0] == "launch"
	parts[2] == "presets=0"
	script := parts[1]
}

unknown_launches contains [parts[1], parts[2]] if {
	some line in signal_lines
	parts := split(line, "\t")
	count(parts) == 3
	parts[0] == "unknown"
}

signal_checked if {
	some line in signal_lines
	line == "CHECKED"
}

signal_text := value if {
	value := input.signals.ds2_run_presets
	is_string(value)
} else := value if {
	value := input.signals.ds2_run_presets.output
	is_string(value)
} else := ""

signal_lines := split(signal_text, "\n")
