# METADATA
# scope: package
# title: Ban ending a turn on prose that shouts
# authors: ["ds2-mods-rs agents"]
# custom:
#   severity: HIGH
#   id: DS2-MODS-NO-SHOUTING-AT-TURN-END
#   description: >-
#     User directive 2026-09-23, written in the style it is complaining about: "When I TALK LIKE
#     THIS everyone thinks the WORDS ARE IMPORTANT but really it distracts from the SUBSTANCE OF
#     THE MESSAGE which is burried by font style and EXCESSIVE PROSE that is supposed to be caught
#     by a REGO POLICY that prevents you from writing in ALL CAPS."
#
#     The habit is the same one docs_no_shouting refuses on the way to disk, arriving instead in
#     the agent's closing prose, where it does more damage: the closing line is the one part of a
#     turn the user actually reads, and a shouted clause in it is skimmed exactly like a wall of
#     text is skimmed. Nothing inside a capitalised phrase says which part of it is the point, so
#     the reader stops looking for one, and the sentence that was supposed to be rescued goes down
#     with it.
#
#     The offence is measured by scripts/cupcake_shouting.py, which owns the two patterns and the
#     verbatim-span stripping so that this policy, its unit test, the PreToolUse arm and any
#     false-positive audit cannot drift into four different answers. Both patterns were tuned by
#     sweeping this repo's own documentation rather than guessed, and that working is recorded in
#     the module; the short version is four capitalised words in a row, or one capitalised word
#     from a closed list of function words that could never be a name.
#
#     What the guard does not see, by construction. Only the turn's FINAL prose run is read
#     (cupcake_turn_scan reads text_runs[-1]), so a shout mid-turn with work after it never reaches
#     this rule. That is what keeps it clear of the launch banner AGENTS.md requires: the banner
#     has to sit immediately before the launch call, which puts it mid-turn, and a banner this
#     guard can see has already been written in the wrong place. Backticked spans, fenced blocks
#     and quoted text are removed before judging, so a log line, a menu string, an identifier or
#     the user's own words quoted back are never the offence.
#
#     The signal emits one facts line -- SHOUTING|score=<n>|sample=<the loudest line> -- so the
#     OBSERVATION lives in the shell and the RULE lives here, where it is unit-testable. The score
#     counts one for an ordinary capitalised word and the whole threshold for a function word, so a
#     single number carries both halves of the test and can be re-applied here rather than trusted.
#     Empty signal means no halt.
#
#     Known gap, the same one every Stop guard here has: an interrupted turn fires no Stop event,
#     so a shouted turn the user cuts short is not caught. The sibling pairs (no_authority_agreement
#     + _reminder, idle_hold + _reminder) close that with a UserPromptSubmit interlock reading the
#     same signal; add one the same way if the gap bites.
#   routing:
#     required_events: ["Stop"]
#     required_signals: ["last_assistant_shouting"]
package cupcake.policies.claude.no_shouting_at_turn_end

import rego.v1

# Enforcement: block turn-end when the closing prose puts its emphasis in the capitalisation.
halt contains decision if {
	input.hook_event_name == "Stop"
	shouting_at_turn_end
	decision := {
		"rule_id": "DS2-MODS-NO-SHOUTING-AT-TURN-END",
		"reason": reason,
		"severity": "HIGH",
	}
}

shouting_at_turn_end if {
	to_number(score) >= min_score
}

# Mirrors MIN_RUN in scripts/cupcake_shouting.py, and deliberately the same number in two places:
# the signal uses it to decide whether to speak at all, and this uses it to decide whether to
# believe what it said. Four ordinary capitalised words in a row reach it; so does one word from
# the function-word list on its own, because that word is scored at the threshold.
min_score := 4

reason := msg if {
	msg := concat("", [
		"You ended the turn on prose that shouts: '", sample,
		"'. Capitals used as emphasis are emphasis by volume, and the closing line is the one part of the turn the user actually reads. Nothing inside a capitalised phrase says which part of it is the point, so it gets skimmed exactly like a wall of text and the sentence it was meant to rescue goes with it. The user's words, 2026-09-23: writing like this \"distracts from the substance of the message, which is buried by font style\".",
		"\n\nRewrite that line and then stop. Put the emphasis in the sentence's structure rather than in its capitalisation: lead with the thing that matters, or give it its own short paragraph, and let the position carry the weight. To be exact about what is banned, because the two are opposites: a NAME in capitals was never the offence -- an acronym, an identifier, a register, a hex constant and a menu string are all fine, and anything in backticks or quotation marks is not even read. Capitalising 'the', 'not', 'every' or four words of an ordinary sentence is, because that is volume standing in for structure.",
	])
}

# --- signal parsing ------------------------------------------------------------------------------
# SHOUTING|score=<n>|sample=<the loudest line>
#
# The sample is split off before anything else is parsed, and this is not tidiness. A closing line
# can be a markdown table row, so the sample can contain the `|` that separates the fields; feeding
# it to the field split would drop the sample from the reason, and a cell containing an `=` would
# inject a junk key into the fact map (two such keys conflicting is an eval error, which in the WASM
# runtime is an undefined decision, which is a silent allow). So the head of the line is parsed as
# fields and everything after the marker is taken verbatim as the sample.
sample_marker := "|sample="

sample_at := indexof(raw, sample_marker)

head := h if {
	sample_at >= 0
	h := substring(raw, 0, sample_at)
} else := raw

sample := s if {
	sample_at >= 0
	s := trim(substring(raw, sample_at + count(sample_marker), -1), " \t\r\n")
} else := ""

# The leading tag carries no "=" so it drops out of the fact map on its own.
fact[k] := v if {
	some kv in split(head, "|")
	n := indexof(kv, "=")
	n > 0
	k := trim(substring(kv, 0, n), " \t\r\n")
	v := trim(substring(kv, n + 1, -1), " \t\r\n")
}

# The trigger. Absent or unparseable means `to_number` is undefined and the rule does not fire --
# the same contract `hits` has in no_status_table_at_turn_end: with no measurement there is nothing
# to quote, and a guessed number in the correction would be a fabricated fact. There is no
# exemption field to fail closed on here, because this guard has no exemptions: a shouted closing
# line is the offence whatever the turn was about.
score := object.get(fact, "score", "")

raw := trim(matched_facts, " \t\r\n")

# Signal value tolerates both the bare-string and {output: ...} shapes cupcake may hand back.
matched_facts := p if {
	p := input.signals.last_assistant_shouting
	is_string(p)
} else := p if {
	p := input.signals.last_assistant_shouting.output
} else := ""
