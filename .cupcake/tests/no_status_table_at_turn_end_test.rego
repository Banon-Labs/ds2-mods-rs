# OPA unit tests for no_status_table_at_turn_end (the Stop-event halt on a turn that closes on a
# tidy table of the agent's own progress).
# Run with:
#   opa test .cupcake/system/commands.rego \
#     .cupcake/policies/claude/no_status_table_at_turn_end.rego \
#     .cupcake/tests/no_status_table_at_turn_end_test.rego
#
# The split of duty, same as the sibling guards. This file pins the RULE: which combination of facts
# halts a turn and what the correction says. The classification that produces those facts -- which
# pipe-bearing lines are a table at all, which cells are progress words, and which prompts count as
# asking for a table -- lives in scripts/cupcake_status_table.py and is pinned by
# scripts/test-status-table-signal.py. Between the two suites the path is covered end to end: text
# in, facts out, halt or no halt. The hit counts used below are the real ones that module returns
# for the tables quoted here, not invented numbers.
package cupcake.policies.claude.no_status_table_at_turn_end_test

import rego.v1

import data.cupcake.policies.claude.no_status_table_at_turn_end as guard

# The first row of the table that prompted this guard (2026-09-23), which is what the signal lifts
# out as its sample. The whole table scores 5: `state` in the header, `state` again inside
# "name/state/stats per slot, in Rust", then `done`, `next` and `after that`.
verbatim_sample := "| piece | state |"

verbatim_hits := "5"

facts(hits, asked, blocked, sample) := concat("", [
	"STATUSTABLE|hits=", hits,
	"|asked=", asked,
	"|blocked=", blocked,
	"|sample=", sample,
])

stop_event(sig) := {
	"hook_event_name": "Stop",
	"signals": {"last_assistant_status_table": sig},
}

stop_event_object_signal(sig) := {
	"hook_event_name": "Stop",
	"signals": {"last_assistant_status_table": {"output": sig, "exit_code": 0}},
}

rule_ids(halts) := {d.rule_id | some d in halts}

halted(sig) if {
	halts := guard.halt with input as stop_event(sig)
	"DS2-MODS-NO-STATUS-TABLE-AT-TURN-END" in rule_ids(halts)
}

# --- deny: the tables this guard exists to refuse ------------------------------------------------

# 1. The verbatim failure of 2026-09-23. Four rows over a save picker, two of them work that was not
# done, closing a turn nobody had asked for a table in. This is the turn the guard is named for.
test_halt_on_the_verbatim_progress_table if {
	halted(facts(verbatim_hits, "0", "0", verbatim_sample))
}

# 2. The minimal shape: a two-column state table, `| item | state |` over `done` and `pending`. It
# scores 3 -- the `state` header plus both cells -- and is the form this defect usually takes when
# there is less to report. Pinned separately so a threshold change cannot quietly stop catching it.
test_halt_on_a_two_column_state_table if {
	halted(facts("3", "0", "0", "| item | state |"))
}

# --- allow: the tables that must keep working ----------------------------------------------------

# 3. Below threshold. One progress word is not a status report, and this is the case that protects
# every real data table: a measurement grid that happens to carry a `state` or `status` column.
test_allow_below_the_threshold if {
	not halted(facts("1", "0", "0", "| 0x14 | done |"))
}

# 4. The user asked for a table. An asked-for table is the deliverable; gagging it would hand back
# exactly the failure this guard exists to remove, in the other direction.
test_allow_when_the_user_asked_for_a_table if {
	not halted(facts(verbatim_hits, "1", "0", verbatim_sample))
}

# 5. The turn is genuinely waiting on the user. A real wait is not a stall, and it is one of the few
# turn shapes that is allowed to stop -- with or without a table in it.
test_allow_when_blocked_on_the_user if {
	not halted(facts(verbatim_hits, "0", "1", verbatim_sample))
}

# 6. A measurement table with one stray progress word, said in full: an offset/value grid whose last
# cell reads `done`. This is the false positive that would matter most, because the repo's
# one-paragraph rule pushes measurements into tables in almost every turn that reports one.
test_allow_on_a_measurement_table_with_one_stray_done if {
	not halted(facts("1", "0", "0", "| 0x0000a3c0 | 0x4141 |"))
}

# --- the shape of the halt -----------------------------------------------------------------------

