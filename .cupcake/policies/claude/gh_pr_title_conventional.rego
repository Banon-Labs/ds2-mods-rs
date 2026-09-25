# METADATA
# scope: package
# title: Refuse a pull request title that is not this repo's commit header
# authors: ["er-quickload agents", "ds2-mods-rs agents (ported from er-mods-rs)"]
# custom:
#   severity: HIGH
#   id: DS2-MODS-PR-TITLE-CONVENTIONAL
#   description: >-
#     Refuse `gh pr create` / `gh pr edit` whose --title is not a conventional-commit
#     header of this repo's shape. docs/COMMITS.md: "The title of a pull request is a
#     commit header and follows the same rule." A squash merge makes the title the commit
#     on main, and nothing checked it: scripts/check-commit-message.py gates commit
#     messages through .beads/hooks/commit-msg and scripts/check.sh, never the title.
#
#     PORTED from er-mods-rs on 2026-09-25 (`ER-EFFECTS-PR-TITLE-CONVENTIONAL`). There the
#     authority was CI's `pr-title` job; here it is scripts/check-commit-message.py, so the
#     type list is that script's TYPES (er's has `style`, this repo's does not) and the two
#     cheap header rules it states are pinned as well: no trailing full stop, and at most
#     120 characters. Whether a scope names a real crate is left to that script, as er
#     left its length and mood rules to its checker: this is a pre-flight at the moment
#     the title is typed, not a second implementation to drift against the first.
#   routing:
#     required_events: ["PreToolUse"]
#     required_tools: ["Bash"]
package cupcake.policies.claude.gh_pr_title_conventional

import rego.v1

command := object.get(input.tool_input, "command", "")

# `gh pr create` and `gh pr edit`, the two commands that set a title.
gh_pr_title_command if {
	regex.match(`(^|[[:space:];|&('"\x60])gh[[:space:]]+pr[[:space:]]+(create|edit)([[:space:]]|$)`, command)
}

# The title as typed, from either quoting style or bare. Three partial rules rather than one list,
# because a list literal holding an undefined value is itself undefined -- er measured the guard
# falling silent on a single-quoted title that way.
titles contains found[1] if {
	found := regex.find_all_string_submatch_n(`--title[[:space:]]+"([^"]*)"`, command, 1)[0]
}

titles contains found[1] if {
	found := regex.find_all_string_submatch_n(`--title[[:space:]]+'([^']*)'`, command, 1)[0]
}

titles contains found[1] if {
	found := regex.find_all_string_submatch_n(`--title[[:space:]]+([^-"'[:space:]][^[:space:];|&]*)`, command, 1)[0]
}

# TYPES in scripts/check-commit-message.py, and the same `type(scope)!: subject` shape.
conventional(subject) if {
	regex.match(`^(feat|fix|perf|refactor|docs|test|build|ci|chore|revert)(\([^()\n]+\))?!?: \S.*$`, subject)
	not endswith(subject, ".")
	count(subject) <= 120
}

offending_title := t if {
	some t in titles
	not conventional(t)
}

deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"
	gh_pr_title_command
	subject := offending_title

	decision := {
		"rule_id": "DS2-MODS-PR-TITLE-CONVENTIONAL",
		"severity": "HIGH",
		"reason": concat("", [
			"This pull request title is not a commit header of this repo's shape, and a squash merge ",
			"makes it the commit on main. docs/COMMITS.md: the title of a pull request is a commit ",
			"header and follows the same rule. Expected `<type>[(scope)][!]: <subject>` where the type ",
			"is one of feat, fix, perf, refactor, docs, test, build, ci, chore, revert; the scope is a ",
			"crate directory or one of scripts, docs, cupcake, beads, workspace, vendor, github; no ",
			"trailing full stop; at most 120 characters.\n\n  title : ",
			subject,
			"\n\nscripts/check-commit-message.py <file holding the title> gives the authoritative verdict.",
		]),
	}
}
