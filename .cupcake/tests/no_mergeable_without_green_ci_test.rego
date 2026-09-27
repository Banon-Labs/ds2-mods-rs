package cupcake.policies.claude.no_mergeable_without_green_ci_test

import data.cupcake.policies.claude.no_mergeable_without_green_ci as policy

# The instance that prompted the rule: the word written while `check` was in_progress.
test_pending_ci_with_the_claim_halts if {
	count(policy.halt) == 1 with input as stop_input("MERGEABLECLAIM:1:PENDING")
}

test_failing_ci_with_the_claim_halts if {
	count(policy.halt) == 1 with input as stop_input("MERGEABLECLAIM:1:FAIL")
}

# An unmeasured verdict is not a passing one.
test_unknown_ci_with_the_claim_halts if {
	count(policy.halt) == 1 with input as stop_input("MERGEABLECLAIM:1:UNKNOWN")
}

test_no_pr_with_the_claim_halts if {
	count(policy.halt) == 1 with input as stop_input("MERGEABLECLAIM:1:NOPR")
}

# The one honest use of the word.
test_passing_ci_with_the_claim_is_allowed if {
	count(policy.halt) == 0 with input as stop_input("MERGEABLECLAIM:1:PASS")
}

# The signal appends the PRs it measured; the policy reads only the verdict before them.
test_passing_named_prs_are_allowed if {
	count(policy.halt) == 0 with input as stop_input("MERGEABLECLAIM:1:PASS:216")
}

test_pending_named_pr_halts if {
	count(policy.halt) == 1 with input as stop_input("MERGEABLECLAIM:1:PENDING:216,217")
}

# A turn that never made the claim is untouched, whatever CI says.
test_no_claim_is_untouched_on_red_ci if {
	count(policy.halt) == 0 with input as stop_input("MERGEABLECLAIM:0:FAIL")
}

test_no_claim_is_untouched_on_pending_ci if {
	count(policy.halt) == 0 with input as stop_input("MERGEABLECLAIM:0:PENDING")
}

# No-fabrication: an absent, untagged or truncated signal asserts nothing rather than inventing a
# verdict, so a broken signal can never halt a turn on a claim that was not made.
test_absent_signal_asserts_nothing if {
	count(policy.halt) == 0 with input as {"hook_event_name": "Stop", "signals": {}}
}

test_untagged_signal_asserts_nothing if {
	count(policy.halt) == 0 with input as stop_input("nonsense")
}

test_truncated_signal_asserts_nothing if {
	count(policy.halt) == 0 with input as stop_input("MERGEABLECLAIM:1")
}

# The 2026-09-27 false positive, verbatim: "PR #228 is up as a draft (...) -- draft because repo
# policy refuses non-draft PRs, and it is not ready to merge until the one unproven step happens:
# you launch DS2, press F6 in game, and confirm `DS2LE.log` has no `F6 press detected` line and the
# GPU does not reset." with #228's CI unmeasured. A negated phrase is not a claim, so the signal prints
# nothing for it (.cupcake/tests/fixtures/mergeable_claim_negated.jsonl drives that through the real
# hook); an empty signal must not halt whatever CI says.
test_negated_not_ready_to_merge_prints_nothing_and_is_allowed if {
	count(policy.halt) == 0 with input as stop_input("")
}

# Other events are not this policy's business.
test_other_events_are_untouched if {
	count(policy.halt) == 0 with input as {
		"hook_event_name": "PreToolUse",
		"signals": {"last_assistant_mergeable_claim": "MERGEABLECLAIM:1:FAIL"},
	}
}

# The reason is the user's line and nothing else -- an appended explanation would be the prose the
# directive exists to stop.
test_the_reason_is_exactly_the_one_line if {
	some decision in policy.halt with input as stop_input("MERGEABLECLAIM:1:PENDING")
	decision.reason == "You're a fucking moron."
}

stop_input(signal) := {
	"hook_event_name": "Stop",
	"signals": {"last_assistant_mergeable_claim": signal},
}
