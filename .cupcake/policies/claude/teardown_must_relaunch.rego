# METADATA
# scope: package
# title: A teardown must relaunch in the same command
# authors: ["er-quickload agents", "ds2-mods-rs agents (ported from er-mods-rs)"]
# custom:
#   severity: HIGH
#   id: DS2-MODS-TEARDOWN-MUST-RELAUNCH
#   description: >-
#     PORTED from er-mods-rs on 2026-09-25 (`ER-EFFECTS-TEARDOWN-MUST-RELAUNCH`), with
#     the script names swapped: `er-teardown.py` -> `scripts/ds2-teardown.py`,
#     `er-run-branch.py` -> `scripts/ds2-run.py`. er's two extra launchers
#     (`~/Elden/launch.sh`, `er-run-gamescope.sh`) and its `--reason` flag have no DS2
#     counterpart and were dropped; `ds2-teardown.py --dry-run`, which only lists what
#     it would signal, is read-only here like `--status`. ds2-run.py tears the previous
#     session down itself before it launches, so the relaunch form here is
#     ds2-run.py alone -- a teardown chained in front of it is redundant and allowed.
#
#     Hard block on a Bash command that runs `scripts/ds2-teardown.py` without
#     launching again in the same command. Teardown belongs immediately before a
#     launch and nowhere else. Stapled to the front of a build it reads as launch
#     hygiene and is actually a kill: on 2026-09-12, in er-mods-rs, it ended a run
#     while the user was driving it, one line after that run logged the fix they
#     were inspecting, for a rebuild nobody had asked for yet. `--status` is
#     read-only and always allowed.
#
#     The rule is deliberately not "ask whether a run is live". The agent cannot
#     see whether a person is looking at the screen, and a memory saying "never
#     tear down a run the user is touching" did not stop it. Requiring the launch
#     in the same command makes the failure mode unreachable: the worst outcome
#     becomes a restarted game rather than no game.
#   routing:
#     required_events: ["PreToolUse"]
#     required_tools: ["Bash"]
package cupcake.policies.claude.teardown_must_relaunch

import rego.v1

import data.cupcake.system.commands

tool_command := object.get(input.tool_input, "command", "")

# The texts this command actually hands to a shell, quoted operand spans and heredoc
# bodies anchor-neutralised. Read instead of the raw command for the reason
# `no_whole_check_sh` records at length: matching the whole command string asks whether
# two tokens are CO-PRESENT, not whether either one runs.
#
# That is not a theoretical distinction here. Measured 2026-09-21 (bd er-effects-rs-ak3q,
# bd er-effects-rs-if5l): `git commit -F - <<EOF ... EOF` was refused because the commit
# message described this guard and named the script, and `bd create --description "..."`
# was refused because the issue text did. The issue reporting the second one had to be
# filed through a body file written by a separate command in order to exist at all, and the
# commit message had to be reworded to say "a repo-relative teardown command" instead of
# naming the file. Rewording a commit message or an issue body to appease a matcher
# degrades the record this guard is not there to police, and the invoked binary in both
# cases was `git` and `bd`.
executed_texts := commands.input_executed_texts

# Whitespace-normalized, so a command written across lines matches the same way in
# the live engine (which collapses whitespace) and under `opa test` (which does not).
norm_command_for(cmd) := concat(" ", [word |
	some word in split(replace(replace(replace(cmd, "\t", " "), "\r", " "), "\n", " "), " ")
	word != ""
])

# Every rule below asks a question about TOKENS -- is this script invoked, is the next word
# `--status`, is the whole command just the teardown -- and answers it without `regex.match`.
# That is forced, not stylistic.
#
# All four were first written as regexes. They pass `opa test`, whose Go runtime links RE2
# natively, and they CRASHED the engine: every `cupcake eval` on this harness aborted with
#
#     memory fault at wasm address 0x1d24804 in linear memory of size 0x1c0000:
#     wasm trap: out of bounds memory access
#
# the backtrace running `re2::Regexp::Decref` <- `re2::RE2::~RE2` <- `reuse(re2::RE2*)` <-
# `opa_regex_match` <- a rule in this file. `reuse` is OPA's compiled-regex cache evicting an
# entry, and the faulting pointer is about 16x past the end of a 1.75 MB linear memory, so the
# cache hands back freed memory. The subject string was 80 characters long, and removing one
# pattern moved the fault to the next rule rather than curing it: the rulebook's policies together
# compile more distinct regexes than that cache survives, and these were the ones that tipped it.
#
# A crashed evaluation is not a refusal. Cupcake returns `{}` and every policy in the rulebook
# goes silent at once, so a regex added here can disarm the Elden Ring launch guard. Prefer
# `split`/`contains`/`startswith` in new policies, and treat a green `opa test` as necessary and
# never as evidence -- `scripts/test-cupcake-policies.py` drives the real runtime and is what
# caught this.

# Whether a token names one of the scripts this guard understands.
#
# This used to split every token and index the last path component. The live Cupcake WASM
# evaluator aborted in that helper on a long read-only Python extraction command that named no
# teardown script at all, turning a guard into a policy-engine crash. The guard only needs exact
# basename matching for two known script names, so avoid array indexing entirely.
token_names_teardown(tok) if {
	tok == "ds2-teardown.py"
}

token_names_teardown(tok) if {
	endswith(tok, "/ds2-teardown.py")
}

token_names_run_branch(tok) if {
	tok == "ds2-run.py"
}

