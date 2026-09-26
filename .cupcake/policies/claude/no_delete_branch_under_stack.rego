# METADATA
# scope: package
# title: Do Not Delete a Branch Another Open PR Is Based On
# authors: ["ds2-mods-rs agents"]
# custom:
#   severity: HIGH
#   id: DS2-MODS-NO-DELETE-BRANCH-UNDER-STACK
#   routing:
#     required_events: ["PreToolUse"]
#     required_tools: ["Bash"]
#     required_signals: ["stack_merge_dependents"]
package cupcake.policies.claude.no_delete_branch_under_stack

import rego.v1

import data.cupcake.system.commands

# Incident 2026-09-26: the base PRs of stacks were merged with `gh pr merge --delete-branch`. GitHub
# CLOSES an open pull request whose base branch is deleted; it does not retarget it. #107, #108, #115
# and #129 were closed that way, #131 merged into #129's branch instead of main, and all of them had
# to be recreated as #139-#143.
#
# Refused: `gh pr merge <N>` with `--delete-branch` or `-d` (clustered `-sd` too) while another open
# PR in the same repository has PR <N>'s head branch as its base. The merge itself is never refused;
# only deleting the branch under a stack is.
#
# ALL DECIDING IS IN scripts/cupcake_stack_merge.py (flag parsing, `gh pr view`, `gh pr list`),
# because composed regexes are dropped by cupcake's WASM runtime. This file maps the verdict to a
# sentence.
#
# FAIL CLOSED like pr_requires_run_stamp: when GitHub cannot be asked (`why=unchecked`), or the
# command names `gh pr merge` with a delete flag and the signal said nothing, the delete is refused
# and the reason says the check could not run. Merging without the delete flag stays open.

deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"
	field("verb") == "merge"
	field("delete") == "1"
	field("ok") != "1"

	decision := {
		"rule_id": "DS2-MODS-NO-DELETE-BRANCH-UNDER-STACK",
		"reason": concat(" ", [why_text, how_to_fix]),
		"severity": "HIGH",
	}
}

deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"
	names_merge_with_delete
	field("verb") != "merge"

	decision := {
		"rule_id": "DS2-MODS-NO-DELETE-BRANCH-UNDER-STACK",
		"reason": concat(" ", [
			"This command runs `gh pr merge` with `--delete-branch`/`-d`, and the stack check could not read it, so it could not check whether another open PR is based on the branch being deleted.",
			"Run the gh command directly, not through a wrapper the check cannot parse.",
			how_to_fix,
		]),
		"severity": "HIGH",
	}
}

names_merge_with_delete if {
	some text in commands.executed_texts(input.tool_input.command)
	regex.match(`(^|[;&|(\n])[ \t]*([a-z_][a-z0-9_]*=[^ \t]*[ \t]+)*((command|env|exec|time)[ \t]+)*([^ \t;&|()]*/)?gh[ \t]+pr[ \t]+merge([ \t;&|)]|$)`, lower(text))
	regex.match(`(^|[ \t])(--delete-branch|-[a-z]*d)([ \t=;&|)]|$)`, lower(text))
}

how_to_fix := "Deleting a branch that open PRs use as their base makes GitHub CLOSE those PRs instead of retargeting them. Instead: retarget each dependent to this PR's base first (`gh pr edit <dep> --base main`), then merge with `--delete-branch`; or merge without `--delete-branch` and delete the branch (`git push origin --delete <branch>`) after the dependents are retargeted."

why_text := concat("", [
	"PR #", field("pr"), "'s head branch `", field("head"),
	"` is the base of open PR(s) #", replace(field("deps"), ",", ", #"),
	". Retarget them to `", field("base"), "` before deleting it.",
]) if {
	field("why") == "dependents"
} else := "Could not check GitHub for open PRs based on this PR's head branch (`gh pr view`/`gh pr list` failed, or the directory the command runs in could not be resolved), so the delete cannot be shown to be safe." if {
	field("why") == "unchecked"
} else := "The stack check returned no verdict."

# --- Reading the signal -------------------------------------------------------

signal_text := out if {
	out := input.signals.stack_merge_dependents
	is_string(out)
} else := out if {
	out := input.signals.stack_merge_dependents.output
	is_string(out)
} else := ""

# `STACKMERGE|verb=merge|delete=1|ok=0|why=dependents|...` -> one key's value, or "" when absent.
field(key) := value if {
	startswith(trim_space(signal_text), "STACKMERGE|")
	some part in split(trim_space(signal_text), "|")
	startswith(part, concat("", [key, "="]))
	value := trim_prefix(part, concat("", [key, "="]))
} else := ""
