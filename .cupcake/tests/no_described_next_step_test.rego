# OPA unit tests for no_described_next_step (the Stop-event halt on a turn that named a concrete
# next action and started none of it).
# Run with:
#   opa test .cupcake/system/commands.rego \
#     .cupcake/policies/claude/no_described_next_step.rego \
#     .cupcake/tests/no_described_next_step_test.rego
#
# The split of duty, same as the sibling guards. This file pins the RULE: which combination of facts
# halts a turn and what the correction says. The prose classification that produces those facts --
# which sentences count as naming a next step, and which are exempt -- lives in the SIGNAL and is
# pinned by scripts/test-described-next-step-signal.py against the verbatim failure. Between the two
# suites the corpus is covered end to end: text in, facts out, halt or no halt.
#
# The four cases the user asked for are asserted in BOTH layers on purpose, because they are the
# intent of the whole guard and a reader should be able to find them in either file:
#   * the verbatim closing message of 2026-09-08  -> HALT;
#   * a turn that describes a next step and starts it in the same turn -> allow;
#   * a turn waiting on the user  -> allow;
#   * a turn reporting a genuine blocker -> allow.
package cupcake.policies.claude.no_described_next_step_test

import rego.v1

import data.cupcake.policies.claude.no_described_next_step as guard

# The clause the signal lifts out of the verbatim turn that prompted this guard (2026-09-08).
verbatim := "The next approach follows from the disassembly rather than from hope: the session identifies itself by holding an active state"

facts(nextstep, acted, blocked, handoff, carried) := concat("", [
	"NEXTSTEPFACTS|nextstep=", nextstep,
	"|acted=", acted,
	"|blocked=", blocked,
	"|handoff=", handoff,
	"|carried=", carried,
])

stop_event(sig) := {
	"hook_event_name": "Stop",
	"signals": {"last_assistant_described_next_step": sig},
}

stop_event_object_signal(sig) := {
	"hook_event_name": "Stop",
	"signals": {"last_assistant_described_next_step": {"output": sig, "exit_code": 0}},
}

rule_ids(halts) := {d.rule_id | some d in halts}

halted(sig) if {
	halts := guard.halt with input as stop_event(sig)
	"DS2-MODS-NO-DESCRIBED-NEXT-STEP" in rule_ids(halts)
}

# --- the four cases the user asked for ----------------------------------------------------------

# 1. The verbatim failure of 2026-09-08. A concrete next step, named and not begun, with no tool
# call, no blocker, no question and nothing running. This is the turn the guard exists to refuse.
test_halt_on_the_verbatim_failure if {
	halted(facts(verbatim, "0", "0", "0", "0"))
}

# 2. Described the step AND started it in the same turn. The correct shape; it must never be touched,
# because narrating what you are about to do and then doing it is ordinary good work.
test_allow_when_the_turn_started_the_step if {
	not halted(facts(verbatim, "1", "0", "0", "0"))
}

# 3. Waiting on the user for something only they can do -- drive the game, log in, invade. A real
# wait is not a stall, and blocking it would gag the one turn shape that has to be allowed to stop.
test_allow_when_blocked_on_the_user if {
	not halted(facts("The next step is reading the join path once the session is live.", "0", "0", "1", "0"))
}

# 4. Reporting a genuine blocker. Naming what stops you is required behaviour in this repo, and a
# guard that punished it would push the agent toward silence, which is worse than the defect.
test_allow_when_a_genuine_blocker_is_stated if {
	not halted(facts("The next step is running the probe, which needs sudo.", "0", "1", "0", "0"))
}

# --- the four closers that escaped on 2026-09-16 ------------------------------------------------
# The prose classification that finds these lives in the signal and is pinned by
# scripts/test-described-next-step-signal.py. What is pinned here is the half this layer owns: with
# the clause extracted and no exemption set, each one halts. They are named rather than folded into
# the verbatim case so a regression says which closer came back.

escape_next_edit := "The pre-flight itself is not built yet -- it needs the DLL to issue and poll its own RequestLobbyList the way the Frida agent does, which is the next edit."

escape_next_step := "The gate answers correctly from the agent, and the DLL still reads the param, which is the next step."

escape_next_change := "The param read is what makes it refuse, so dropping it and answering the gate instead is what I would do; that would be the next change."

escape_remaining_work := "The remaining work is to wire it into the game task."

# The judgement call, recorded in the policy comment under "the unperformed measurement": a log line
# the agent's own DLL prints is an oracle it has, and the in-game input it defers to is an input the
# agent drives itself, so this denies rather than resolving to the in-game-observation exemption.
unperformed_measurement := "The one thing measurement has not yet covered: I have proved the query answers correctly from Frida, but not yet watched the DLL's own copy of it print preflight: in a live log -- that needs the finger used once in this run."

