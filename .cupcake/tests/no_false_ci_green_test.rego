# OPA unit tests for no_false_ci_green.
#
# The case that created the policy is `test_pending_forbids_the_word`: an answer called PR #384
# "green" while `gh pr checks` read `check pending`, on the strength of a local check.sh exit 0.
#
# Run with:
#   opa test .cupcake/policies/claude/no_false_ci_green.rego \
#     .cupcake/tests/no_false_ci_green_test.rego
package cupcake.policies.claude.no_false_ci_green_test

import rego.v1

import data.cupcake.policies.claude.no_false_ci_green as guard

ups(sig) := {
	"hook_event_name": "UserPromptSubmit",
	"signals": {"ci_state_for_branch": sig},
}

says(ctxs, needle) if {
	some c in ctxs
	contains(c, needle)
}

# The incident, verbatim from the signal that now measures it.
test_pending_forbids_the_word if {
	ctxs := guard.add_context with input as ups("CISTATE:fix/ersc-v201-invasion-warp-repin:384:PENDING:byte-identical=skipped,scope=success,check=in_progress")
	says(ctxs, "is PENDING")
	says(ctxs, "DO NOT write")
	says(ctxs, "check=in_progress")
}

# A local gate is explicitly named as not being this measurement, because that is the exact
# substitution that produced the false claim.
test_pending_names_the_local_gate_substitution if {
	ctxs := guard.add_context with input as ups("CISTATE:b:1:PENDING:check=queued")
	says(ctxs, "check.sh")
}

test_failing_leads_with_the_failure if {
	ctxs := guard.add_context with input as ups("CISTATE:b:7:FAIL:check=failure")
	says(ctxs, "is FAILING")
	says(ctxs, "check=failure")
}

# PASS licenses the word, and only about CI.
test_pass_licenses_the_word_but_scopes_it if {
	ctxs := guard.add_context with input as ups("CISTATE:b:9:PASS:check=success")
	says(ctxs, "is PASSING")
	says(ctxs, "CI ONLY")
}

test_no_pr_is_not_a_pass if {
	ctxs := guard.add_context with input as ups("CISTATE:b:0:NOPR:no pull request for this branch")
	says(ctxs, "NO pull request")
	not says(ctxs, "PASSING")
}

# An unmeasured verdict must not read as a passing one -- the failure mode the whole policy exists
# to stop, in its quietest form.
test_unknown_is_not_a_pass if {
	ctxs := guard.add_context with input as ups("CISTATE:b:3:UNKNOWN:gh returned no check rows")
	says(ctxs, "unknown")
	not says(ctxs, "PASSING")
}

# A summary containing ":" is rejoined, not truncated at the first colon.
test_summary_with_colons_survives if {
	ctxs := guard.add_context with input as ups("CISTATE:b:5:FAIL:job:one=failure,job:two=success")
	says(ctxs, "job:one=failure")
	says(ctxs, "job:two=success")
}

# No signal, or an untagged one, asserts NOTHING -- absence is not evidence of a pass.
test_absent_signal_asserts_nothing if {
	ctxs := guard.add_context with input as ups("")
	count(ctxs) == 0
}

test_untagged_signal_asserts_nothing if {
	ctxs := guard.add_context with input as ups("everything looks fine")
	count(ctxs) == 0
}

test_truncated_tag_asserts_nothing if {
	ctxs := guard.add_context with input as ups("CISTATE:b:5:FAIL")
	count(ctxs) == 0
}

# UserPromptSubmit only: a Stop verdict is rendered to the user verbatim and fires after the claim
# has already been read.
test_contributes_nothing_on_stop if {
	ctxs := guard.add_context with input as {
		"hook_event_name": "Stop",
		"signals": {"ci_state_for_branch": "CISTATE:b:1:FAIL:check=failure"},
	}
	count(ctxs) == 0
}

test_defines_no_halt if {
	not guard.halt
}
