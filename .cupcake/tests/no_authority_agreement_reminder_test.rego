# OPA unit tests for no_authority_agreement_reminder (the UserPromptSubmit reminder + interlock
# backstop that catches an interrupted-turn slip the Stop halt could not).
# Run with:
#   opa test .cupcake/system/commands.rego \
#     .cupcake/policies/claude/no_authority_agreement_reminder.rego \
#     .cupcake/tests/no_authority_agreement_reminder_test.rego
package cupcake.policies.claude.no_authority_agreement_reminder_test

import rego.v1

import data.cupcake.policies.claude.no_authority_agreement_reminder as guard

ups_event(sig) := {
	"hook_event_name": "UserPromptSubmit",
	"signals": {"last_assistant_authority_agreement": sig},
}

has_interlock(ctxs) if {
	some c in ctxs
	startswith(c, "INTERLOCK TRIPPED:")
}

has_standing_reminder(ctxs) if {
	some c in ctxs
	startswith(c, "BANNED PHRASING:")
}

# The standing reminder is injected on every UserPromptSubmit, even with a clean prior turn.
test_standing_reminder_always_present if {
	ctxs := guard.add_context with input as ups_event("")
	has_standing_reminder(ctxs)
}

# Clean prior turn -> no interlock directive (only the standing reminder).
test_no_interlock_on_clean_prior_turn if {
	ctxs := guard.add_context with input as ups_event("")
	not has_interlock(ctxs)
}

# A prior-turn slip (bare/untagged string signal) trips the interlock backstop on the next prompt.
test_interlock_on_prior_slip_string_signal if {
	ctxs := guard.add_context with input as ups_event("You're right")
	has_interlock(ctxs)
}

# Category A tagged value (AUTH:<phrase>) trips the interlock.
test_interlock_on_auth_tagged_signal if {
	ctxs := guard.add_context with input as ups_event("AUTH:you're right")
	has_interlock(ctxs)
}

# Category B tagged value (ACK:<phrase>) trips the interlock.
test_interlock_on_ack_tagged_signal if {
	ctxs := guard.add_context with input as ups_event("ACK:Point taken")
	has_interlock(ctxs)
}

# The standing reminder must not tell the agent to record a memory. The exception that made one the
# price of replying to a correction was removed by user directive 2026-09-20, and a reminder that
# still named `bd remember` would keep asking for it every turn.
test_standing_reminder_does_not_ask_for_a_memory if {
	ctxs := guard.add_context with input as ups_event("")
	every ctx in ctxs {
		not contains(ctx, "bd remember")
	}
}

# Object-shaped signal ({output: ...}) trips the interlock too.
test_interlock_on_prior_slip_object_signal if {
	ctxs := guard.add_context with input as {
		"hook_event_name": "UserPromptSubmit",
		"signals": {"last_assistant_authority_agreement": {"output": "Exactly,", "exit_code": 0}},
	}
	has_interlock(ctxs)
}

# Whitespace-only signal is treated as clean -> no interlock.
test_no_interlock_on_whitespace_signal if {
	ctxs := guard.add_context with input as ups_event("   \n")
	not has_interlock(ctxs)
}

# The interlock only applies to UserPromptSubmit, not other events carrying the signal.
test_no_interlock_on_non_ups_event if {
	ctxs := guard.add_context with input as {
		"hook_event_name": "Stop",
		"signals": {"last_assistant_authority_agreement": "You're right"},
	}
	count(ctxs) == 0
}
