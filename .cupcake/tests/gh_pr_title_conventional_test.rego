# OPA unit tests for gh_pr_title_conventional. Ported from er-mods-rs on 2026-09-25 with this
# repo's type list and header rules.
# Run with:
#   opa test .cupcake/system/commands.rego .cupcake/policies/claude/gh_pr_title_conventional.rego \
#     .cupcake/tests/gh_pr_title_conventional_test.rego
package cupcake.policies.claude.gh_pr_title_conventional_test

import rego.v1

import data.cupcake.policies.claude.gh_pr_title_conventional as guard

RULE := "DS2-MODS-PR-TITLE-CONVENTIONAL"

bash_event(cmd) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Bash",
	"tool_input": {"command": cmd},
}

rule_ids(denials) := {d.rule_id | some d in denials}

denied(cmd) if {
	denials := guard.deny with input as bash_event(cmd)
	RULE in rule_ids(denials)
}

# --- what it refuses ----------------------------------------------------------

# The er-mods-rs 2026-09-09 title, which CI refused five seconds after the PR opened.
test_deny_a_prose_title if {
	denied(`gh pr create --draft --base main --title "Cancel a local invasion that lands in the wrong place" --body-file /tmp/b.md`)
}

test_deny_on_edit_too if {
	denied(`gh pr edit 91 --title "make the thing work"`)
}

test_deny_a_single_quoted_title if {
	denied(`gh pr create --title 'no type here'`)
}

test_deny_a_bare_title if {
	denied("gh pr create --title untyped")
}

test_deny_a_near_miss_type if {
	denied(`gh pr create --title "feature(ds2-menu-row): close but not a listed type"`)
}

# er's list has `style`; scripts/check-commit-message.py does not.
test_deny_a_type_this_repo_does_not_use if {
	denied(`gh pr create --title "style(ds2-menu-row): reflow"`)
}

test_deny_a_colon_without_a_type if {
	denied(`gh pr create --title "menu rows: lock them in multiplayer"`)
}

test_deny_a_type_with_no_subject if {
	denied(`gh pr create --title "feat:"`)
}

test_deny_a_trailing_full_stop if {
	denied(`gh pr create --title "feat(ds2-menu-row): lock the rows in multiplayer."`)
}

test_deny_an_overlong_header if {
	denied(concat("", [`gh pr create --title "feat(cupcake): `, concat("", [x | some _ in numbers.range(1, 110); x := "a"]), `"`]))
}

# --- what it must accept ------------------------------------------------------

test_allow_a_plain_type if {
	not denied(`gh pr create --title "feat: port the er-mods-rs guard policies"`)
}

test_allow_a_crate_scope if {
	not denied(`gh pr create --draft --title "feat(ds2-menu-row): added pause-menu rows are locked and greyed in multiplayer"`)
}

test_allow_a_breaking_marker if {
	not denied(`gh pr create --title "refactor(scripts)!: ds2-run.py takes no switch to play online"`)
}

# --- what it must not reach ---------------------------------------------------

test_allow_a_pr_command_with_no_title if {
	not denied("gh pr create --draft --fill")
}

test_allow_a_pr_view if {
	not denied(`gh pr view 91 --json title`)
}

test_allow_a_git_commit_with_a_bare_subject if {
	not denied(`git commit -m "no type here"`)
}

test_allow_prose_mentioning_it if {
	not denied(`echo "remember to gh pr create later"`)
}

test_the_allow_cases_are_not_vacuous if {
	not denied(`gh pr create --title "feat: a typed subject"`)
	denied(`gh pr create --title "an untyped subject"`)
}
