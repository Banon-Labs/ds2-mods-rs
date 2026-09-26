# METADATA
# scope: package
# title: A build's exit code is the verdict; grepping its output for "error" is not
# authors: ["er-mods-rs agents", "ds2-mods-rs agents"]
# custom:
#   severity: HIGH
#   id: DS2-MODS-NO-GREP-FOR-BUILD-ERRORS
#   description: >-
#     Hard block on piping a BUILD/TEST/CHECK command into a pattern matcher in order to decide
#     whether it succeeded. The tool already answers that question exactly, once, at the end: its
#     EXIT CODE. A grep over its output is a strictly worse oracle -- it is a guess at which strings
#     a failure prints -- and unlike the exit code it can be wrong in the direction that matters,
#     reporting success for a build that failed.
#
#     MEASURED 2026-09-03, and this is the whole reason the policy exists. `cargo check -p
#     er-quickload` was run as `... 2>&1 | grep -E 'error' -A6 | head -20` followed by `echo "---
#     clean ---"`. The build had FAILED with E0603 (a private module path), but the matcher's window
#     did not surface it, "--- clean ---" printed, and the failure was reported to the user as a
#     successful build. The DLL on disk stayed at the previous link for another twenty minutes of
#     work built on top of it. The user's response is the rule: "When you run something that reports
#     error codes, why would you grep it for errors? It has an error code. It only returns one at
#     the end." There was no defensible answer, so the behaviour is prevented rather than
#     remembered -- an advisory note would only fire if it were recalled at the right moment, and
#     this one would not have been.
#
#     WHAT IS BLOCKED: a build-ish command (cargo, rustc, opa, make, ninja, cmake, go, npm/pnpm/yarn,
#     pytest, tsc, or one of this repo's own build scripts) piped into grep/rg/egrep/fgrep/ag/ack.
#     WHAT IS NOT: piping into head/tail/sed/awk/wc/jq/sort/uniq/cut/tr/less/python, which shape or
#     excerpt output rather than adjudicating it; grep ANYWHERE else, including over a build LOG FILE
#     already on disk, which is reading evidence after the exit code has already been believed; and
#     any pipeline whose matcher is not deciding pass/fail because the exit code was captured first.
#
#     THE HAPPY PATH is simply to run the command and let a non-zero exit speak, then read the tail
#     for the message -- `cmd; echo "exit=$?"`, or `cmd || tail -40 build.log`. For a background
#     build, note that the harness's task exit code describes the WRAPPER, not the build: read the
#     log file the build wrote, or its provenance record.
#   routing:
#     required_events: ["PreToolUse"]
#     required_tools: ["Bash"]
package cupcake.policies.claude.no_grep_for_build_errors

import rego.v1

import data.cupcake.system.commands

command := object.get(input.tool_input, "command", "")

# Commands whose exit code IS the verdict.
#
# THE VERB MUST BE IN COMMAND POSITION. er-mods-rs anchors this on `(^|[[:space:];|&('"`])` --
# any WHITESPACE will do -- so the token only has to APPEAR somewhere in the command, and an
# ordinary read of a repo script was denied here on 2026-09-21:
#
#     grep -n 'cupcake' scripts/test-cupcake-policies.py | grep -nE 'eval|policy-dir' | head -20
#
# Nothing was built and nothing was adjudicated: `scripts/test-cupcake-policies.py` is an OPERAND
# of grep, matched by the `scripts/...test...` arm because a space stood in front of it, and the
# second grep supplied the matcher. That shape -- name a script under scripts/ whose name contains
# build/check/test, pipe anything into grep -- is how this repo reads its own guard sources, so the
# guard was denying the work of maintaining it. Same defect class as the co-presence bugs fixed in
# git_block_no_verify: a token test that asks "are these both present" rather than "is this the
# program being run".
#
# commands.command_position_prefix_pattern + commands.path_prefix_pattern is the same construction
# has_command_verb uses, spliced here rather than called because the verb is an alternation rather
# than a literal. It anchors on `(^|[;&|(){}\n])` -- a shell separator, not a space -- so an operand
# can never satisfy it, while `cargo build | grep` (string start) and `x && cargo build | grep`
# (separator) both still do.
#
# The interpreter clause keeps the coverage that costs: `python3 scripts/check-foo.py | grep FAIL`
# runs the gate through an interpreter, so the script name is an argument rather than the program.
# Only OPTION tokens may stand between the interpreter and the script, which is what stops
# `bash -c 'grep x scripts/test-y.py' | grep z` -- where the script is again just text -- from
# being swept back in.
build_verbs := "(cargo|rustc|opa|make|ninja|cmake|go|npm|pnpm|yarn|pytest|tsc|[[:alnum:]_.-]*(build|check|test)[[:alnum:]_.-]*\\.(sh|py))"

#
# The optional quote at the end is `bash -c 'cargo build' 2>&1 | grep error`: the payload runs, and
# its output is what the pipe carries, but the quote stood between the interpreter and the verb, so
# no build verb was ever seen there (found 2026-09-25 while writing the per-pipeline tests below).
interpreter_prefix := "((python3?|uv|uvx|bash|sh|zsh|node|deno|bun)[ \t]+(-{1,2}[A-Za-z0-9][^ \t]*[ \t]+)*['\"]?)?"

build_verb_pattern := concat("", [
	commands.command_position_prefix_pattern,
	interpreter_prefix,
	commands.path_prefix_pattern,
	build_verbs,
	"($|[^[:alnum:]_.-])",
])

