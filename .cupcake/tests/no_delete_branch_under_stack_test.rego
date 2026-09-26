# OPA unit tests for no_delete_branch_under_stack.
#   opa test .cupcake/system/commands.rego .cupcake/policies/claude/no_delete_branch_under_stack.rego \
#     .cupcake/tests/no_delete_branch_under_stack_test.rego
#
# Pins verdict -> denial and the fail-closed arm. Which command earns which verdict (flag parsing,
# the GitHub questions) is pinned by `python3 scripts/cupcake_stack_merge.py --selftest`; the live
# engine path by scripts/test-cupcake-policies.py.
package cupcake.policies.claude.no_delete_branch_under_stack_test

import rego.v1

import data.cupcake.policies.claude.no_delete_branch_under_stack as guard

ev(cmd, sig) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Bash",
	"tool_input": {"command": cmd},
	"signals": {"stack_merge_dependents": sig},
}

denied(e) if {
	some d in guard.deny with input as e
	d.rule_id == "DS2-MODS-NO-DELETE-BRANCH-UNDER-STACK"
}

reason(e) := r if {
	some d in guard.deny with input as e
	r := d.reason
}

dependents_sig := "STACKMERGE|verb=merge|delete=1|ok=0|why=dependents|pr=129|head=boot-timeline|base=main|deps=131,132"

test_delete_branch_with_dependents_denied if {
	e := ev("gh pr merge 129 --squash --delete-branch", dependents_sig)
	denied(e)
	contains(reason(e), "#131, #132")
	contains(reason(e), "`boot-timeline`")
	contains(reason(e), "gh pr edit <dep> --base main")
	contains(reason(e), "merge without `--delete-branch`")
}

test_short_d_with_dependents_denied if {
	denied(ev("gh pr merge 129 -s -d", dependents_sig))
}

test_delete_branch_without_dependents_allowed if {
	not denied(ev("gh pr merge 129 --squash --delete-branch", "STACKMERGE|verb=merge|delete=1|ok=1|pr=129|head=boot-timeline"))
}

test_merge_without_delete_allowed if {
	not denied(ev("gh pr merge 129 --squash", "STACKMERGE|verb=merge|delete=0"))
}

test_unchecked_denied_and_says_so if {
	e := ev("gh pr merge 129 -d", "STACKMERGE|verb=merge|delete=1|ok=0|why=unchecked")
	denied(e)
	contains(reason(e), "Could not check GitHub")
}

# Fail closed: a delete flag on `gh pr merge` and a silent or `none` signal.
test_silent_signal_with_delete_denied if {
	e := ev("gh pr merge 129 --delete-branch", "")
	denied(e)
	contains(reason(e), "could not check")
}

test_none_signal_with_short_d_denied if {
	denied(ev("gh pr merge 129 -sd", "STACKMERGE|verb=none"))
}

test_object_shaped_signal_read if {
	denied({
		"hook_event_name": "PreToolUse",
		"tool_name": "Bash",
		"tool_input": {"command": "gh pr merge 129 -d"},
		"signals": {"stack_merge_dependents": {"output": dependents_sig}},
	})
}

# Out of jurisdiction.
test_silent_signal_without_delete_allowed if {
	not denied(ev("gh pr merge 129 --squash", ""))
}

test_other_gh_not_gated if {
	not denied(ev("gh pr view 129 --json headRefName", "STACKMERGE|verb=none"))
	not denied(ev("gh pr list --state open --base boot-timeline", "STACKMERGE|verb=none"))
	not denied(ev("gh pr edit 131 --base main", "STACKMERGE|verb=none"))
}

test_quoted_mention_not_gated if {
	not denied(ev("git commit -m 'never gh pr merge 129 -d under a stack'", "STACKMERGE|verb=none"))
}

test_plain_command_not_gated if {
	not denied(ev("ls -la", "STACKMERGE|verb=none"))
}
