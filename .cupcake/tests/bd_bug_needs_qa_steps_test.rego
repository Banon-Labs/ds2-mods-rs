package cupcake.policies.claude.bd_bug_needs_qa_steps_test

import rego.v1

import data.cupcake.policies.claude.bd_bug_needs_qa_steps as guard

bash(command) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Bash",
	"tool_input": {"command": command},
}

denied(command) if {
	count(guard.deny) > 0 with input as bash(command)
}

allowed(command) if {
	count(guard.deny) == 0 with input as bash(command)
}

# --- the issue that caused this rule -------------------------------------------------------------
#
# ds2-mods-rs-8iz, filed with a cause and a log line and no steps. A clean run did
# not reproduce it and the agent went disassembling to invent the precondition.

test_denies_a_bug_with_a_symptom_and_no_steps if {
	denied("bd create --type bug 'dialog-skip answers the load confirm with its cancel edge' -d 'Measured in the live run. The box logs cancel-dest=0x55 and the load never happens.'")
}

test_allows_the_same_bug_once_it_carries_steps if {
	allowed(`bd create --type bug 'dialog-skip answers the load confirm' -d 'Steps to reproduce: 1. LOAD GAME at the title. 2. Press confirm on an occupied slot. Expected: the character loads. Actual: nothing happens.'`)
}

# --- what counts as steps ------------------------------------------------------------------------

test_a_numbered_sequence_counts_without_a_heading if {
	allowed(`bd create -t bug 'row caption goes stale' -d '1. Press the row. 2. Return to the title. 3. Reopen the pause menu -- the caption is still the old one.'`)
}

test_one_numbered_line_is_not_a_sequence if {
	denied(`bd create -t bug 'something is wrong' -d '1. It breaks sometimes.'`)
}

test_repro_steps_spelling_counts if {
	allowed(`bd create --type bug 'x' -d 'Repro steps: open the menu, press A.'`)
}

# --- jurisdiction ---------------------------------------------------------------------------------
#
# Only `--type bug`. A feature has acceptance criteria, not a repro, and gating it
# would make the rule noise that gets switched off.

test_allows_a_feature_with_no_steps if {
	allowed("bd create --type feature 'Port er-invasion-path to DS2' -d 'A walkable route to other players.'")
}

test_allows_a_task_with_no_steps if {
	allowed("bd create --type task 'Runtime-test the swap end to end' -d 'Launch and drive it.'")
}

test_allows_reading_and_closing_issues if {
	allowed("bd show ds2-mods-rs-8iz")
}

test_allows_closing_a_bug_without_steps if {
	allowed("bd close ds2-mods-rs-8iz --reason 'not reproducible'")
}

# --- spellings that must not walk past -------------------------------------------------------------

test_denies_the_long_type_equals_form if {
	denied("bd create --title x --type=bug -d 'it is broken'")
}

test_denies_an_absolute_path_invocation if {
	denied("$HOME/.local/bin/bd create --type bug 'x' -d 'it is broken'")
}

test_denies_when_chained_after_another_command if {
	denied("cargo build && bd create --type bug 'x' -d 'it is broken'")
}

# The body is somewhere this guard cannot read, so the steps cannot be shown to be
# there. Fail closed, and say so.
test_denies_a_body_file_even_though_it_might_contain_steps if {
	denied("bd create --type bug 'x' --body-file /tmp/report.md")
}

test_denies_a_body_file_that_mentions_steps_in_the_command if {
	denied("bd create --type bug 'x' --body-file steps-to-reproduce.md")
}

# --- prose is not an invocation --------------------------------------------------------------------

test_allows_a_commit_message_describing_the_rule if {
	allowed("git commit -m 'the guard denies bd create --type bug without steps'")
}
