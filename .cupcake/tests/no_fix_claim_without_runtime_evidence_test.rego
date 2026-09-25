# OPA unit tests for no_fix_claim_without_runtime_evidence: the Stop-event halt on a closing message
# that calls a change to game-loaded code a fix with no run behind it.
#
# Run with:
#   opa test .cupcake/system/commands.rego \
#     .cupcake/policies/claude/no_fix_claim_without_runtime_evidence.rego \
#     .cupcake/tests/no_fix_claim_without_runtime_evidence_test.rego
#
# These cases exercise the conjunction, which is what lives in the policy. The prose classification
# -- which sentences count as a fix claim at all, and which paths count as game-loaded code -- lives
# in scripts/cupcake_fix_claim.py and is proven by scripts/test-fix-claim-classifier.py, because a
# Rego test cannot see a regex in a shell signal. Both halves are needed: a policy green on
# hand-typed facts says nothing about whether a real transcript ever produces them, and that is the
# exact way every Stop guard in this repo stayed inert for 36 days.
package cupcake.policies.claude.no_fix_claim_without_runtime_evidence_test

import rego.v1

import data.cupcake.policies.claude.no_fix_claim_without_runtime_evidence as guard

# A facts line with the culpable half set and every exemption clear, so each test can flip exactly
# the one field it is about.
fix_facts(clause) := concat("", [
	"FIXFACTS|claim=", clause,
	"|changed=1|evidence=0|hedged=0|blocked=0|hostobject=0",
])

# The same line with one field substituted. Substituted rather than appended: the policy builds its
# fact map with an object comprehension, so a repeated key is a conflict rather than an override,
# and appending would test an evaluation error instead of the rule.
fix_facts_with(clause, field, value) := replace(
	fix_facts(clause),
	concat("", [field, "=0"]),
	concat("", [field, "=", value]),
)

stop_event(sig) := {
	"hook_event_name": "Stop",
	"signals": {"last_assistant_fix_claim_without_runtime_evidence": sig},
}

rule_ids(halts) := {d.rule_id | some d in halts}

fix_claim := "DS2-MODS-NO-FIX-CLAIM-WITHOUT-RUNTIME-EVIDENCE"

# --- the verbatim shape -------------------------------------------------------------------------
# The phrase the user quoted, in the sentence that closed the turn which then failed live.
test_halt_on_the_real_fix if {
	halts := guard.halt with input as stop_event(fix_facts("That is the real fix."))
	fix_claim in rule_ids(halts)
}

test_halt_on_this_is_the_fix if {
	halts := guard.halt with input as stop_event(fix_facts("This is the fix: the row was cloned before the menu existed."))
	fix_claim in rule_ids(halts)
}

test_halt_on_this_fixes_it if {
	halts := guard.halt with input as stop_event(fix_facts("This fixes the crash on the second load."))
	fix_claim in rule_ids(halts)
}

test_halt_on_the_fix_works if {
	halts := guard.halt with input as stop_event(fix_facts("The fix works."))
	fix_claim in rule_ids(halts)
}

test_halt_on_a_bare_past_tense_report if {
	halts := guard.halt with input as stop_event(fix_facts("Fixed the ordering in the row cloner."))
	fix_claim in rule_ids(halts)
}

# --- one case per exemption ----------------------------------------------------------------------
# A run artifact was opened after the edit. This is the whole point of the rule: reading what the
# run wrote is what the word costs.
test_no_halt_when_a_run_artifact_was_read if {
	count(guard.halt) == 0 with input as stop_event(fix_facts_with(
		"The fix works: er-quit-menu.log shows the row on both loads.",
		"evidence",
		"1",
	))
}

# An honest hedge is the behaviour being asked for and must never be punished.
test_no_halt_when_hedged if {
	count(guard.halt) == 0 with input as stop_event(fix_facts_with(
		"That is the fix, unverified -- nothing has run yet.",
		"hedged",
		"1",
	))
}

# A dependency the agent cannot dissolve by working harder.
test_no_halt_when_blocked_externally if {
	count(guard.halt) == 0 with input as stop_event(fix_facts_with(
		"That is the real fix, which I have left for you to call.",
		"blocked",
		"1",
	))
}

