# OPA unit tests for no_authority_agreement (the Stop-event halt on banned authority-coded agreement).
# Run with:
#   opa test .cupcake/system/commands.rego \
#     .cupcake/policies/claude/no_authority_agreement.rego \
#     .cupcake/tests/no_authority_agreement_test.rego
package cupcake.policies.claude.no_authority_agreement_test

import rego.v1

import data.cupcake.policies.claude.no_authority_agreement as guard

stop_event(sig) := {
	"hook_event_name": "Stop",
	"signals": {"last_assistant_authority_agreement": sig},
}

stop_event_object_signal(sig) := {
	"hook_event_name": "Stop",
	"signals": {"last_assistant_authority_agreement": {"output": sig, "exit_code": 0}},
}

rule_ids(halts) := {d.rule_id | some d in halts}

# A banned phrase in the last message halts the turn-end (string-shaped signal).
# Bare/untagged value (backward compat with older/crafted signal shapes) is treated as Category A.
test_halt_on_banned_phrase_string_signal if {
	halts := guard.halt with input as stop_event("You're right")
	"DS2-MODS-NO-AUTHORITY-AGREEMENT" in rule_ids(halts)
}

# Category A tagged value (AUTH:<phrase>) halts.
test_halt_on_auth_tagged_signal if {
	halts := guard.halt with input as stop_event("AUTH:you're right")
	"DS2-MODS-NO-AUTHORITY-AGREEMENT" in rule_ids(halts)
}

# Category B tagged value (ACK:<phrase>) halts.
test_halt_on_ack_tagged_signal if {
	halts := guard.halt with input as stop_event("ACK:Point taken")
	"DS2-MODS-NO-AUTHORITY-AGREEMENT" in rule_ids(halts)
}

# The remedy is to drop the prose, never to record a memory. This asserted the opposite until
# 2026-09-20, when the user removed the beads-memory exception: the escape hatch made a memory the
# price of replying to a correction, so corrections became memories instead of behaviour.
test_ack_reason_does_not_ask_for_a_memory if {
	halts := guard.halt with input as stop_event("ACK:Point taken")
	some d in halts
	not contains(d.reason, "beads memory")
	not contains(d.reason, "bd remember")
}

# The same holds for the authority-coded case, whose correction text also used to demand one.
test_auth_reason_does_not_ask_for_a_memory if {
	halts := guard.halt with input as stop_event("AUTH:you're right")
	some d in halts
	not contains(d.reason, "bd memory")
	not contains(d.reason, "bd remember")
}

# Object-shaped signal ({output: ...}) is handled too.
test_halt_on_banned_phrase_object_signal if {
	halts := guard.halt with input as stop_event_object_signal("Exactly,")
	"DS2-MODS-NO-AUTHORITY-AGREEMENT" in rule_ids(halts)
}

# No banned phrase -> no halt (empty signal).
test_no_halt_on_clean_message if {
	halts := guard.halt with input as stop_event("")
	count(halts) == 0
}

# Whitespace-only signal is treated as clean.
test_no_halt_on_whitespace_signal if {
	halts := guard.halt with input as stop_event("   \n")
	count(halts) == 0
}

# The halt only applies to Stop events, not other events that might carry the signal.
test_no_halt_on_non_stop_event if {
	halts := guard.halt with input as {
		"hook_event_name": "PreToolUse",
		"signals": {"last_assistant_authority_agreement": "You're right"},
	}
	count(halts) == 0
}