# Object-shaped signal ({output: ...}) is handled, since cupcake may hand back either shape.
test_halt_on_object_signal if {
	halts := guard.halt with input as stop_event_object_signal(facts(verbatim_hits, "0", "0", verbatim_sample))
	"DS2-MODS-NO-STATUS-TABLE-AT-TURN-END" in rule_ids(halts)
}

# The correction quotes the offending row back, so the agent knows which table to delete, and names
# the count so the measurement is visible rather than asserted.
test_reason_quotes_the_offending_row if {
	halts := guard.halt with input as stop_event(facts(verbatim_hits, "0", "0", verbatim_sample))
	some d in halts
	contains(d.reason, verbatim_sample)
	contains(d.reason, "5 of its cells")
}

# The correction says to delete the table and then either carry on or name the one blocker in a
# line. Without both halves it would read as "write less", which is not the instruction.
test_reason_directs_the_table_to_be_deleted_and_the_work_continued if {
	halts := guard.halt with input as stop_event(facts(verbatim_hits, "0", "0", verbatim_sample))
	some d in halts
	contains(d.reason, "DELETE THE TABLE")
	contains(d.reason, "KEEP WORKING")
	contains(d.reason, "ONE line")
}

# And it must say explicitly that a table of DATA is fine. This is the half a reader of the halt is
# most likely to over-generalise from, and over-generalising it would fight DS2-MODS-WALL-OF-TEXT,
# which tells the agent to put content into tables.
test_reason_exempts_tables_of_data_explicitly if {
	halts := guard.halt with input as stop_event(facts(verbatim_hits, "0", "0", verbatim_sample))
	some d in halts
	contains(d.reason, "a table of DATA is fine")
	contains(d.reason, "A table of YOUR OWN PROGRESS is not")
}

# Severity matches the sibling Stop guards, so the halt is reported at the same weight.
test_halt_is_high_severity if {
	halts := guard.halt with input as stop_event(facts(verbatim_hits, "0", "0", verbatim_sample))
	some d in halts
	d.severity == "HIGH"
}

# --- clean and degraded signals -------------------------------------------------------------------

# No table observed -> no halt. This is the overwhelmingly common case and the one that must never
# cost a turn.
test_no_halt_on_clean_turn if {
	not halted("")
}

# Whitespace-only signal is treated as clean.
test_no_halt_on_whitespace_signal if {
	not halted("   \n")
}

# A facts line with no hit count at all is clean: with no measurement there is nothing to quote, and
# a guessed number in the correction would be a fabricated fact.
test_no_halt_when_hits_is_empty if {
	not halted(facts("", "0", "0", verbatim_sample))
}

# A non-numeric hit count is not silently read as a hit either -- `to_number` is undefined and the
# rule simply does not fire.
test_no_halt_when_hits_is_not_a_number if {
	not halted(facts("many", "0", "0", verbatim_sample))
}

# Missing signal entirely (routing not satisfied) -> no halt, and no evaluation error.
test_no_halt_when_signal_absent if {
	halts := guard.halt with input as {"hook_event_name": "Stop", "signals": {}}
	count(halts) == 0
}

# Only Stop events are judged. The same facts on another event must not halt.
test_no_halt_on_non_stop_event if {
	halts := guard.halt with input as {
		"hook_event_name": "PreToolUse",
		"signals": {"last_assistant_status_table": facts(verbatim_hits, "0", "0", verbatim_sample)},
	}
	count(halts) == 0
}

# A degraded or crafted facts line that carries a hit count but omits the exemption fields must fail
# CLOSED and halt, rather than being waved through by a missing field.
test_degraded_signal_fails_closed if {
	halted("STATUSTABLE|hits=5")
}

# The sample is a table ROW, so it always contains the field separator. It is split off before the
# fields are parsed, which is what keeps its pipes out of the fact map -- without that the sample
# vanishes from the correction, and a cell containing an `=` injects a junk key.
test_sample_with_pipes_survives_parsing if {
	halts := guard.halt with input as stop_event(facts(verbatim_hits, "0", "0", "| name/state/stats per slot, in Rust | done, host-testable |"))
	some d in halts
	contains(d.reason, "| name/state/stats per slot, in Rust | done, host-testable |")
}

# And a cell that looks like a field must not be able to turn the exemptions on from inside the
# sample. `asked=1` written in a table cell is text, not a fact.
test_sample_cannot_forge_an_exemption if {
	halted(facts(verbatim_hits, "0", "0", "| flag | asked=1 | blocked=1 |"))
}
