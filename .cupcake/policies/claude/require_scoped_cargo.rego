# METADATA
# scope: package
# title: Require an explicitly named crate scope on agent cargo invocations
# authors: ["er-mods-rs agents", "ds2-mods-rs agents"]
# custom:
#   severity: HIGH
#   id: DS2-MODS-REQUIRE-SCOPED-CARGO
#   description: >-
#     Hard block on a whole-workspace cargo invocation from an agent Bash command:
#     a compiling subcommand with no `-p`/`--package`, or one that says
#     `--workspace`/`--all` outright. Naming what to build is the AGENT's work. It
#     is the same reasoning already done to decide what to edit, and delegating it
#     to the build system is what turns a thirty-second question into an hour,
#     because the build system's answer to an unscoped request is always "all of
#     it".
#
#     MEASURED 2026-09-02 (bd subagent-full-check-sh-sleep-poll-is-the-hour-long-tax-2026-09-02):
#     three er-npc-possess subagents each ran ~72 minutes. Almost none of that was
#     the research they were dispatched for -- the Ghidra MCP accounted for 51
#     calls totalling 59 seconds. What consumed the time was whole-workspace
#     validation, run repeatedly, in separate worktrees with separate target/ dirs
#     so no build cache was shared. Four ran concurrently on a 16-core box: load
#     average 54.26, and a gate that costs minutes on a quiet tree was still
#     unfinished at 19 minutes. Because it far outruns the Bash tool timeout each
#     agent then sleep-polled its own run -- 18x `sleep 118` = 2003s for one, 60x
#     `timeout 28 sleep 27` = 1330s for another -- and a poll costs a whole model
#     turn, not merely its sleep. One agent spent 33 of its 45 tool-minutes asleep.
#
#     `cargo test -p er-npc-possess` is seconds and answers the question the agent
#     actually has. The whole-workspace gate is the ORCHESTRATOR's job, run once at
#     integration on a quiet tree -- which is also the only condition under which
#     its verdict means anything, since scripts/check.sh reports NOT RUN and
#     INCONCLUSIVE steps and a contended box manufactures both. That restriction
#     lives INSIDE check.sh, which knows its own repo_root and so cannot be evaded
#     by cd'ing; it is deliberately NOT enforced here. A first draft of this policy
#     did deny the gate by name and immediately blocked the edit that was removing
#     it, because the file being edited contains the string -- the same defect
#     block_manual_pgrep documents as "a guard whose own removal cannot be
#     described in the commit that removes it is unwritable in the repo that
#     enforces it".
#
#     Exempt by shape, not by intent: `cargo fmt` (whole-tree formatting is the
#     point, and it compiles nothing), the non-building `cargo metadata`/`tree`/
#     `--version`, a `-p`-scoped invocation however many crates it names, and the
#     narrow non-executing TEXT positions every guard in this directory shares --
#     a single non-chained bd command, or a git commit message.
#   routing:
#     required_events: ["PreToolUse"]
#     required_tools: ["Bash"]
package cupcake.policies.claude.require_scoped_cargo

import rego.v1

command := object.get(input.tool_input, "command", "")

# Whitespace-normalized command. The live cupcake engine collapses unquoted
# newlines to spaces before any policy runs while `opa test` sees the raw text;
# normalizing here makes both behave identically. Same reasoning, and the same
# builtin-only construction, as block_manual_pgrep's norm_command -- the engine
# evaluates policies as wasm modules whose host provides no regex.replace or
# sprintf.
norm_command := concat(" ", [word |
	some word in split(replace(replace(replace(command, "\t", " "), "\r", " "), "\n", " "), " ")
	word != ""
])

# --- quote scrub -------------------------------------------------------------
# Only the text-mention exemptions read this; detection scans the whole command
# so nothing can be smuggled past the guard inside quotes.
escapes_stripped := replace(replace(norm_command, `\"`, ""), `\'`, "")

double_parts := split(escapes_stripped, `"`)

outside_double := concat(" ", [double_parts[idx] |
	some idx
	double_parts[idx]
	idx % 2 == 0
])

single_parts := split(outside_double, "'")

unquoted_command := concat(" ", [single_parts[idx] |
	some idx
	single_parts[idx]
	idx % 2 == 0
])