token_names_run_branch(tok) if {
	endswith(tok, "/ds2-run.py")
}

# A teardown that will actually kill something: the script stands in a command slot, and
# its own next word is not `--status` or `--dry-run` (which lists what it would signal and
# signals nothing).
#
# Folding `--status` in here rather than testing it as a separate `status_only` closes a
# hole the two-rule shape had: `ds2-teardown.py --status; ds2-teardown.py` satisfied "some
# teardown is followed by --status" and the real kill chained behind it went unexamined.
killing_teardown if {
	some text in executed_texts
	words := commands.command_slot_words(text)
	some index, word in words
	token_names_teardown(word)
	commands.word_in_command_slot(words, index)
	not next_word(words, index) in {"--status", "--dry-run"}
}

# The word after `index`, or the empty string when the invocation ends the text. Spelled
# with `else` because a bare `words[index + 1]` is undefined past the end, and an undefined
# term takes the whole rule with it -- here that would mean a teardown written as the last
# word of a command was not a teardown at all.
next_word(words, index) := word if {
	index + 1 < count(words)
	word := words[index + 1]
} else := ""

# Any invocation of the teardown script, with or without a path prefix, `--status` included.
runs_teardown if {
	some text in executed_texts
	words := commands.command_slot_words(text)
	some index, word in words
	token_names_teardown(word)
	commands.word_in_command_slot(words, index)
}

invokes_run_branch if {
	some text in executed_texts
	words := commands.command_slot_words(text)
	some index, word in words
	token_names_run_branch(word)
	commands.word_in_command_slot(words, index)
}

# The relaunch that has to ride along. `ds2-run.py` is the only launcher in this repo.
relaunches if {
	invokes_run_branch
}

# A dry run stages and launches nothing, so pairing a teardown with one would
# satisfy the letter of the rule and still leave the user with no game.
#
# Asked per SEGMENT, because `--dry-run` only excuses nothing when it belongs to the launch: a
# teardown chained with a real launch and some other command's `--dry-run` is still a relaunch.
dry_run if {
	some text in executed_texts
	some segment in commands.shell_segments(text)
	contains(segment, "ds2-run.py")
	contains(segment, "--dry-run")
}

# A teardown that is the WHOLE command, ending a run on purpose.
#
# The harm this rule exists to stop was a teardown that rode along with OTHER work
# -- `ds2-teardown.py; <a build> ...` read as launch hygiene and was a kill.
# A teardown alone is the honest form of "this run is finished", and forbidding it
# deadlocked the very workflow the sibling guard demands: a source edit is refused
# while a run is live, so ending that run has to be expressible without also
# relaunching a build that is about to be replaced.
#
# `>/dev/null`-style redirections are allowed because they suppress output rather
# than add work; a `;`, `|`, `&` or `&&` that introduces another command is not.
#
# Read off `norm_command_for(tool_command)`, which is split on whitespace ONLY. That is what makes the
# rule correct: a separator stays glued to its token, so `ds2-teardown.py;` is not the bare script
# name and a chained command cannot pass as one that stands alone.
interpreter_word(tok) if {
	tok in {"python", "python3"}
}

# `>`, `2>&1`, `>/dev/null` and the `/dev/null` operand of a detached `>`.
redirect_word(tok) if {
	contains(tok, ">")
}

redirect_word(tok) if {
	startswith(tok, "/dev/")
}

teardown_alone if {
	runs_teardown
	words := [tok |
		some tok in split(norm_command_for(tool_command), " ")
		tok != ""
	]
	rest := [words[i] |
		some i, _ in words
		not interpreter_word(words[i])
		not redirect_word(words[i])
	]
	count(rest) == 1
	token_names_teardown(rest[0])
}

block_reason := "🧁 Cupcake blocked a teardown that does not relaunch. `scripts/ds2-teardown.py` belongs immediately before a launch and nowhere else -- stapled to the front of a build it reads as hygiene and is actually a kill of a session the user may be driving (er-mods-rs, 2026-09-12: it ended a run one line after that run logged the fix the user was inspecting). You do not need it before a launch at all: `scripts/ds2-run.py` tears the previous session down itself before it starts one. So:\n\n    python3 /home/banon/projects/ds2-mods-rs/scripts/ds2-run.py ...\n\nBuild FIRST, in its own command, then launch -- the build does not need the game stopped. `--status` and `--dry-run` are read-only and always allowed. A `ds2-run.py --dry-run` does not count as a relaunch: it stages nothing and still leaves the user with no game.\n\nA teardown that is the WHOLE command is allowed, which is the form to hand the user when they ask for one:\n\n    python3 /home/banon/projects/ds2-mods-rs/scripts/ds2-teardown.py\n\nSpell it absolutely there. The user's shell is not in the repo root, so `python3 scripts/ds2-teardown.py` names nothing for them."

deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"
	killing_teardown
	not teardown_alone
	not relaunches

	decision := {
		"rule_id": "DS2-MODS-TEARDOWN-MUST-RELAUNCH",
		"severity": "HIGH",
		"reason": concat("", [block_reason, "\n\nSource: ", tool_command]),
	}
}

deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"
	killing_teardown
	relaunches
	dry_run

	decision := {
		"rule_id": "DS2-MODS-TEARDOWN-MUST-RELAUNCH",
		"severity": "HIGH",
		"reason": concat("", [block_reason, "\n\nSource: ", tool_command]),
	}
}
