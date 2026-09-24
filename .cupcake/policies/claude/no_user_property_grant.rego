# METADATA
# scope: package
# title: Ban closing prose that hands the user permission over their own property
# authors: ["ds2-mods-rs agents"]
# custom:
#   severity: HIGH
#   id: DS2-MODS-NO-USER-PROPERTY-GRANT
#   description: >-
#     User directive 2026-09-23. The instance, verbatim, from a turn that had just finished
#     building a DLL and had not relaunched the game:
#
#         "the DLL is built but I have not relaunched, so your session is still yours to end when
#         you want the next run."
#
#     THE OBJECTION. The sentence hands the user a right they already hold. Nobody granted the
#     agent authority over when the user's own session ends, so there is nothing for the agent to
#     hand back -- the clause is ceremony wearing the costume of restraint. What belongs there
#     instead is the factual half only: "the DLL is built; I have not relaunched." No clause about
#     what is theirs.
#
#     WHAT IS BANNED IS THE SHAPE, not any single sentence. A closing sentence that casts the agent
#     as the one bestowing control over the user's own property -- their session, their save,
#     their decision, their time -- back to them. The tell is idiomatic: "yours to end", "up to
#     you", "feel free to", "whenever you want" are all stock phrases for granting permission, and
#     every one of them presupposes the speaker held the thing being granted. The agent never did.
#
#     WHAT SURVIVES. A plain statement of agent action ("I have not relaunched.") carries the same
#     information with none of the ceremony and is untouched. A direct question ("Do you want me
#     to relaunch?") asks rather than grants and is untouched. Quoting the banned phrasing to
#     explain it -- this file, a report about the guard -- is not committing it; see the
#     mention/use carve-out in scripts/cupcake_property_grant.py, copied from the sibling guard's
#     `quotable()` for the same reason. Ordinary possessive prose ("your save file", "your own
#     folder") names whose property something is without staging a handover, and the lexicon is
#     anchored on fixed idioms rather than a bare "your"/"yours" match precisely so this never
#     fires on it.
#
#     The signal emits ONE facts line -- PROPERTYGRANT|phrase=<the offending phrase> -- so the
#     OBSERVATION lives in the shell and the RULE lives here, where it is unit-testable. The
#     lexicon and the quoting carve-out live in scripts/cupcake_property_grant.py so the signal,
#     its unit test and any audit cannot drift into three different answers. Empty signal -> no
#     halt.
#
#     KNOWN GAP, the same one every Stop guard here has: an interrupted turn fires no Stop event,
#     so a grant the user cuts short is not caught.
#   routing:
#     required_events: ["Stop"]
#     required_signals: ["last_assistant_property_grant"]
package cupcake.policies.claude.no_user_property_grant

import rego.v1

# Enforcement: block turn-end when the closing prose grants the user something already theirs.
halt contains decision if {
	input.hook_event_name == "Stop"
	phrase != ""
	decision := {
		"rule_id": "DS2-MODS-NO-USER-PROPERTY-GRANT",
		"reason": reason,
		"severity": "HIGH",
	}
}

reason := msg if {
	msg := concat("", ["You closed by granting the user permission over something already theirs -- '", phrase, "' -- and nobody handed you authority over it to begin with. Say what you did and did not do instead, and drop the clause about what is theirs: 'the DLL is built; I have not relaunched' carries the same information with none of the ceremony."])
}

# --- signal parsing ------------------------------------------------------------------------------
# PROPERTYGRANT|phrase=<the offending phrase>
#
# The phrase is taken verbatim from after the marker rather than through a field split, for the
# same reason the status-table guard does it: a quoted phrase can contain the `|` and `=` the
# field parser splits on, and two conflicting keys in a fact map is an eval error, which in the
# WASM runtime is an undefined decision, which is a silent ALLOW.
phrase_marker := "|phrase="

phrase_at := indexof(raw, phrase_marker)

phrase := p if {
	phrase_at >= 0
	p := trim(substring(raw, phrase_at + count(phrase_marker), -1), " \t\r\n")
} else := ""

raw := trim(matched_facts, " \t\r\n")

# Signal value tolerates both the bare-string and {output: ...} shapes cupcake may hand back.
matched_facts := p if {
	p := input.signals.last_assistant_property_grant
	is_string(p)
} else := p if {
	p := input.signals.last_assistant_property_grant.output
} else := ""
