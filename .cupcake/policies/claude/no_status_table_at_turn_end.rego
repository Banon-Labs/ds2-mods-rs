# METADATA
# scope: package
# title: Ban ending a turn on a tidy table of the agent's own progress
# authors: ["ds2-mods-rs agents"]
# custom:
#   severity: HIGH
#   id: DS2-MODS-NO-STATUS-TABLE-AT-TURN-END
#   description: >-
#     User directive 2026-09-23, in their words: "Maybe we can add a rego stop policy for tidy
#     tables. I don't think you've ever produced one when I asked for one, you only produce them
#     when you're trying to stop early."
#
#     THE INSTANCE, from the turn immediately before that directive. The work in flight was a save
#     picker. One piece of it landed, and the turn ended on this:
#
#         | piece                                                     | state                |
#         |-----------------------------------------------------------|----------------------|
#         | name/state/stats per slot, in Rust                        | done, host-testable  |
#         | soul level                                                | deliberately absent  |
#         | picker rows over those slots                              | next                 |
#         | load one chosen character instead of the whole container  | after that           |
#
#     Nothing in it is information the user asked for. Two of its four rows are work that was NOT
#     done, written out in the format that makes not-doing-it look like a deliverable. The agent's
#     own account, from the following turn: "ending a turn with a tidy table reads as progress and
#     costs me nothing". That is the whole defect -- the grid is cheap, it looks like delivery, and
#     it is emitted at the exact moment the work stops.
#
#     WHAT THIS DOES NOT TOUCH, AND THE DISTINCTION IS THE POINT. Not the table -- the CONTENT of
#     its cells. A table of DATA is the right shape for something the user has to scan: an offset
#     and its value, a file and its hash, a flag and its effect. This repo's own one-paragraph rule
#     (DS2-MODS-WALL-OF-TEXT) actively pushes content INTO tables for exactly that reason -- "put
#     what remains in a table, a list or a code block, which are scanned rather than read" -- so a
#     guard that read "table" as the offence would be fighting a rule this repo enforces on every
#     prompt. The offence is a table whose cells are `done` / `next` / `missing` / `after that`:
#     a status report on the agent, in a grid, standing in for the work.
#
#     THE VIOLATION IS A CONJUNCTION OF THREE FACTS, computed in the signal:
#       1. the turn's FINAL prose run contains a markdown table (a separator row is required, so a
#          shell pipeline in prose is not one) with at least MIN_HITS=2 progress words in its cells;
#       2. the user did not ask for a table -- "table", "compare", "side by side", "matrix",
#          "checklist" in their last prompt all exempt it, because an asked-for table is the
#          deliverable and gagging it would be the worse failure;
#       3. the turn is not genuinely waiting on the user (cupcake_turn_scan.blocked_on_user).
#     A table that is not in the final prose run never reaches this rule at all: the signal reads
#     `turn.text_runs[-1]`, so a table mid-turn with work after it is the agent showing findings and
#     carrying on, which is the shape this guard wants more of.
#
#     WHY TWO HITS AND NOT ONE. `done`, `state`, `status`, `step` and `stage` are in the lexicon and
#     are the ones that can plausibly head a column in a real data table. One of them alone proves
#     nothing -- a measurement table with a `state` column is a measurement table. Two progress
#     words in one table is no longer a coincidence; the turn that prompted this guard scores five,
#     and a two-column `| item | state |` / `done` / `pending` table scores three. The threshold is
#     MIN_HITS in scripts/cupcake_status_table.py, which owns the lexicon, the table detection and
#     the ask detection so the signal, its unit test and any audit cannot drift into three different
#     answers. It is re-applied here rather than trusted, so a degraded or crafted signal cannot
#     walk a one-hit table past the rule, and so the threshold is testable in this layer too.
#
#     The signal emits ONE facts line --
#     STATUSTABLE|hits=<n>|asked=0|1|blocked=0|1|sample=<first offending row> -- so the OBSERVATION
#     lives in the shell and the RULE lives here, where it is unit-testable. Empty signal -> no halt.
#
#     KNOWN GAP, the same one every Stop guard here has: an interrupted turn fires no Stop event, so
#     a status table the user cuts short is not caught. The sibling pairs (no_authority_agreement +
#     _reminder, idle_hold + _reminder) close that with a UserPromptSubmit interlock reading the same
#     signal; add one the same way if the gap bites.
#   routing:
#     required_events: ["Stop"]
#     required_signals: ["last_assistant_status_table"]
package cupcake.policies.claude.no_status_table_at_turn_end

