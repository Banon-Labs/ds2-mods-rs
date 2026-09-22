# METADATA
# scope: package
# title: Green is a claim about CI, and CI is measurable -- so measure it before saying it
# authors: ["er-mods-rs agents", "ds2-mods-rs agents"]
# custom:
#   severity: HIGH
#   id: DS2-MODS-NO-FALSE-CI-GREEN
#   description: >-
#     Puts the MEASURED CI state of the current branch's pull request into context before every
#     answer, so "green" / "passing" / "CI is clean" cannot be written from a local gate result.
#
#     MEASURED 2026-09-03. An answer described PR #384 as "pushed, green, and now carries the
#     retraction". At that moment `gh pr checks 384` read `check  pending`. The word came from a
#     local `scripts/check.sh` that had exited 0 minutes earlier and silently became a claim about
#     GitHub. The user's objection is the rule: "saying it is 'green' while CI is not passing on the
#     latest push is objectively false."
#
#     WHY THIS IS NOT A STYLE NOTE. "Green" is the word a reviewer acts on -- it is the difference
#     between merging and waiting. A local gate and CI are different job sets, and a freshly pushed
#     commit routinely has NOTHING run against it yet, so the local result is not even weak evidence
#     for the remote one: it is evidence about a different question.
#
#     SHAPE: UserPromptSubmit context, like `wall_of_text`. A Stop halt cannot work here for the
#     reasons that policy documents at length -- its verdict is rendered to the user verbatim, and
#     it fires only after the false claim has already been read. Injecting the measurement BEFORE
#     the answer means the true state is in context at the moment the sentence is written, which is
#     the only point where the error is preventable rather than correctable.
#
#     The signal fails open: no gh, no repo, or no PR emits nothing, and nothing is asserted. An
#     ABSENT signal must never be read as "CI is fine" -- it means the question was not answered.
#   routing:
#     required_events: ["UserPromptSubmit"]
#     required_signals: ["ci_state_for_branch"]
package cupcake.policies.claude.no_false_ci_green

import rego.v1

# The measurement, stated plainly, whatever the verdict. Present on every prompt so the words are
# never written from memory of a local run.
add_context contains context if {
	input.hook_event_name == "UserPromptSubmit"
	some s in [state]
	context := context_for(s)
}

# PASS is the only verdict that licenses the word, and even then only about CI -- not about the
# change being correct, reviewed, or safe to merge.
context_for(s) := msg if {
	s.verdict == "PASS"
	msg := concat("", ["CI STATE (measured, not remembered): PR #", s.number, " on `", s.branch, "` is PASSING -- ", s.summary, ". You may say CI is green. That word covers CI ONLY: it says nothing about whether the change is correct, reviewed, or safe to merge."])
}

context_for(s) := msg if {
	s.verdict == "PENDING"
	msg := concat("", ["CI STATE (measured, not remembered): PR #", s.number, " on `", s.branch, "` is PENDING -- ", s.summary, ". DO NOT write \"green\", \"passing\", \"CI is clean\" or any equivalent: nothing has finished. A local `scripts/check.sh` exit 0 is NOT this measurement -- different job set, and a fresh push often has nothing run against it yet. Say \"CI is still running\" and name the job."])
}

context_for(s) := msg if {
	s.verdict == "FAIL"
	msg := concat("", ["CI STATE (measured, not remembered): PR #", s.number, " on `", s.branch, "` is FAILING -- ", s.summary, ". Lead with the failure, name the failing job, and do not describe the branch as ready, green or mergeable. A passing local gate does not soften this; it means the local gate does not cover what CI does."])
}

context_for(s) := msg if {
	s.verdict == "NOPR"
	msg := concat("", ["CI STATE (measured): `", s.branch, "` has NO pull request, so there is no CI verdict to report. Do not describe the branch as green or passing on the strength of a local gate."])
}

context_for(s) := msg if {
	s.verdict == "UNKNOWN"
	msg := concat("", ["CI STATE: could not be measured for `", s.branch, "` (", s.summary, "). An unmeasured verdict is NOT a passing one -- say CI state is unknown rather than inferring it from a local gate."])
}

# Parse CISTATE:<branch>:<number>:<verdict>:<summary>. The summary may itself contain ":", so it is
# rejoined rather than taken as one field. An untagged or short value yields no `state`, so the
# policy contributes nothing rather than inventing a verdict -- the same no-fabrication contract
# `wall_of_text` uses.
state := s if {
	startswith(raw, "CISTATE:")
	parts := split(raw, ":")
	count(parts) >= 5
	s := {
		"branch": parts[1],
		"number": parts[2],
		"verdict": parts[3],
		"summary": concat(":", array.slice(parts, 4, count(parts))),
	}
}

raw := trim(matched, " \t\r\n")

matched := s if {
	s := input.signals.ci_state_for_branch
	is_string(s)
} else := s if {
	s := input.signals.ci_state_for_branch.output
} else := ""
