# OPA unit tests for wall_of_text -- the UserPromptSubmit one-paragraph rule and its measured
# correction. There is deliberately NO halt to test: see the policy metadata for why a Stop halt
# cannot work here (its verdict is always rendered to the user, and it fires after the wall of text
# has already been streamed).
#
# NOTE: `opa test` runs the OPA INTERPRETER, which is NOT what cupcake executes -- cupcake compiles
# to WASM and runs its own runtime, where an unimplemented builtin silently yields undefined. These
# cases pin the logic; scripts/test-cupcake-stop-guards.py pins that it fires through the real
# binary, and scripts/check-cupcake-wasm-builtins.py pins that every builtin used here can execute.
#
# Run with:
#   opa test .cupcake/system/commands.rego \
#     .cupcake/policies/claude/wall_of_text.rego \
#     .cupcake/tests/wall_of_text_test.rego
package cupcake.policies.claude.wall_of_text_test

import rego.v1

import data.cupcake.policies.claude.wall_of_text as guard

ups_event(sig) := {
	"hook_event_name": "UserPromptSubmit",
	"signals": {"last_assistant_wall_of_text": sig},
}

has_standing_rule(ctxs) if {
	some c in ctxs
	startswith(c, "ONE PARAGRAPH.")
}

has_correction(ctxs) if {
	some c in ctxs
	startswith(c, "MEASURED:")
}

# The standing rule goes in on every prompt, so the constraint is present before the answer exists.
test_standing_rule_always_present if {
	ctxs := guard.add_context with input as ups_event("")
	has_standing_rule(ctxs)
}

# A clean previous turn gets the rule and nothing else -- no correction to make.
test_no_correction_on_clean_previous_turn if {
	ctxs := guard.add_context with input as ups_event("")
	not has_correction(ctxs)
	count(ctxs) == 1
}

# A measured violation adds the correction, quoting the count and the opener back.
test_correction_quotes_count_and_opener if {
	ctxs := guard.add_context with input as ups_event("WALLOFTEXT:9:The game is up and both DLLs proved themselves")
	has_correction(ctxs)
	some c in ctxs
	startswith(c, "MEASURED:")
	contains(c, "9 paragraphs")
	contains(c, "The game is up and both DLLs proved themselves")
}

# An opener containing colons survives -- the tag is split on ":" and the remainder rejoined.
test_opener_with_colons_is_rejoined if {
	ctxs := guard.add_context with input as ups_event("WALLOFTEXT:3:Root cause: the hook fires late: after streaming")
	some c in ctxs
	startswith(c, "MEASURED:")
	contains(c, "Root cause: the hook fires late: after streaming")
}

# Object-shaped signal ({output: ...}) is accepted too.
test_object_shaped_signal if {
	ctxs := guard.add_context with input as {
		"hook_event_name": "UserPromptSubmit",
		"signals": {"last_assistant_wall_of_text": {"output": "WALLOFTEXT:4:Four paragraphs here", "exit_code": 0}},
	}
	has_correction(ctxs)
}

# Whitespace-only signal is clean.
test_whitespace_signal_is_clean if {
	ctxs := guard.add_context with input as ups_event("   \n")
	not has_correction(ctxs)
}

# An untagged value carries no measurement, so it must NOT produce a correction quoting a number the
# guard does not have. Fabricating "N paragraphs" from an unparseable signal would be inventing a
# fact, which is worse than staying quiet.
test_untagged_signal_does_not_fabricate_a_count if {
	ctxs := guard.add_context with input as ups_event("something unparseable")
	not has_correction(ctxs)
}

# A malformed tag with no opener field is ignored rather than half-rendered.
test_tag_without_opener_is_ignored if {
	ctxs := guard.add_context with input as ups_event("WALLOFTEXT:7")
	not has_correction(ctxs)
}

# This guard is UserPromptSubmit-only. In particular it must contribute NOTHING on Stop: a Stop
# verdict is rendered to the user verbatim, which is the defect this policy was rewritten to remove.
test_contributes_nothing_on_stop if {
	ctxs := guard.add_context with input as {
		"hook_event_name": "Stop",
		"signals": {"last_assistant_wall_of_text": "WALLOFTEXT:9:A nine paragraph answer"},
	}
	count(ctxs) == 0
}

# And it defines no halt at all, on any event -- the property the user asked for ("I see it").
test_defines_no_halt if {
	not guard.halt
}

# --- (3) the WORD budget arm ------------------------------------------------------------------
#
# Paragraph count is blind to a single paragraph that simply keeps going. On 2026-09-03 one
# 281-word paragraph carried four subjects and closed on a pronoun the agent could not resolve when
# the user answered "it". These pin the arm that catches that shape.

words_event(para_sig, words_sig) := {
	"hook_event_name": "UserPromptSubmit",
	"signals": {
		"last_assistant_wall_of_text": para_sig,
		"last_assistant_prose_words": words_sig,
	},
}

has_words_correction(ctxs) if {
	some c in ctxs
	startswith(c, "MEASURED: your PREVIOUS answer was one paragraph but")
}

# The case the paragraph arm cannot see: one paragraph, far too many words.
test_one_long_paragraph_is_corrected if {
	ctxs := guard.add_context with input as words_event("", "PROSEWORDS:281:The fallback is the in-game missing-save picker")
	has_words_correction(ctxs)
}

# The measured number is quoted, never invented.
test_word_correction_quotes_the_measurement if {
	ctxs := guard.add_context with input as words_event("", "PROSEWORDS:281:The fallback is the in-game missing-save picker")
	some c in ctxs
	contains(c, "281 words of prose")
	contains(c, "The fallback is the in-game missing-save picker")
}

# Both arms firing at once would hand the model two corrections for one turn. The paragraph arm is
# the stronger statement (multiple paragraphs are unread outright), so it wins and this one stands
# down.
test_paragraph_arm_suppresses_the_word_arm if {
	ctxs := guard.add_context with input as words_event("WALLOFTEXT:3:A three paragraph answer", "PROSEWORDS:400:A three paragraph answer")
	has_correction(ctxs)
	not has_words_correction(ctxs)
}

# Within budget the signal is empty and nothing is added beyond the standing rule.
test_short_answer_gets_no_word_correction if {
	ctxs := guard.add_context with input as words_event("", "")
	has_standing_rule(ctxs)
	not has_words_correction(ctxs)
}

# Same no-fabrication contract the paragraph arm has: an unparseable or truncated tag stays quiet
# rather than rendering half a sentence with a missing number.
test_malformed_word_tag_is_ignored if {
	ctxs := guard.add_context with input as words_event("", "PROSEWORDS:281")
	not has_words_correction(ctxs)
}

test_untagged_word_signal_does_not_fabricate if {
	ctxs := guard.add_context with input as words_event("", "lots of words honestly")
	not has_words_correction(ctxs)
}

# UserPromptSubmit-only, like everything else here: a Stop verdict is rendered to the user verbatim.
test_word_arm_contributes_nothing_on_stop if {
	ctxs := guard.add_context with input as {
		"hook_event_name": "Stop",
		"signals": {"last_assistant_prose_words": "PROSEWORDS:400:A long single paragraph"},
	}
	count(ctxs) == 0
}