# The sentence says a gate was fixed and names nothing inside the game. A green gate is the proof of
# host work, and demanding a game launch for a clippy lint is how a guard earns a reputation for
# being wrong.
test_no_halt_when_the_object_is_host_machinery if {
	count(guard.halt) == 0 with input as stop_event(fix_facts_with(
		"The integration gate came back red and I have fixed all four failures.",
		"hostobject",
		"1",
	))
}

# The turn edited nothing that can reach a DLL: scripts, docs and policies are outside the rule
# because a run cannot show any of them working or failing.
test_no_halt_when_no_game_loaded_code_changed if {
	count(guard.halt) == 0 with input as stop_event(replace(
		fix_facts("That is the fix for the audit script."),
		"changed=1",
		"changed=0",
	))
}

# --- shape of the guard itself --------------------------------------------------------------------
# A clean turn emits nothing, and nothing must halt.
test_no_halt_on_empty_signal if {
	count(guard.halt) == 0 with input as stop_event("")
}

# Stop-only: a PreToolUse carrying the same signal must not halt.
test_no_halt_on_other_event if {
	count(guard.halt) == 0 with input as {
		"hook_event_name": "PreToolUse",
		"signals": {"last_assistant_fix_claim_without_runtime_evidence": fix_facts("That is the real fix.")},
	}
}

# Both the bare-string and the {output: ...} shapes cupcake may hand back.
test_halt_on_object_shaped_signal if {
	halts := guard.halt with input as {
		"hook_event_name": "Stop",
		"signals": {"last_assistant_fix_claim_without_runtime_evidence": {
			"output": fix_facts("That is the real fix."),
			"exit_code": 0,
		}},
	}
	fix_claim in rule_ids(halts)
}

# The asymmetric defaults, which are where this rule parts company with its neighbours.
#
# A line that drops `changed` stays silent: that is the fact which makes the turn culpable, and a
# rule of this shape costs far more when it is wrong than when it is quiet.
test_missing_changed_field_stays_silent if {
	count(guard.halt) == 0 with input as stop_event("FIXFACTS|claim=That is the real fix.")
}

# A line that drops an exemption field still halts, so a crafted or degraded signal cannot buy
# silence by omission.
test_missing_exemption_fields_fail_closed if {
	halts := guard.halt with input as stop_event("FIXFACTS|claim=That is the real fix.|changed=1")
	fix_claim in rule_ids(halts)
}

# The offending sentence is quoted back, so the agent knows which sentence to make true or withdraw.
test_reason_quotes_the_claim if {
	halts := guard.halt with input as stop_event(fix_facts("That is the real fix."))
	some d in halts
	contains(d.reason, "That is the real fix.")
}

# The correction has to name what would settle it, or the agent is told only that it is wrong.
test_reason_names_the_evidence_that_would_settle_it if {
	halts := guard.halt with input as stop_event(fix_facts("That is the real fix."))
	some d in halts
	contains(d.reason, "ds2-loader.log")
}

# And it has to say that withdrawing the word is a pass, or the rule teaches agents to claim less
# carefully rather than to hedge honestly.
test_reason_offers_the_hedge_as_a_pass if {
	halts := guard.halt with input as stop_event(fix_facts("That is the real fix."))
	some d in halts
	contains(d.reason, "An honest hedge passes this guard by design")
}

# --- the 2026-09-13 escape ------------------------------------------------------------------------
# The turn edited three feature-gate predicates in crates/er-quickload, cross-compiled, launched the
# game, and closed in the same message as the launch -- before the process had written a line -- on
# "Fixed and relaunched as 07f2729b". Nothing halted. The user: "Does the word 'fix' to you mean that
# its proven?"
#
# Where the miss actually was, measured against that transcript rather than reasoned about: the
# conjunction below was already correct and the claim was already detected. The facts line the signal
# produced read `evidence=1`, because the turn's last tool call was a `Monitor` armed on
# `tail -F er-quickload-autoload-debug.log` -- a filename in a tool input, on a log the just-started
# process had not written to. The allowlist in scripts/cupcake_fix_claim.py matched the name and
# called a subscription a read. So the fix is in the classifier, and these cases pin what the policy
# must do once the classifier stops lying to it. The claim strings are verbatim from the transcript,
# after the scrub that drops backticked spans.
#
# The other half -- that a real transcript of this shape now produces `evidence=0` -- is pinned in
# scripts/test-fix-claim-classifier.py and end to end in .cupcake/tests/fixtures/fix_claim_watcher_armed.jsonl,
# because a Rego test cannot see a regex in a shell signal and a policy green on hand-typed facts
# says nothing about whether a transcript ever produces them.
test_halt_on_the_2026_09_13_escape if {
	halts := guard.halt with input as stop_event(fix_facts("Fixed and relaunched as : , and now each take a term, so this composition should give you the logo, audible title music and a live online-mode getter"))
	fix_claim in rule_ids(halts)
}

