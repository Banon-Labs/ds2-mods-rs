# METADATA
# scope: package
# title: A user claim relayed to another agent says whether it was verified
# authors: ["ds2-mods-rs agents"]
# custom:
#   severity: HIGH
#   id: DS2-MODS-NO-UNMARKED-USER-CLAIM-RELAY
#   description: >-
#     Deny an Agent/Task `prompt` or a SendMessage `message` that relays a
#     factual claim attributed to the user ("the user says", "user (2026-10-30)
#     says", "per the user", "the user reports") with no status marker in that
#     sentence or the next one. The marker says either that the claim was
#     verified, with its source ("verified: <source>", "measured in <log>",
#     "confirmed by <disassembly>"), or that it was not ("unverified",
#     "hypothesis", "to verify"). History: on 2026-10-30 the orchestrator sent a
#     subagent "Correction to the filter: keep the Estus Flask in invader kits.
#     The user (2026-10-30) says vanilla disables Estus for invaders and
#     Seamless re-enables it. ... So the rule is: drop items ... except Estus,
#     which Seamless exempts." Nobody had verified the Seamless half, and the
#     subagent built on it as settled fact. The rule against exactly that
#     ("mark every claim in a subagent prompt as either verified, with its
#     source, or as a hypothesis for the subagent to check") was already in
#     /home/banon/AGENTS.md and was broken within the hour of being written, so
#     it is enforced here instead. A relayed decision or preference ("the user
#     decided", "the user wants", "the user says to keep X", "the user's rule")
#     is an instruction, not a claim about the world, and passes.
#   routing:
#     required_events: ["PreToolUse"]
#     required_tools: ["Agent", "Task", "SendMessage"]
package cupcake.policies.claude.no_unmarked_user_claim_relay

import rego.v1

# The text this call hands to another agent. Agent (and its older name, Task) carries it in
# `prompt`; SendMessage carries it in `message`. Any other tool has none, and is never judged.
tool_input := object.get(input, "tool_input", {})

relay_text := object.get(tool_input, "prompt", "") if {
	input.tool_name in {"Agent", "Task"}
}

relay_text := object.get(tool_input, "message", "") if {
	input.tool_name == "SendMessage"
}

# Quoted and backticked spans are removed before anything is judged. A prompt that quotes a
# message containing "the user says" -- the task that wrote this guard did exactly that, quoting the
# Estus message -- is talking about the sentence, not relaying the claim in it. The attribution
# itself has to sit outside the quotes to count, so `The user said "X"` is still judged.
#
# Splitting on the quote character and keeping the even-numbered pieces is the outside of every
# quoted span, using only builtins the WASM runtime is proven to run (no regex replace). A stray
# unbalanced quote hides the text after it; that is a known gap, not a design choice.
straight_quoted := replace(replace(relay_text, "“", "\""), "”", "\"")

outside_double_quotes := concat(" ", [piece |
	some i, piece in split(straight_quoted, "\"")
	i % 2 == 0
])

unquoted := concat(" ", [piece |
	some i, piece in split(outside_double_quotes, "`")
	i % 2 == 0
])

# A sentence ends at `.`, `!` or `?` followed by whitespace or the end of the line. A period inside
# a word (ds2-run.py, 0x1.5, e.g) does not end one, and neither does a date. Lines are split first so
# a bullet list reads as one sentence per bullet.
sentence_pattern := `(?:[^.!?]|[.!?]+[^\s.!?])+[.!?]*`

# regex.find_n would read more plainly here and does not run: scripts/check-cupcake-wasm-builtins.py
# measured it returning undefined in cupcake's WASM runtime on 2026-10-30, which would have left
# this policy allowing everything with a green `opa test`. The submatch form runs.
sentences := [m[0] |
	some line in split(unquoted, "\n")
	some m in regex.find_all_string_submatch_n(sentence_pattern, line, -1)
]