test_halt_on_which_is_the_next_edit if {
	halted(facts(escape_next_edit, "0", "0", "0", "0"))
}

test_halt_on_which_is_the_next_step if {
	halted(facts(escape_next_step, "0", "0", "0", "0"))
}

test_halt_on_that_would_be_the_next_change if {
	halted(facts(escape_next_change, "0", "0", "0", "0"))
}

test_halt_on_the_remaining_work_is if {
	halted(facts(escape_remaining_work, "0", "0", "0", "0"))
}

test_halt_on_the_unperformed_measurement if {
	halted(facts(unperformed_measurement, "0", "0", "0", "0"))
}

# The other half of the judgement, and the one that matters more: a turn that genuinely has no
# oracle asks for the observation, asking sets `handoff`, and the guard stays out of its way. An
# over-broad guard that fires on a legitimate ending is worse than the miss it was widened to close.
test_allow_when_the_observation_is_asked_for if {
	not halted(facts(
		"Launched as br-20260913-142426-cc94 -- but not yet confirmed by you or by any oracle.",
		"0", "0", "1", "0",
	))
}

# Naming the next edit and then making it is the correct shape, and the widening must not touch it.
test_allow_when_the_next_edit_was_made if {
	not halted(facts(escape_next_edit, "1", "0", "0", "0"))
}

# The remaining work named beside the one thing that stops it being done now.
test_allow_when_the_remaining_work_is_blocked if {
	not halted(facts(escape_remaining_work, "0", "1", "0", "0"))
}

# --- the remaining exemption, and the shape of the halt -----------------------------------------

# Live background work is already carrying the step.
test_allow_when_background_work_carries_it if {
	not halted(facts(verbatim, "0", "0", "0", "1"))
}

# Object-shaped signal ({output: ...}) is handled, since cupcake may hand back either shape.
test_halt_on_object_signal if {
	halts := guard.halt with input as stop_event_object_signal(facts(verbatim, "0", "0", "0", "0"))
	"DS2-MODS-NO-DESCRIBED-NEXT-STEP" in rule_ids(halts)
}

# The correction quotes the offending sentence back, so the agent knows which step to go and take.
test_reason_quotes_the_described_step if {
	halts := guard.halt with input as stop_event(facts(verbatim, "0", "0", "0", "0"))
	some d in halts
	contains(d.reason, verbatim)
}

# The correction says to take the step now, names starting it in the background as the alternative
# for a large one, and names the three shapes that are still allowed to stop.
test_reason_directs_the_step_to_be_taken if {
	halts := guard.halt with input as stop_event(facts(verbatim, "0", "0", "0", "0"))
	some d in halts
	contains(d.reason, "Do it NOW")
	contains(d.reason, "START it")
	contains(d.reason, "destructive/irreversible")
	contains(d.reason, "real fork")
}

# Severity matches the sibling Stop guards, so the halt is reported at the same weight.
test_halt_is_high_severity if {
	halts := guard.halt with input as stop_event(facts(verbatim, "0", "0", "0", "0"))
	some d in halts
	d.severity == "HIGH"
}

# --- clean and degraded signals -----------------------------------------------------------------

# No next step observed -> no halt. This is the overwhelmingly common case and the one that must
# never cost a turn.
test_no_halt_on_clean_turn if {
	not halted("")
}

# Whitespace-only signal is treated as clean.
test_no_halt_on_whitespace_signal if {
	not halted("   \n")
}

# A facts line with an empty nextstep is clean even when every other fact is present.
test_no_halt_when_nextstep_is_empty if {
	not halted(facts("", "0", "0", "0", "0"))
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
		"signals": {"last_assistant_described_next_step": facts(verbatim, "0", "0", "0", "0")},
	}
	count(halts) == 0
}

# A degraded or crafted facts line that carries a next step but omits the exemption fields must fail
# CLOSED and halt, rather than being waved through by a missing field. Mirrors how the sibling
# guards treat an untagged non-empty value.
test_degraded_signal_fails_closed if {
	halted(concat("", ["NEXTSTEPFACTS|nextstep=", verbatim]))
}

# A sentence carrying a pipe would split the facts line; the signal replaces pipes before emitting,
# and this pins the parser's half of that contract -- the clause is read up to the next field, not
# past it.
test_fact_parsing_stops_at_the_field_boundary if {
	halts := guard.halt with input as stop_event(facts("The next move is reading the vtable.", "0", "0", "0", "0"))
	some d in halts
	contains(d.reason, "The next move is reading the vtable.")
	not contains(d.reason, "acted=")
}
