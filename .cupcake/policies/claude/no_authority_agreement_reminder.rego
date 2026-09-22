# METADATA
# scope: package
# title: Reinforce the agreement/acknowledgement ban every turn (and catch interrupted-turn slips)
# authors: ["er-mods-rs agents", "ds2-mods-rs agents"]
# custom:
#   severity: LOW
#   id: DS2-MODS-NO-AUTHORITY-AGREEMENT-REMINDER
#   description: >-
#     Companion to no_authority_agreement (the Stop-event halt). Two jobs on UserPromptSubmit:
#     (1) inject the ban into every turn's context so the phrasing is avoided pre-emptively; and
#     (2) INTERLOCK BACKSTOP -- if the just-finished assistant turn used banned phrasing (Category A
#     authority-coded agreement, or Category B feedback-acknowledgement) but the Stop halt did
#     not catch it (e.g. the user INTERRUPTED the turn, so no Stop event fired), inject a HIGH-priority
#     mandatory-correction directive on the next prompt. UserPromptSubmit always runs, even after an
#     interrupt, so this closes the hole a Stop-only guard leaves open.
#   routing:
#     required_events: ["UserPromptSubmit"]
#     required_signals: ["last_assistant_authority_agreement"]
package cupcake.policies.claude.no_authority_agreement_reminder

import rego.v1

# (1) Standing every-turn reminder.
add_context contains context if {
	input.hook_event_name == "UserPromptSubmit"
	context := "BANNED PHRASING: (A) never use authority-coded agreement -- \"You're right\", \"You're correct\", \"That's right\", or sentence-initial \"Correct,\"/\"Exactly,\"/\"Absolutely,\"/\"Precisely,\". (B) never emit feedback-acknowledgement / receipt-announcement prose -- \"Point taken\", \"Got it\", \"Understood\", \"Noted\", \"Fair point\", \"Makes sense\", etc. Both are banned outright. Do not open with agreement or a receipt to be agreeable, and do not announce that you have internalized a correction -- applying it is the acknowledgement. If the user's claim is verified against evidence in context, state the verified fact and its proof directly; if it is only plausible, say so and state what would prove it; otherwise just proceed. Turn-end is guarded: these halt the stop and force a correction."
}

# (2) Interlock backstop: the previous turn slipped and the Stop halt missed it (interrupted turn).
add_context contains context if {
	input.hook_event_name == "UserPromptSubmit"
	some h in [hit]
	context := interlock_for(h)
}

# Correction directive per case.
interlock_for(h) := msg if {
	h.case == "AUTH"
	msg := concat("", ["INTERLOCK TRIPPED: your PREVIOUS turn used the banned authority-coded agreement '", h.phrase, "' and it was not caught at turn-end (the turn was likely interrupted, so no Stop halt fired). Before addressing the new request, acknowledge the slip in one line WITHOUT repeating the banned phrasing and restate the point as the verified fact plus its proof (or simply proceed). Do not use authority-coded agreement again."])
}

interlock_for(h) := msg if {
	h.case == "ACK"
	msg := concat("", ["INTERLOCK TRIPPED: your PREVIOUS turn emitted feedback-acknowledgement prose '", h.phrase, "', and it was not caught at turn-end (the turn was likely interrupted, so no Stop halt fired). Announcing that you received a correction tells the user nothing they cannot see from what you do next. Address the new request without receipt-prose, and do not emit it again."])
}

# Parse the tagged signal into {case, phrase}. Untagged-but-non-empty falls back to a Category-A hit
# so a bare/crafted "You're right" value still trips the interlock. Empty -> hit undefined -> none.
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
