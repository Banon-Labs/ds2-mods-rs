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
#
#     JUDGED PER INVOCATION since 2026-09-24 (bd ds2-mods-rs-mom). The rule used to
#     ask whether a compiling subcommand and an unscoped-looking flag were BOTH
#     PRESENT somewhere in the command text. They are different questions, and the
#     difference is a measured false deny: see the comment block below.
#   routing:
#     required_events: ["PreToolUse"]
#     required_tools: ["Bash"]
package cupcake.policies.claude.require_scoped_cargo

import data.cupcake.system.commands
import rego.v1

# WHAT WAS MEASURED, AND WHAT IT COST (2026-09-24, bd ds2-mods-rs-mom).
#
# This rule used to run four regexes over the WHOLE command string and AND their
# answers together. That asks whether a compiling subcommand and an unscoped flag
# are CO-PRESENT in the text, not whether they belong to the same invocation --
# the same defect commands.rego's segmentation comment documents for the
# destructive guard, arriving here through the flag half instead of the operand
# half. Two commands were denied on 2026-09-23 that start nothing unscoped:
#
#     cargo fmt --all -- --check && cargo clippy -p a -p b -p c --all-targets
#
# -- every BUILDING invocation names its crates; the `--all` belongs to the
# formatter, which this policy's own message declares exempt. Denied, because
# `--all` and `cargo clippy` were both somewhere in the line.
#
#     cargo fmt --all
#     cargo check -p er-quickload --all-targets
#     echo built
#
# -- the same shape with a newline instead of `&&`, which the engine collapses to
# a space before any policy runs. Pinned as an ALLOW in
# scripts/test-cupcake-hook-shim.py since that file was written, and failing
# there the whole time; that expectation is what this rewrite finally satisfies.
#
# A false deny is worse here than a gap, and not by a little. The obvious way to
# satisfy the guard is to drop `--all-targets` from clippy -- the flag that makes
# clippy compile and lint the tests -- so the rule would have been quietly
# trading coverage for the appearance of scoping. And the report OF the misfire
# was itself denied, because a `bd create` whose --description quoted the command
# put the same tokens in the same line: a guard whose own defect cannot be
# described in the tracker that tracks it.
#
# WHAT IT WAS NOT. `--all-targets` and `--all-features` were NOT colliding with
# `--all` by prefix -- the flag regex already required `([[:space:]]|$)` after
# the alternation, and `cargo check -p ds2-rva --all-targets --all-features`
# measured ALLOW against the live engine before this change. The whole-line read
# was the entire defect. The boundary is kept, spelled the same way, and now
# pinned by a test, because a later edit reaching for `contains()` would
# reintroduce a collision this file would then be blamed for.
#
# THREE GRANULARITIES, narrowing:
#
#   1. commands.executed_texts   -- every shell text this command hands to a
#      shell, so a `bash -c "<payload>"` wrapper is decomposed instead of being
#      read as one flat string.
#   2. commands.shell_segments   -- `;`, `&&`, `||`, `&` and `|`. A flag in one
#      statement cannot scope, or unscope, a build in another.
#   3. cargo_invocations         -- each `cargo` token WITHIN a segment starts a
#      new invocation. This exists because of the engine, not because of the
#      shell: `cupcake eval` replaces every unquoted newline with a space before
#      policy evaluation, so the two lines of an ordinary build script arrive
#      welded into one segment with no separator between them. Splitting on the
#      verb reconstructs the boundary the collapse destroyed. Without it, step 2
#      fixes the `&&` spelling of the false deny and leaves the newline spelling
#      firing.
#
# THE INPUT IS THE SEGMENT WITH QUOTED SPANS DELETED (commands.quotes_removed),
# so a command that merely QUOTES a cargo line -- a bd description, a commit
# message, a heredoc body -- has no cargo in it at all to judge. That is the
# same asymmetry guard_layer_destructive_guard documents, and it is why the two
# bd/git text-mention exemptions this file used to carry are GONE: they existed
# to buy back exactly the case that quote deletion now handles generally, and a
# special case that no longer has a case is a rule nobody can reason about. Their
# tests stayed, and they pass through the general path, which is the proof.

