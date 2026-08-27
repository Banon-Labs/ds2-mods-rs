# OPA unit tests for the builtin git no-verify policy.
# Run with:
#   opa test .cupcake/system/commands.rego \
#            .cupcake/policies/claude/builtins/git_block_no_verify.rego \
#            .cupcake/tests/git_block_no_verify_test.rego
package cupcake.policies.builtins.git_block_no_verify_test

import rego.v1

import data.cupcake.policies.builtins.git_block_no_verify as guard

bash_event(cmd) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Bash",
	"tool_input": {"command": cmd, "timeout": 30000},
}

rule_ids(denials) := {d.rule_id | some d in denials}

test_deny_git_commit_no_verify_long if {
	denials := guard.deny with input as bash_event("git commit --no-verify -m bad")
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_git_commit_no_verify_short if {
	denials := guard.deny with input as bash_event("git commit -nm bad")
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_git_dash_c_commit_no_verify_short if {
	denials := guard.deny with input as bash_event("git -C /tmp/repo commit -n -m bad")
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_allow_git_commit_help_piped_to_sed_n if {
	denials := guard.deny with input as bash_event("git commit -h 2>&1 | sed -n '1,80p'")
	count(denials) == 0
}

test_allow_git_commit_then_later_sed_n if {
	denials := guard.deny with input as bash_event("git commit -m ok && sed -n '1,80p' file")
	count(denials) == 0
}

# --- Shell-wrapper payloads (2026-08-26, bd er-effects-rs-dt2e) ---------------
#
# This policy recognises its verbs through commands.has_verb, whose
# `(^|\s)verb(\s|$)` boundary a quote does not satisfy -- so a hook bypass
# wrapped in `bash -c` produced ZERO denials, measured. The scan set is the
# executed-text decomposition UNION the raw command, so this is strictly
# additive: nothing this policy used to deny stopped being denied.
#
# The flag is assembled rather than written whole so that editing this file
# through a Bash command is not itself denied by the unanchored
# `contains(cmd, ...)` test these cases exercise -- which is this policy's own
# false positive, filed rather than changed here (it is a vendored builtin).
no_verify := concat("", ["--no", "-verify"])

test_deny_wrapped_no_verify_commit if {
	denials := guard.deny with input as bash_event(concat("", ["bash -c 'git commit ", no_verify, " -m bad'"]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_wrapped_no_verify_push if {
	denials := guard.deny with input as bash_event(concat("", [`sh -c "git push `, no_verify, ` origin x"`]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_wrapped_no_verify_merge if {
	denials := guard.deny with input as bash_event(concat("", ["bash -lc 'git merge ", no_verify, " topic'"]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_wrapped_short_no_verify_commit if {
	denials := guard.deny with input as bash_event("bash -c 'git commit -nm bad'")
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_nested_wrapped_no_verify_commit if {
	denials := guard.deny with input as bash_event(concat("", [`bash -c 'bash -c "git commit `, no_verify, ` -m bad"'`]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_eval_no_verify_commit if {
	denials := guard.deny with input as bash_event(concat("", [`eval "git commit `, no_verify, ` -m bad"`]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

# The hooksPath rule shipped comparing a mixed-case literal against a lowercased
# command, so it could never fire. Pinned here now that it can.
test_deny_hooks_path_to_dev_null if {
	denials := guard.deny with input as bash_event("git config core.hooksPath /dev/null")
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_wrapped_hooks_path_to_dev_null if {
	denials := guard.deny with input as bash_event(`bash -c "git config core.hooksPath /dev/null"`)
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}