# The attribution: the user, an optional parenthetical (a date, "on the 30th"), optional filler
# adverbs, and a verb that reports something the user said, saw or believes. Group 1 is the rest of
# the sentence after the verb, which is what separates a claim from a relayed instruction.
attribution_pattern := `(?i)\b(?:the\s+)?user(?:\s*\([^)]*\))?(?:\s+(?:also|now|just|earlier|previously|originally|already|has|had|explicitly|clearly|then|first))*\s+(?:says|said|reports|reported|claims|claimed|states|stated|tells\s+me|told\s+me|tells\s+us|told\s+us|mentions|mentioned|notes|noted|believes|believed|thinks|thought|recalls|recalled|remembers|remembered|observes|observed|saw|sees|confirms|confirmed|insists|insisted|suspects|suspected|explains|explained|points\s+out|pointed\s+out|asserts|asserted|swears|swore)\b(.*)$`

# Attributions with no reporting verb: the claim follows the phrase.
attribution_prefix_pattern := `(?i)\b(?:per|according\s+to)\s+(?:the\s+)?user\b|\bthe\s+user's\s+(?:claim|report|observation|account|recollection|understanding|reading|word)\b`

# What follows the verb when the user is being relayed as giving an instruction or stating a
# preference rather than a fact: "says to keep", "said not to", "says we should", "said they want".
directive_tail_pattern := `(?i)^[\s,:]*(?:(?:not\s+)?to\s+\w|(?:that\s+)?(?:(?:we|you|i|they|he|she|it)\s+)?(?:should|must|shall|need\s+to|needs\s+to|have\s+to|has\s+to|ought\s+to|may\s+not|can't|cannot)\b|(?:that\s+)?(?:they|he|she)\s+(?:want|wants|wanted|prefer|prefers|preferred|decided|chose|choose|would\s+like|'d\s+like|like|likes|asked|approve|approves|approved|don't\s+want|doesn't\s+want)\b|(?:yes|no|ok|okay|go\s+ahead|proceed|stop)\b)`

# The status marker. Unverified side: the claim is passed on as something to check. Verified side:
# the claim was checked, and the marker names how ("verified: <source>", "measured in <log>",
# "confirmed by <disassembly>"). A bare "verified" or "confirmed" is not a marker, because "the user
# confirmed it" is the very claim being relayed, not a check of it.
marker_pattern := `(?i)\bunverified\b|\bnot\s+(?:yet\s+)?(?:been\s+)?(?:verified|checked|confirmed)\b|\bunconfirmed\b|\bunchecked\b|\bunproven\b|\bhypothes[ie]s\b|\bto\s+(?:verify|check|confirm)\b|\bverify\s+(?:it|this|that|first|before)\b|\b(?:verified|measured|confirmed)\s*(?::|\s(?:by|in|against|with|from|via|at|on|using|through)\b)|\bsource\s*:`

claim_attribution(s) if {
	some m in regex.find_all_string_submatch_n(attribution_pattern, s, -1)
	not regex.match(directive_tail_pattern, m[1])
}

claim_attribution(s) if regex.match(attribution_prefix_pattern, s)

marked(i) if regex.match(marker_pattern, sentences[i])

marked(i) if regex.match(marker_pattern, sentences[i + 1])

unmarked_claims contains trim_space(sentences[i]) if {
	some i, s in sentences
	claim_attribution(s)
	not marked(i)
}

reason := concat("", [
	"🧁 Cupcake blocked a relayed user claim with no status marker. This text hands another agent a claim attributed to the user as if it were settled fact:\n\n    ",
	concat("\n    ", [c | some c in unmarked_claims]),
	"\n\nOn 2026-10-30 the orchestrator told a subagent \"The user (2026-10-30) says vanilla disables Estus for invaders and Seamless re-enables it\", nobody had checked the Seamless half, and the subagent built on it. The rule in /home/banon/AGENTS.md: mark every claim in a subagent prompt as verified, with its source, or as a hypothesis for the subagent to check.\n\n",
	"Fix: in the same sentence as the attribution, or the one right after it, add one of\n",
	"    (unverified -- verify before relying on it)\n",
	"    hypothesis to check: <how to check it>\n",
	"    verified: <source -- the disassembly, log line, param row or file that shows it>\n",
	"    measured in <run/log>\n",
	"If this is the user's decision or preference rather than a claim about the world, say so: \"the user decided\", \"the user wants\", \"the user says to <do X>\" all pass.",
])

deny contains decision if {
	input.hook_event_name == "PreToolUse"
	count(unmarked_claims) > 0

	decision := {
		"rule_id": "DS2-MODS-NO-UNMARKED-USER-CLAIM-RELAY",
		"severity": "HIGH",
		"reason": reason,
	}
}