import rego.v1

# Enforcement: block turn-end when the closing prose is a grid of the agent's own progress.
halt contains decision if {
	input.hook_event_name == "Stop"
	status_table_at_turn_end
	decision := {
		"rule_id": "DS2-MODS-NO-STATUS-TABLE-AT-TURN-END",
		"reason": reason,
		"severity": "HIGH",
	}
}

# The conjunction. `asked` and `blocked` are the two ways out, and each is a turn shape that is
# allowed to end on a progress table: one the user requested, and one that is genuinely waiting on
# them. The hit count is the trigger, and it is checked here as well as in the signal.
status_table_at_turn_end if {
	to_number(hits) >= min_hits
	asked == "0"
	blocked == "0"
}

# Mirrors MIN_HITS in scripts/cupcake_status_table.py. The two are deliberately the same number in
# two places: the signal uses it to decide whether to speak at all, and this uses it to decide
# whether to believe what it said.
min_hits := 2

reason := msg if {
	msg := concat("", ["You ended the turn on a table of your OWN PROGRESS, beginning '", sample, "' -- ", hits, " of its cells are status words (done / next / missing / after that / still to). That is a report on you, not information for the user, and it arrived at the exact moment you stopped working. A tidy grid reads as progress and costs nothing to write, which is precisely why it is not one: the rows labelled `next` and `after that` are work that does not exist. Nobody asked for it -- the user's last prompt did not request a table -- and you named no blocker. DELETE THE TABLE, and then either KEEP WORKING (make the tool call that does the row you labelled `next`) or, if something genuinely stops you, say what it is in ONE line. To be exact about what is banned, because the two are opposites: a table of DATA is fine and always was -- an offset and its value, a file and its hash, a flag and its effect are scanned faster in a grid, and this repo's one-paragraph rule actively pushes content into one. A table of YOUR OWN PROGRESS is not, because it dresses work you have not done as a deliverable."])
}

# --- signal parsing ------------------------------------------------------------------------------
# STATUSTABLE|hits=<n>|asked=0|blocked=0|sample=<first offending row>
#
# `sample` IS SPLIT OFF BEFORE ANYTHING ELSE IS PARSED, and this is not tidiness. The sample is a
# markdown table row, so it ALWAYS contains the `|` that separates the fields -- feeding it to the
# field split would drop the sample from the reason, and a cell containing an `=` would inject a
# junk key into the fact map (two such cells conflicting is an eval error, which in the WASM runtime
# is an undefined decision, which is a silent ALLOW). So the head of the line is parsed as fields and
# everything after the marker is taken verbatim as the sample.
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

# The leading tag carries no "=" so it drops out of the fact map on its own. A field the signal omits
# falls back to a default that does NOT exempt: a degraded or crafted signal fails closed and halts,
# matching how the sibling guards treat an untagged non-empty value.
fact[k] := v if {
	some kv in split(head, "|")
	n := indexof(kv, "=")
	n > 0
	k := trim(substring(kv, 0, n), " \t\r\n")
	v := trim(substring(kv, n + 1, -1), " \t\r\n")
}

# The trigger. Absent or unparseable -> `to_number` is undefined -> no halt, the same contract
# `nextstep` has in no_described_next_step: with no measurement there is nothing to quote, and a
# guessed number in the correction would be a fabricated fact.
hits := object.get(fact, "hits", "")

asked := object.get(fact, "asked", "0")

blocked := object.get(fact, "blocked", "0")

raw := trim(matched_facts, " \t\r\n")

# Signal value tolerates both the bare-string and {output: ...} shapes cupcake may hand back.
matched_facts := p if {
	p := input.signals.last_assistant_status_table
	is_string(p)
} else := p if {
	p := input.signals.last_assistant_status_table.output
} else := ""
