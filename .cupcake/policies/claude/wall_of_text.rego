# METADATA
# scope: package
# title: One paragraph of prose per turn, injected before the answer instead of scolded after it
# authors: ["er-mods-rs agents", "ds2-mods-rs agents"]
# custom:
#   severity: LOW
#   id: DS2-MODS-WALL-OF-TEXT
#   description: >-
#     User directive 2026-08-21, stated absolutely: "I'll never. Repeat: NEVER read more than a
#     single paragraph of response text from you. Everything else is a skim."
#     Every word past the first paragraph is not merely unwanted, it is UNREAD. That makes long
#     prose actively worse than short prose: the agent believes it has communicated when it has not,
#     and any caveat, blocker or correction buried past paragraph one has effectively been withheld
#     from the person who needed it.
#     Structure is NOT prose: fenced code, tables, lists, headings and captions are exempt in the
#     signal, because the objection is to READING, and those are scanned. One paragraph plus a table
#     is fine; two paragraphs are not.
#
#     THIS WAS A Stop HALT UNTIL 2026-08-22 AND THE HALT WAS THE WRONG SHAPE. The user, in their
#     words: "That stop hook is not working if 1) I see it and 2) it doesn't prevent you from spewing
#     information." Both halves are structural facts about the harness, verified against Claude Code
#     2.1.240, not tuning problems:
#
#       (1) NO Stop VERDICT CAN BE HIDDEN. A blocking `reason` is pushed into the
#           `stop_hook_summary` system message as `hookErrors` and rendered verbatim as
#           "Stop hook error: <reason>"; `hookSpecificOutput.additionalContext` on Stop is rendered
#           the same way as "Stop hook feedback: <text>"; `systemMessage` is documented as
#           user-facing. `suppressOutput` gates only a hook's plain stdout, never the summary. So the
#           user reads the scolding, always.
#       (2) Stop FIRES AFTER THE TEXT IS ALREADY ON SCREEN. The assistant message is streamed to the
#           terminal and only then does the Stop event run -- its input even carries
#           `last_assistant_message`. A halt cannot unsend the wall of text; it can only append a
#           rewrite. Net effect of the old guard: the user read the long version, then the scolding,
#           then the short version. Three times the reading, from a rule that exists to reduce it.
#
#     SO THE CORRECTION MOVED TO UserPromptSubmit, which has neither problem: its
#     `additionalContext` becomes a `hook_additional_context` attachment, which the REPL filters out
#     of the rendered message list (it sits in the hidden-attachment set alongside `todo_reminder`
#     and `output_style`), and it lands in context BEFORE the next answer is written. Invisible, and
#     preventative for the turn about to start rather than punitive about the turn already read.
#
#     WHAT THIS BUYS AND WHAT IT COSTS, said plainly: an injected reminder is weaker than a halt --
#     it cannot refuse anything. The trade is deliberate, because the halt's enforcement was paid for
#     entirely out of the user's reading budget, which is the one resource this rule protects. The
#     available escalation, if the reminder proves too weak, is the same `additionalContext` on
#     PreToolUse (also invisible, also already wired with matcher "*"), re-asserted mid-turn so the
#     rule is the last thing in context before the closing message. It is not taken here because it
#     costs a transcript scan on every tool call.
#   routing:
#     required_events: ["UserPromptSubmit"]
#     required_signals: ["last_assistant_wall_of_text", "last_assistant_prose_words"]
package cupcake.policies.claude.wall_of_text

import rego.v1

# (1) Standing every-turn rule, so the constraint is in context before the answer is composed.
add_context contains context if {
	input.hook_event_name == "UserPromptSubmit"
	context := "ONE PARAGRAPH. The user reads the first paragraph of your answer and skims the rest, so anything you put after it -- a caveat, a blocker, the thing that actually mattered -- was not read. Lead with the answer. If it will not fit in one paragraph, that is the signal to CUT content, not to add a paragraph: drop the reasoning nobody asked for, and put what remains in a table, a list or a code block, which are scanned rather than read and do not count. Mid-turn one-line narration between tool calls is fine and is not measured; this is about the message you close on."
}

# (2) Measured correction: the turn that just ended ran long, so name the number and the opener.
# Specific beats generic -- a standing note in the system prompt is exactly what failed before.
add_context contains context if {
	input.hook_event_name == "UserPromptSubmit"
	some h in [hit]
	context := correction_for(h)
}

correction_for(h) := msg if {
	msg := concat("", ["MEASURED: your PREVIOUS answer ran to ", h.count, " paragraphs of prose in one unbroken run, beginning '", h.opener, "'. The user read the first one and skimmed the rest, so paragraphs 2..", h.count, " were not delivered -- if anything load-bearing was in them, it did not reach them. Do not re-send it and do not apologise for it. Answer THIS prompt in a single paragraph, and if something from the unread part still matters, that is what belongs in the one paragraph you get."])
}

# (3) Measured correction: ONE paragraph, but too many words in it -- the case (2) is blind to.
#
# Paragraph count treats a 281-word single paragraph as clean. On 2026-09-03 exactly that shape
# carried four subjects and closed on a pronoun; the user replied "it", nothing in the sentence
# uniquely answered to it, and the agent could not resolve its own prose. Their diagnosis is the
# rule: "If a user says it, and you have more than one subject it could apply to, that is a problem
# with YOUR prose being overloaded." A long turn also forces any unambiguous reply to be long, which
# charges the user for the overload. So bound the words, not just the breaks.
add_context contains context if {
	input.hook_event_name == "UserPromptSubmit"
	not hit
	some w in [words_hit]
	context := words_correction_for(w)
}

words_correction_for(w) := msg if {
	msg := concat("", ["MEASURED: your PREVIOUS answer was one paragraph but ", w.words, " words of prose, beginning '", w.opener, "'. That length is how a turn ends up carrying several subjects at once, and then a plain reply like 'it' has no unique antecedent and YOU cannot resolve your own sentence -- which is your defect, not the user's brevity. Answer THIS prompt on ONE subject. If a second thing is genuinely live, put it in a table row or leave it for its own turn; never let it trail in prose where a pronoun can reach it. Do not re-send the long version and do not apologise for it."])
}

words_hit := w if {
	startswith(words_raw, "PROSEWORDS:")
	parts := split(words_raw, ":")
	count(parts) >= 3
	w := {"words": parts[1], "opener": concat(":", array.slice(parts, 2, count(parts)))}
}

words_raw := trim(words_matched, " \t\r\n")

words_matched := s if {
	s := input.signals.last_assistant_prose_words
	is_string(s)
} else := s if {
	s := input.signals.last_assistant_prose_words.output
} else := ""

# Parse the tagged signal into {count, opener}. Untagged-but-non-empty is treated as clean: unlike a
# banned phrase, a raw value here carries no measurement to quote, and a guessed number in the
# correction would be a fabricated fact. Empty -> hit undefined -> standing rule only. `words_hit`
# above follows the same contract for PROSEWORDS.
hit := h if {
	startswith(raw, "WALLOFTEXT:")
	parts := split(raw, ":")
	count(parts) >= 3
	h := {"count": parts[1], "opener": concat(":", array.slice(parts, 2, count(parts)))}
}

raw := trim(matched, " \t\r\n")

# Signal value tolerates both the bare-string and {output: ...} shapes cupcake may hand back.
matched := s if {
	s := input.signals.last_assistant_wall_of_text
	is_string(s)
} else := s if {
	s := input.signals.last_assistant_wall_of_text.output
} else := ""
