# OPA unit tests for no_shouting_at_turn_end (the Stop-event halt on a turn whose closing prose
# puts its emphasis in the capitalisation).
#
# Run with:
#   opa test .cupcake/system/commands.rego \
#     .cupcake/policies/claude/no_shouting_at_turn_end.rego \
#     .cupcake/tests/no_shouting_at_turn_end_test.rego
#
# The split of duty, the same one the sibling Stop guards use. This file pins the RULE: which score
# halts a turn and what the correction says. The classification that produces the score -- which
# spans are stripped before judging, which words count, and what a run is -- lives in
# scripts/cupcake_shouting.py and is pinned by scripts/test-shouting-signal.py, which also refuses
# to let that module and .cupcake/policies/claude/docs_no_shouting.rego drift apart. Between the
# two suites the path is covered end to end: text in, a score out, halt or no halt. Every score
# used below is the real number that module returns for the line quoted beside it, not an invented
# one.
package cupcake.policies.claude.no_shouting_at_turn_end_test

import rego.v1

import data.cupcake.policies.claude.no_shouting_at_turn_end as guard

# "THE GAME IS INVISIBLE TO pgrep, because it runs under Wine." Five capitalised words, four of
# them on the function-word list, so the module scores it 14.
verbatim_sample := "THE GAME IS INVISIBLE TO pgrep, because it runs under Wine."

verbatim_score := "14"

facts(score, sample) := concat("", ["SHOUTING|score=", score, "|sample=", sample])

stop_event(sig) := {
	"hook_event_name": "Stop",
	"signals": {"last_assistant_shouting": sig},
}

stop_event_object_signal(sig) := {
	"hook_event_name": "Stop",
	"signals": {"last_assistant_shouting": {"output": sig, "exit_code": 0}},
}

rule_ids(halts) := {d.rule_id | some d in halts}

halted(sig) if {
	halts := guard.halt with input as stop_event(sig)
	"DS2-MODS-NO-SHOUTING-AT-TURN-END" in rule_ids(halts)
}

# --- halt: the closing lines this guard exists to refuse ------------------------------------------

# A shouted clause in the middle of a closing sentence, which is the commonest form of the habit.
test_halt_on_a_shouted_clause if {
	halted(facts(verbatim_score, verbatim_sample))
}

# A one-word absolute, scored at exactly the threshold. The run rule cannot see a single word at
# all, so this case is the whole reason the function-word list exists, and it sits on the boundary
# where a threshold change would silently stop catching it.
test_halt_on_a_one_word_absolute_at_the_threshold if {
	halted(facts("4", "NOTHING in ds2-save-file has been run."))
}

# Four ordinary content words with no function word among them, scored 6. This is the case the run
# rule exists for and the only shape the word list cannot reach.
test_halt_on_content_words_only if {
	halted(facts("6", "ONE ROW PITCH PER ADDED ROW."))
}

# --- allow: the closing lines that must keep working ----------------------------------------------

# Below the threshold. One capitalised name in a sentence of prose is a name, and this is the case
# that protects every acronym, register and symbol in the repo's vocabulary.
test_allow_below_the_threshold if {
	not halted(facts("1", "The DLL is built for MSVC and its RVA table is checked against the PE headers."))
}

# Three capitalised words in a row, scored 3, which is what a proper noun written out looks like.
# The threshold sits one above it deliberately: at three, the sweep that tuned this guard collected
# the game's name 114 times and nobody shouting.
test_allow_three_capitalised_words if {
	not halted(facts("3", "The build is staged for DARK SOULS II and nothing else."))
}

# --- the shape of the halt ------------------------------------------------------------------------

# Object-shaped signal ({output: ...}) is handled, since cupcake may hand back either shape.
test_halt_on_object_signal if {
	halts := guard.halt with input as stop_event_object_signal(facts(verbatim_score, verbatim_sample))
	"DS2-MODS-NO-SHOUTING-AT-TURN-END" in rule_ids(halts)
}

# The correction quotes the offending line back, so the agent knows which sentence to rewrite
# rather than being told its turn shouts somewhere.
test_reason_quotes_the_offending_line if {
	halts := guard.halt with input as stop_event(facts(verbatim_score, verbatim_sample))
	some d in halts
	contains(d.reason, verbatim_sample)
}

# It has to say what to do instead, in the terms the user asked for: move the emphasis into the
# structure of the sentence. Without that it reads as "write quieter", which is not the instruction.
test_reason_names_the_alternative if {
	halts := guard.halt with input as stop_event(facts(verbatim_score, verbatim_sample))
	some d in halts
	contains(d.reason, "structure")
	contains(d.reason, "own short paragraph")
}

# And it must say explicitly that a name in capitals was never the offence. This is the half a
# reader is most likely to over-generalise from, and over-generalising it would make an agent
# afraid to write DLL, RVA or a hex constant in a sentence.
test_reason_exempts_names_explicitly if {
	halts := guard.halt with input as stop_event(facts(verbatim_score, verbatim_sample))
	some d in halts
	contains(d.reason, "a NAME in capitals was never the offence")
	contains(d.reason, "backticks")
}

# Severity matches the sibling Stop guards, so the halt is reported at the same weight.
test_halt_is_high_severity if {
	halts := guard.halt with input as stop_event(facts(verbatim_score, verbatim_sample))
	some d in halts
	d.severity == "HIGH"
}

# --- clean and degraded signals -------------------------------------------------------------------

# No shouting observed -> no halt. This is the overwhelmingly common case and the one that must
# never cost a turn.
test_no_halt_on_clean_turn if {
	not halted("")
}

test_no_halt_on_whitespace_signal if {
	not halted("   \n")
}

# A facts line with no score at all is clean: with no measurement there is nothing to quote, and a
# guessed number in the correction would be a fabricated fact.
test_no_halt_when_score_is_empty if {
	not halted(facts("", verbatim_sample))
}

# A non-numeric score is not silently read as a hit either -- `to_number` is undefined and the rule
# simply does not fire.
test_no_halt_when_score_is_not_a_number if {
	not halted(facts("loud", verbatim_sample))
}

test_no_halt_when_signal_absent if {
	halts := guard.halt with input as {"hook_event_name": "Stop", "signals": {}}
	count(halts) == 0
}

# Only Stop events are judged. The same facts on another event must not halt.
test_no_halt_on_non_stop_event if {
	halts := guard.halt with input as {
		"hook_event_name": "PreToolUse",
		"signals": {"last_assistant_shouting": facts(verbatim_score, verbatim_sample)},
	}
	count(halts) == 0
}

# The sample is a line of the agent's own prose, so it can be a markdown table row and carry the
# field separator. It is split off before the fields are parsed, which is what keeps its pipes out
# of the fact map -- without that the sample vanishes from the correction, and a cell containing an
# `=` injects a junk key whose conflict is an eval error, which in the WASM runtime is a silent
# allow.
test_sample_with_pipes_survives_parsing if {
	halts := guard.halt with input as stop_event(facts(verbatim_score, "| piece | THE WHOLE THING IS DONE |"))
	some d in halts
	contains(d.reason, "| piece | THE WHOLE THING IS DONE |")
}

# And a cell that looks like a field must not be able to change the measurement from inside the
# sample. `score=1` written in a table cell is text, not a fact.
test_sample_cannot_forge_a_lower_score if {
	halted(facts(verbatim_score, "| field | score=1 |"))
}