command := object.get(input.tool_input, "command", "")

# --- what the engine actually hands us ---------------------------------------
# The live engine collapses UNQUOTED newlines to spaces before any policy runs,
# while `opa test` sees the raw text; commands.executed_texts has already turned
# the newlines INSIDE a quoted span into spaces, so collapsing what remains makes
# the two views identical. That equivalence is the point -- a unit test that
# segments a command the engine would have welded is testing a shape production
# never sees, and the welded shape is precisely where the measured false deny
# lived.
#
# Applied AFTER executed_texts and BEFORE shell_segments: executed_texts needs
# the quote structure intact to find a wrapper payload, and shell_segments'
# contract is that it receives executed_texts output. Collapsing whitespace
# removes no separator and invents none, so the contract holds either side of it.
#
# Builtin-only construction (no regex.replace, no sprintf) because the engine
# compiles policies to wasm and its host does not provide those.
newlines_collapsed(text) := concat(" ", [word |
	some word in split(replace(replace(replace(text, "\t", " "), "\r", " "), "\n", " "), " ")
	word != ""
])

# --- which text in a segment is executable -----------------------------------
# Quoted spans are DELETED before anything is judged, because a quoted argument
# to another program is data. The exception is a segment carrying a command
# SUBSTITUTION: inside `"$(cargo test)"` the quotes are the shell's and the
# payload really runs, and commands.executed_texts does not decompose `$(...)`
# into a text of its own (deliberately -- see its LIMITS comment). Keeping the
# raw segment there is the fail-CLOSED direction and preserves what the previous
# whole-line form caught: `bd close x --reason "$(cargo test)"` was denied before
# this change and is denied after it.
substitution_present(text) if {
	contains(text, "$(")
}

substitution_present(text) if {
	contains(text, "`")
}

judged_text(segment) := segment if {
	substitution_present(segment)
} else := commands.quotes_removed(segment)

# --- one judgement per cargo invocation --------------------------------------
# Split at each ` cargo ` token, which is a boundary a shell would have given us
# had the engine not eaten it. The leading space is REQUIRED and is what keeps a
# path apart from a program: `~/.cargo/bin/cargo test` has no ` cargo ` in it at
# all, so it stays one invocation and is matched whole by cargo_build_pattern,
# whose own optional path-prefix group handles it. `ls /home/u/.cargo/registry`
# likewise never splits, and then fails the command-position test below.
#
# THE HONEST GAP, in the fail-closed direction: an argument whose VALUE is the
# bare word `cargo` (`cargo test --bin cargo -p x`) splits into two chunks and
# the second loses the `-p`, so that shape is denied. A binary named `cargo`
# inside this workspace does not exist; a wrong ALLOW on a real build does, and
# it is the more expensive of the two mistakes.
cargo_invocations(segment) := {invocation |
	some raw in split(replace(segment, " cargo ", " \ncargo "), "\n")
	invocation := trim_space(raw)
	invocation != ""
}

# --- detection ---------------------------------------------------------------
# A `cargo` token at the start of the invocation -- or after any path prefix
# ending in `/` (`~/.cargo/bin/cargo`) -- followed by a COMPILING subcommand.
# `cargo xwin build` is the same shape with one word in between, hence the
# optional `xwin`. The trailing class keeps `cargotest` from matching, and the
# required whitespace after the subcommand keeps a path like `.cargo/registry`
# from matching at all.
#
# The SUBCOMMAND IS READ FROM THE INVOCATION, never from the line: that is what
# keeps `cargo fmt` exempt whatever flags it carries even when a building
# invocation stands next to it, and it is the half of the fix the `&&` case and
# the collapsed-newline case both turn on. `fmt`, `metadata`, `tree` and a bare
# `--version` are absent from the alternation, which is the whole of their
# exemption -- there is no allowlist to keep in sync.
cargo_build_pattern := "(^|[[:space:];|&('\"`])([^[:space:];|&('\"`]*/)?cargo[[:space:]]+(xwin[[:space:]]+)?(build|test|check|clippy|bench|doc)($|[^[:alnum:]_-])"