# --- detection ---------------------------------------------------------------
# A `cargo` token at command start or after a shell separator -- quotes included,
# so `bash -c 'cargo build'` is caught -- optionally preceded by any path prefix
# ending in `/` (`~/.cargo/bin/cargo`), followed by a COMPILING subcommand.
# `cargo xwin build` is the same shape with one word in between, hence the
# optional `xwin`. The trailing class keeps `cargotest` from matching, and the
# required whitespace after the subcommand keeps a path like `.cargo/registry`
# from matching at all.
cargo_build_pattern := "(^|[[:space:];|&('\"`])([^[:space:];|&('\"`]*/)?cargo[[:space:]]+(xwin[[:space:]]+)?(build|test|check|clippy|bench|doc)($|[^[:alnum:]_-])"

cargo_build_invoked if {
	regex.match(cargo_build_pattern, norm_command)
}

# `-p`/`--package` in any accepted spelling: `-p x`, `-p=x`, `--package x`,
# `--package=x`. A crate scope anywhere in the command satisfies the rule --
# naming several crates is still naming them.
has_package_flag if {
	regex.match("(^|[[:space:]])(-p|--package)([[:space:]]|=)", norm_command)
}

# `--workspace`/`--all` are the explicit spelling of the thing being blocked, so
# they never count as a scope even when paired with a `-p`.
explicit_whole_workspace if {
	regex.match("(^|[[:space:]])(--workspace|--all)([[:space:]]|$)", norm_command)
}

unscoped_cargo if {
	cargo_build_invoked
	not has_package_flag
}

unscoped_cargo if {
	cargo_build_invoked
	explicit_whole_workspace
}

# --- text-mention exemptions -------------------------------------------------
# Identical in shape and rationale to block_manual_pgrep's: bd records text and
# git records a message, so a single non-chained invocation whose token sits
# entirely inside quoted text is documentation, not a build. Anything chained,
# substituted, or wrapped in `bash -c` fails the shape and stays denied.
text_mention_only if {
	bd_text_command
	not regex.match(cargo_build_pattern, unquoted_command)
}

text_mention_only if {
	git_commit_text_command
	not regex.match(cargo_build_pattern, unquoted_command)
}

bd_text_command if {
	input.tool_name == "Bash"
	regex.match(`^[[:space:]]*((\$HOME|\$\{HOME\}|~|/home/[[:alnum:]._-]+|/root|/Users/[[:alnum:]._-]+)/\.local/bin/)?bd[[:space:]]+(create|update|comment|comments|remember|close)([[:space:]]|$)`, norm_command)
	not regex.match(`[;|&()<>\x60]`, unquoted_command)
	not contains(command, "$(")
	not contains(command, "`")
}

git_commit_text_command if {
	input.tool_name == "Bash"
	not contains(command, "$(")
	not contains(command, "`")
	not contains(command, "<<")
	regex.match(git_commit_only_pattern, unquoted_command)
}

git_commit_only_pattern := `^[[:space:]]*git([[:space:]]+-C[[:space:]]+[^[:space:];|&()<>]+)?[[:space:]]+(add|commit)[^;|&()<>]*(&&[[:space:]]*git([[:space:]]+-C[[:space:]]+[^[:space:];|&()<>]+)?[[:space:]]+(add|commit)[^;|&()<>]*)*$`

# --- decision ----------------------------------------------------------------

block_reason := "🧁 Cupcake blocked an UNSCOPED cargo invocation. Name the crates: `cargo test -p <crate>`, repeating -p for each one your change touches. Deciding which those are is your work, not the build system's -- its answer to an unscoped request is always 'all of it'. MEASURED 2026-09-02: three subagents each burned ~72 minutes, almost none of it on the research they were sent to do (the Ghidra MCP was 51 calls / 59s total). The cost was whole-workspace validation run repeatedly in worktrees with unshared target/ dirs: four concurrent runs, load average 54 on 16 cores, a gate still unfinished at 19 minutes, then 2003s and 1330s spent sleep-polling because it outruns the Bash timeout. `--workspace` and `--all` are the explicit spelling of the same thing and are blocked too. Exempt: `cargo fmt`, and the non-building `cargo metadata`/`tree`/`--version`. If a command may exceed the Bash timeout, launch it with run_in_background: true -- never `sleep N; tail log`, which converts wall time into model turns at 1:1. See bd subagent-full-check-sh-sleep-poll-is-the-hour-long-tax-2026-09-02."

deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"
	unscoped_cargo
	not text_mention_only

	decision := {
		"rule_id": "DS2-MODS-REQUIRE-SCOPED-CARGO",
		"severity": "HIGH",
		"reason": concat("", [block_reason, "\n\nSource: ", command]),
	}
}