# A watcher armed on a log is the closing sentence of that same turn, and it must not be read as
# evidence or as a hedge. At this layer that means only one thing: the signal hands over
# `evidence=0`, and the halt still lands.
test_halt_when_the_only_watcher_was_armed_not_read if {
	halts := guard.halt with input as stop_event(fix_facts("Fixed and relaunched as 07f2729b. The monitor will tell me if splash-skip: patched appears anyway."))
	fix_claim in rule_ids(halts)
}

# --- the synonyms from the same session ----------------------------------------------------------
# Three more closing shapes that session produced, none of which any branch read before 2026-09-13.
test_halt_on_solved_and_user_confirmed if {
	halts := guard.halt with input as stop_event(fix_facts("Solved and user-confirmed: the orphan title window is reaped on the first frame after the dialog closes."))
	fix_claim in rule_ids(halts)
}

# A user confirming something is still not a measurement this turn read. The honest form of it cites
# what was measured, or hedges -- both of which pass below.
test_halt_on_the_mechanism_named_as_the_answer if {
	halts := guard.halt with input as stop_event(fix_facts("So the fix is , which is what the native path uses for a dialog dismissed without a selection."))
	fix_claim in rule_ids(halts)
}

test_halt_on_an_edit_asserted_to_have_produced_an_effect if {
	halts := guard.halt with input as stop_event(fix_facts("This single edit restored BOTH the chrome and the six-cell Quit grid."))
	fix_claim in rule_ids(halts)
}

# --- honest reporting, which must never halt -----------------------------------------------------
# A guard that fires on every sentence containing the word is worked around within a day and is worse
# than nothing. Each case below is a shape the classifier resolves to a facts line that cannot halt,
# and each is written the way the signal would actually emit it.
#
# Reporting a measurement with its numbers. The classifier finds no claim at all here, so the signal
# emits nothing -- the empty line, which is what a clean turn looks like.
test_no_halt_on_a_measured_result if {
	count(guard.halt) == 0 with input as stop_event("")
}

# The word about a plan rather than a change. Same shape: the infinitive in "the fix is to gate it"
# is what separates a plan from a claim, so no claim is emitted.
test_no_halt_on_the_fix_is_to_do_something if {
	count(guard.halt) == 0 with input as stop_event("")
}

# A claim that names what would settle it. The claim IS emitted here -- the sentence says "That is
# the fix" -- and the hedge column is what clears it, which is the behaviour the correction asks for.
test_no_halt_when_the_message_names_what_would_prove_it if {
	count(guard.halt) == 0 with input as stop_event(fix_facts_with(
		"That is the fix. What would prove it is er-quickload-autoload-debug.log with no splash-skip: patched line.",
		"hedged",
		"1",
	))
}

# "changed" and "attempted" are the plain words this guard exists to make attractive.
test_no_halt_on_changed_rather_than_fixed if {
	count(guard.halt) == 0 with input as stop_event(fix_facts_with(
		"Attempted the gate change; the next run would show whether the logo comes back.",
		"hedged",
		"1",
	))
}

# A merge conflict is host work, and the run cannot speak about it either way. Needed by the
# `resolved` vocabulary added on 2026-09-13: without the host column this sentence would convict on
# the filename it names.
test_no_halt_on_a_merge_conflict_in_a_game_file if {
	count(guard.halt) == 0 with input as stop_event(fix_facts_with(
		"Resolved the conflict in rows.rs and kept both hunks.",
		"hostobject",
		"1",
	))
}

# And the case the whole rule is for: the same escape sentence, once the log has actually been read.
test_no_halt_on_the_escape_sentence_with_the_log_read if {
	count(guard.halt) == 0 with input as stop_event(fix_facts_with(
		"Fixed and relaunched as 07f2729b: er-quickload-autoload-debug.log carries no splash-skip: patched line across the boot.",
		"evidence",
		"1",
	))
}