building_invocation(invocation) if {
	regex.match(cargo_build_pattern, invocation)
}

# COMMAND POSITION, never mere presence: the word must be the program a shell
# would run. `echo cargotest`, `ls /home/banon/.cargo/registry` and a `--reason`
# whose text was quoted all carry the letters and start nothing. commands.rego's
# has_command_verb steps over wrappers and assignments (`sudo`, `env FOO=1`,
# `CARGO_TARGET_DIR=/x`) and over a leading path, so no real invocation is lost
# by asking the narrower question.
#
# It is asked of the INVOCATION rather than of the whole segment, and that is a
# deliberate difference from guard_layer_destructive_guard. A segment here may
# hold several commands that the engine's newline collapse welded together, and
# only the first of them stands at the segment's own command position; the rest
# begin exactly at the ` cargo ` boundaries cargo_invocations cut. Asking at
# segment granularity would silently stop denying the second line of every
# multi-line build script -- which is the production path, not a unit-test
# artefact.
cargo_invocation(invocation) if {
	commands.has_command_verb(invocation, "cargo")
}

# `-p`/`--package` in any accepted spelling: `-p x`, `-p=x`, `--package x`,
# `--package=x`. Naming several crates is still naming them, so one is enough
# anywhere in the SAME invocation.
has_package_flag(invocation) if {
	regex.match("(^|[[:space:]])(-p|--package)([[:space:]]|=)", invocation)
}

# `--workspace`/`--all` are the explicit spelling of the thing being blocked, so
# they never count as a scope even when paired with a `-p`.
#
# MATCHED AS A WHOLE ARGUMENT -- the trailing `([[:space:]]|$)` is load-bearing
# and is why `--all-targets` and `--all-features` are ordinary flags here. A
# `contains(invocation, "--all")` would deny both, and denying `--all-targets` is
# worse than useless: it is the flag that makes clippy lint the tests, so the
# cheapest way to satisfy such a guard is to reduce coverage.
explicit_whole_workspace(invocation) if {
	regex.match("(^|[[:space:]])(--workspace|--all)([[:space:]]|$)", invocation)
}

unscoped_invocation(invocation) if {
	cargo_invocation(invocation)
	building_invocation(invocation)
	not has_package_flag(invocation)
}

unscoped_invocation(invocation) if {
	cargo_invocation(invocation)
	building_invocation(invocation)
	explicit_whole_workspace(invocation)
}

unscoped_cargo if {
	some text in commands.executed_texts(command)
	some segment in commands.shell_segments(newlines_collapsed(text))
	some invocation in cargo_invocations(judged_text(segment))
	unscoped_invocation(invocation)
}

# --- decision ----------------------------------------------------------------

block_reason := "🧁 Cupcake blocked an UNSCOPED cargo invocation. Name the crates: `cargo test -p <crate>`, repeating -p for each one your change touches. Deciding which those are is your work, not the build system's -- its answer to an unscoped request is always 'all of it'. MEASURED 2026-09-02: three subagents each burned ~72 minutes, almost none of it on the research they were sent to do (the Ghidra MCP was 51 calls / 59s total). The cost was whole-workspace validation run repeatedly in worktrees with unshared target/ dirs: four concurrent runs, load average 54 on 16 cores, a gate still unfinished at 19 minutes, then 2003s and 1330s spent sleep-polling because it outruns the Bash timeout. `--workspace` and `--all` are the explicit spelling of the same thing and are blocked too. Exempt: `cargo fmt`, and the non-building `cargo metadata`/`tree`/`--version`. If a command may exceed the Bash timeout, launch it with run_in_background: true -- never `sleep N; tail log`, which converts wall time into model turns at 1:1. See bd subagent-full-check-sh-sleep-poll-is-the-hour-long-tax-2026-09-02."

deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"
	unscoped_cargo

	decision := {
		"rule_id": "DS2-MODS-REQUIRE-SCOPED-CARGO",
		"severity": "HIGH",
		"reason": concat("", [block_reason, "\n\nSource: ", command]),
	}
}
