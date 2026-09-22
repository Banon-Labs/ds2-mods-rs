# METADATA
# scope: package
# title: Ban authority-coded agreement + unbacked feedback-acknowledgement prose
# authors: ["er-mods-rs agents", "ds2-mods-rs agents"]
# custom:
#   severity: HIGH
#   id: DS2-MODS-NO-AUTHORITY-AGREEMENT
#   description: >-
#     User directives 2026-07-17, amended 2026-09-20. Category A -- authority-coded agreement
#     ("You're right", "Correct,", "Exactly,", "Absolutely,", "That's right") -- is banned OUTRIGHT.
#     Category B -- feedback-acknowledgement / receipt-announcement prose ("Point taken", "Got it",
#     "Understood", "Noted", "Fair point", "Makes sense", ...) -- is banned outright too. The signal
#     returns AUTH:<phrase> and ACK:<phrase>; both HALT turn-end so the agent must correct. rego cannot
#     pre-filter prose (no pre-response hook), so this is reinforce-every-turn + halt-and-correct.
#
#     Category B carried an exception until 2026-09-20: the prose was allowed when the same turn
#     recorded a beads memory. User directive removed it, because it made a memory the price of
#     replying to a correction -- so corrections became memories instead of behaviour, and the store
#     grew by one per slip. The correct response to a correction is to apply it and say nothing about
#     having received it.
#   routing:
#     required_events: ["Stop"]
#     required_signals: ["last_assistant_authority_agreement"]
package cupcake.policies.claude.no_authority_agreement

import rego.v1

# Enforcement: block turn-end when the just-emitted assistant turn used banned phrasing.
# (The every-turn reminder lives in no_authority_agreement_reminder.rego -- cupcake forbids a
# Stop-routed policy from also emitting add_context.)
halt contains decision if {
	input.hook_event_name == "Stop"
	some h in [hit]
	decision := {
		"rule_id": "DS2-MODS-NO-AUTHORITY-AGREEMENT",
		"reason": reason_for(h),
		"severity": "HIGH",
	}
}

# Correction directive per case.
reason_for(h) := msg if {
	h.case == "AUTH"
	msg := concat("", ["Banned authority-coded agreement detected in your reply: '", h.phrase, "'. Per the 2026-07-17 directive this phrasing is forbidden. Send a corrected reply that removes the phrase and instead states the verified fact plus its proof (or simply proceeds)."])
}

reason_for(h) := msg if {
	h.case == "ACK"
	msg := concat("", ["Banned feedback-acknowledgement prose detected in your reply: '", h.phrase, "'. Announcing that you received or internalized feedback tells the user nothing they cannot see from what you do next. Send a corrected reply that drops the receipt-prose and simply proceeds with the corrected behaviour."])
}

# Parse the tagged signal into {case, phrase}. Untagged-but-non-empty falls back to a Category-A hit
# so a bare/crafted "You're right" value still halts. Empty -> hit undefined -> no halt.
hit := h if {
	startswith(raw, "AUTH:")
	h := {"case": "AUTH", "phrase": trim(trim_prefix(raw, "AUTH:"), " \t\r\n")}
} else := h if {
	startswith(raw, "ACK:")
	h := {"case": "ACK", "phrase": trim(trim_prefix(raw, "ACK:"), " \t\r\n")}
} else := h if {
	raw != ""
	h := {"case": "AUTH", "phrase": raw}
}

raw := trim(matched_phrase, " \t\r\n")

# Signal value tolerates both the bare-string and {output: ...} shapes cupcake may hand back.
matched_phrase := p if {
	p := input.signals.last_assistant_authority_agreement
	is_string(p)
} else := p if {
	p := input.signals.last_assistant_authority_agreement.output
} else := ""
