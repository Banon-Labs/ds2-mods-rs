# METADATA
# scope: package
# title: No Filing a Bug Without QA Steps
# authors: ["ds2-mods-rs agents"]
# custom:
#   severity: HIGH
#   id: DS2-MODS-BUG-NEEDS-QA-STEPS
#   description: >-
#     Deny `bd create --type bug` unless the command carries steps a person can
#     follow to reproduce the defect.
#
#     STANDING ORDER, user 2026-09-24: "if there is a bug, and we file it, it must
#     have QA steps for reproducability. If we follow the QA steps and it is no
#     longer reproducable, we can safely close it."
#
#     The second half is what makes the first half worth enforcing. Steps are not
#     paperwork -- they are the only thing that can ever CLOSE a bug. Without them
#     an issue is unfalsifiable: it cannot be confirmed, it cannot be shown fixed,
#     and it sits in the ready queue forever being recommended as work.
#
#     WHAT PROMPTED IT. ds2-mods-rs-8iz carried a real log line
#     (`kind=88 cancel-dest=0x55`) and a real cause and no steps. A clean launch
#     with dialog_skip on loaded the character correctly; the box never appeared;
#     both surviving logs held zero occurrences of that line and the session it was
#     measured in had rotated away. The agent's response was to start disassembling
#     substate 0x58 to reconstruct a precondition the issue had never stated -- an
#     hour aimed at a symptom nobody could produce. The user stopped it: "Why are
#     you searching down an issue that can't be reproduced."
#
#     Measured the same day: 15 of 20 open bugs carry no repeatable steps. This
#     guard does not touch those; it stops the queue growing.
#
#     WHAT COUNTS. Either a named section -- "steps to reproduce", "repro steps",
#     "QA steps", "to reproduce" -- or a numbered sequence, which is what steps look
#     like when nobody bothered with a heading. A symptom and a log line are not
#     steps, and neither is a session state ("the state the run left them in") that
#     names no action.
#
#     WHY `--body-file` AND `--design-file` ARE REFUSED FOR BUGS rather than trusted.
#     This guard reads the command; it cannot read a file. Exempting a flag whose
#     payload is invisible would make the rule optional, and an optional rule about
#     discipline is the one that gets skipped on the day it matters. The steps go in
#     the command, where they are reviewable in the transcript that files them.
#
#     NOT GATED: tasks, features, epics and chores. A feature has acceptance
#     criteria, not a repro. Only `--type bug` is a claim that something is broken,
#     and only that claim needs to be checkable.
#   routing:
#     required_events: ["PreToolUse"]
#     required_tools: ["Bash"]
package cupcake.policies.claude.bd_bug_needs_qa_steps

import rego.v1

import data.cupcake.system.commands

steer := "A bug has to carry steps someone can follow: a numbered sequence with the expected result and the actual one. Add a `Steps to reproduce:` section, or number the steps. Without them the issue can never be confirmed and can never be closed -- and following steps that no longer reproduce is what lets a bug be closed safely, which is the other half of this rule."

deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"
	some text in executed_texts
	files_a_bug(text)
	not has_steps

	decision := {
		"rule_id": "DS2-MODS-BUG-NEEDS-QA-STEPS",
		"severity": "HIGH",
		"reason": concat(" ", ["This files a bug with no way to reproduce it.", steer]),
	}
}

# The body is in a file this guard cannot read, so the steps cannot be shown to be
# there. Same fail-closed direction as every other guard here: unreadable and absent
# reach the same verdict.
deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"
	some text in executed_texts
	files_a_bug(text)
	body_in_a_file(text)

	decision := {
		"rule_id": "DS2-MODS-BUG-NEEDS-QA-STEPS",
		"severity": "HIGH",
		"reason": concat(" ", ["This files a bug whose body is in a file, which this guard cannot read, so its steps cannot be shown to exist.", steer]),
	}
}

# A wrapper whose payload cannot be read, naming `bd` and `bug`. Scoped so an
# unrelated opaque command is not answered with a bug-report denial.
deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"
	commands.unparsed_shell_payload(raw_command)
	regex.match(`(?i)\bbd\b`, raw_command)
	regex.match(`(?i)\bbug\b`, raw_command)

	decision := {
		"rule_id": "DS2-MODS-BUG-NEEDS-QA-STEPS",
		"severity": "HIGH",
		"reason": concat(" ", ["This command wraps a shell payload the guard cannot read while naming `bd` and `bug`, so it cannot be shown not to file a bug without steps. Run the `bd` command directly.", steer]),
	}
}

raw_command := object.get(input.tool_input, "command", "")

# Quoted spans are KEPT here, unlike the pgrep guard: the steps live inside the
# quoted description, so deleting quoted text would delete the very thing being
# looked for. The invocation test is anchored tightly enough that a sentence
# mentioning `bd create --type bug` in prose does not read as one -- it has to be at
# a command position.
executed_texts := commands.executed_texts(raw_command)

# `bd create` with `--type bug` or `-t bug`, in either order, with any options
# between. `$HOME/.local/bin/bd` and a bare `bd` both count.
files_a_bug(text) if {
	regex.match(`(^|[;&|(\n])[[:space:]]*([^[:space:];&|()]*/)?bd[[:space:]]`, text)
	regex.match(`[[:space:]]create([[:space:]]|$)`, text)
	regex.match(`(--type[[:space:]]+|--type=|-t[[:space:]]+)bug([[:space:]"']|$)`, text)
}

body_in_a_file(text) if {
	regex.match(`--(body|design)-file([[:space:]=]|$)`, text)
}

# A named section, in any of the spellings this repo's issues already use.
has_steps if {
	regex.match(`(?i)(steps to reproduce|repro steps|qa steps|steps:|to reproduce)`, raw_command)
}

# Or a numbered sequence, which is what steps look like with no heading. Two
# consecutive numbers is the test: one "1." is a list of one thing, and a bug that
# reproduces in a single step still has a second line for what was expected.
has_steps if {
	regex.match(`(^|[[:space:]"'(\n])1[.)][[:space:]]`, raw_command)
	regex.match(`(^|[[:space:]"'(\n])2[.)][[:space:]]`, raw_command)
}