# Matchers that ADJUDICATE. head/tail/sed/awk/wc/jq/python are deliberately absent: they excerpt or
# reshape, they do not decide whether the build passed.
#
# The optional `&` after the pipe is bash's merge-stderr-and-pipe shorthand, which puts a character
# between the pipe and the matcher. Without it that spelling walks straight past this guard, which
# is the whole failure mode -- a build whose errors go to stderr is exactly the one that gets piped
# that way.
#
# Checked per pipe STAGE below (the stage text after the `|`), so the `&` is optional at its start.

# The matcher must sit DOWNSTREAM of the build verb, in the SAME pipeline. `cargo build 2>&1 | tee
# log | grep error` and `cargo build |& grep -c error` are the same mistake with more plumbing, and
# both still are: the verb is in one pipe stage and the matcher in a later stage of that pipeline.
#
# MEASURED 2026-09-25: this rule used to ask the two regexes about the WHOLE command, independently,
# which is co-presence again -- the defect recorded above for the command-position anchor, and in
# git_block_no_verify. Two read-only commands were denied for it, and nothing in either was built
# or adjudicated:
#
#     ls scripts/ | grep -i cupcake; command -v opa cupcake
#
# `| grep` belonged to the first command and `opa` to the second, which was only LOOKING `opa` up.
# The other was a for-loop whose quoted word list held the text `'ls scripts/ | grep -i cupcake'`
# and whose body piped `python3 scripts/cupcake-check-command.py "$c" 2>&1` into head: the `| grep`
# was a string, and the build-shaped script name was never piped into a matcher at all.
#
# So the question is now asked per pipeline. The command is read through commands.executed_texts,
# which neutralises quoted separators (a quoted `| grep` is prose, not a pipe) and hands back the
# payload of any `bash -c '...'` as a text of its own, so a build wrapped that way is still seen.
# Each text is cut into STATEMENTS at `;`, `&&`, `||` and newline -- commands that share no data
# stream -- and each statement into pipe stages, and a build stage must come BEFORE a matcher
# stage. A single `&` is deliberately NOT a cut: commands.shell_statements does cut there, and that
# splits `2>&1` and `|&` down the middle, which is exactly the plumbing a build's stderr takes into
# grep. The cost is that `a & b` stays joined, which can only deny more.
#
# `command -v`, `type`, `which`, `whereis` and `hash` only look a name up; they do not run it. The
# shared command_position_prefix_pattern lets `command` and then `-v` stand in front of a verb,
# which is right for `command rm -rf x` and wrong here, so a lookup stage is not a build stage.
#
# Grouping is the one shape statement cutting would lose: `{ cargo build; cargo test; } 2>&1 | grep
# error` and `(cd crates; cargo check) | grep error` put the build before a `;` and the matcher
# after it, piped from the group's closing bracket. The second greps_a_build body keeps those: a
# build verb, then later a closing `)`/`}` piped straight into a matcher.
matcher_names := `(grep|egrep|fgrep|rg|ag|ack)($|[[:space:]])`

matcher_stage_pattern := concat("", [`^&?[[:space:]]*(/?([[:alnum:]_.-]+/)*)?`, matcher_names])

lookup_stage_pattern := `^[[:space:]({]*(command[[:space:]]+-[A-Za-z]*[vV]|type|which|whereis|hash)([[:space:]]|$)`

grouped_build_pattern := concat("", [
	build_verb_pattern,
	`[^\n]*[)}][[:space:]]*([0-9]*>&[0-9-]+[[:space:]]*)*\|&?[[:space:]]*(/?([[:alnum:]_.-]+/)*)?`,
	matcher_names,
])

pipelines(text) := {statement |
	no_or := replace(text, "||", "\n")
	no_and := replace(no_or, "&&", "\n")
	cut := replace(no_and, ";", "\n")
	some raw in split(cut, "\n")
	statement := trim_space(raw)
	statement != ""
}

build_stage(stage) if {
	regex.match(build_verb_pattern, trim_space(stage))
	not regex.match(lookup_stage_pattern, trim_space(stage))
}

greps_a_build if {
	some text in commands.executed_texts(command)
	some statement in pipelines(text)
	stages := split(statement, "|")
	some i, j
	build_stage(stages[i])
	regex.match(matcher_stage_pattern, stages[j])
	j > i
}

greps_a_build if {
	some text in commands.executed_texts(command)
	regex.match(grouped_build_pattern, text)
}

deny contains decision if {
	greps_a_build
	decision := {
		"rule_id": "DS2-MODS-NO-GREP-FOR-BUILD-ERRORS",
		"reason": "A build reports success or failure ONCE, at the end, as its EXIT CODE. Grepping its output for 'error' is a guess at which strings a failure prints, and it fails in the direction that matters: on 2026-09-03 `cargo check ... | grep -E 'error' -A6 | head -20` missed an E0603, printed '--- clean ---', and a FAILED build was reported as built while the previous DLL stayed on disk. Run the command and let the exit code answer -- `<cmd>; echo \"exit=$?\"`, or `<cmd> || tail -40 <log>` to read the message only when it actually failed. Piping into head/tail/sed/awk/wc/jq/python to excerpt output is fine and not blocked; so is grepping a log file that is already on disk. For a BACKGROUND build the harness's task exit code describes the wrapper, not the build -- read the build's own log or its provenance record instead.",
		"severity": "HIGH",
	}
}
