# OPA unit tests for require_scoped_cargo.
#
# Not loaded by the cupcake engine (which scans .cupcake/policies/<harness>/
# and .cupcake/system/ only). Run with:
#   opa test .cupcake/system/commands.rego \
#            .cupcake/policies/claude/require_scoped_cargo.rego \
#            .cupcake/tests/require_scoped_cargo_test.rego
# (commands.rego joined the list on 2026-09-24, when the rule stopped matching
# the whole line and started judging each shell segment on its own.)
# End-to-end engine coverage lives in scripts/test-cupcake-policies.py.
package cupcake.policies.claude.require_scoped_cargo_test

import rego.v1

import data.cupcake.policies.claude.require_scoped_cargo as guard

bash_event(cmd) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Bash",
	"tool_input": {"command": cmd, "timeout": 30000, "description": "test case"},
}

rule_ids(denials) := {d.rule_id | some d in denials}

denied_cargo(cmd) if {
	denials := guard.deny with input as bash_event(cmd)
	"DS2-MODS-REQUIRE-SCOPED-CARGO" in rule_ids(denials)
}

allowed(cmd) if {
	denials := guard.deny with input as bash_event(cmd)
	count(denials) == 0
}

# --- (a) unscoped cargo is DENIED --------------------------------------------

test_deny_bare_cargo_test if {
	denied_cargo("cargo test")
}

test_deny_bare_cargo_build if {
	denied_cargo("cargo build --release")
}

test_deny_bare_cargo_check if {
	denied_cargo("cargo check")
}

# The exact shape AGENTS.md documents for the DLL, which silently builds only
# default-members and reads as a successful incremental build.
test_deny_cargo_xwin_build_without_p if {
	denied_cargo("cargo xwin build --release --target x86_64-pc-windows-msvc")
}

# --workspace / --all are the explicit spelling of the thing being blocked.
test_deny_cargo_test_workspace if {
	denied_cargo("cargo test --workspace")
}

test_deny_cargo_test_all if {
	denied_cargo("cargo test --all")
}

# ...even paired with -p, which would otherwise satisfy has_package_flag.
test_deny_workspace_even_with_package if {
	denied_cargo("cargo test --workspace -p er-gfx")
}

# No escape hatch through quoting or a wrapper shell.
test_deny_cargo_inside_bash_c if {
	denied_cargo("bash -c 'cargo test'")
}

test_deny_cargo_after_separator if {
	denied_cargo("cd /tmp && cargo build")
}

test_deny_cargo_piped if {
	denied_cargo("cargo test 2>&1 | tail -5")
}

test_deny_path_prefixed_cargo if {
	denied_cargo("~/.cargo/bin/cargo test")
}

# Unquoted newlines arrive collapsed to spaces in the live engine; norm_command
# makes opa test see the same thing.
test_deny_cargo_on_second_line if {
	denied_cargo("echo hi\ncargo test")
}

# --- (b) scoped cargo is ALLOWED ---------------------------------------------

test_allow_cargo_test_with_p if {
	allowed("cargo test -p er-npc-possess")
}

test_allow_cargo_test_multiple_p if {
	allowed("cargo test -p er-quickload -p er-title-flow --lib")
}

test_allow_long_package_flag if {
	allowed("cargo test --package er-gfx")
}

test_allow_equals_package_flag if {
	allowed("cargo build --package=er-hook")
}

test_allow_cargo_xwin_build_with_p if {
	allowed("cargo xwin build --release --target x86_64-pc-windows-msvc -p er-invasion-warp")
}

test_allow_manifest_path_with_p if {
	allowed("cargo test --manifest-path /repo/Cargo.toml -p er-save-loader")
}

# --- (c) non-building cargo subcommands are ALLOWED --------------------------

# Whole-tree formatting is the point of `cargo fmt`, and it compiles nothing.
test_allow_cargo_fmt_all if {
	allowed("cargo fmt --all -- --check")
}

test_allow_cargo_metadata if {
	allowed("cargo metadata --no-deps --format-version 1")
}

test_allow_cargo_tree if {
	allowed("cargo tree -i syn")
}

test_allow_cargo_version if {
	allowed("cargo --version")
}

# A word merely CONTAINING cargo must not match.
test_allow_cargo_substring_word if {
	allowed("echo cargotest")
}

test_allow_cargo_culted_path_word if {
	allowed("ls /home/banon/.cargo/registry")
}

# --- (d) text-mention exemptions ---------------------------------------------

# bd records text; a single non-chained bd command may describe the guard.
test_allow_bd_remember_mentioning_cargo if {
	allowed("$HOME/.local/bin/bd remember --key k \"agents must run cargo test -p <crate>, never bash scripts/check.sh\"")
}

# A git commit message may describe the change that adds this guard.
test_allow_git_commit_message_mentioning_cargo if {
	allowed("git commit -m \"guard: deny unscoped cargo test and scripts/check.sh\"")
}

# ...but a chained batch is not a single text-recording invocation.
test_deny_bd_chained_with_real_cargo if {
	denied_cargo("$HOME/.local/bin/bd remember --key k \"note\" && cargo test")
}

# ...and an UNQUOTED token in a bd command is not prose. Once the engine has
# collapsed newlines to spaces, `--reason cargo test` and a second line that
# runs `cargo test` are the same bytes apart from the option name, and telling
# them apart would mean knowing bd's option arity. Denied, which is the
# fail-closed side, and the escape hatch is the one the issue report itself
# used: quote the text.
test_deny_bd_with_unquoted_cargo if {
	denied_cargo("$HOME/.local/bin/bd close x --reason cargo test")
}

# --- (e) per-invocation judging (2026-09-24, bd ds2-mods-rs-mom) -------------
#
# MEASURED REPRO 1, denied by the whole-line form on 2026-09-23. Every BUILDING
# invocation names its crates; the `--all` belongs to the formatter, which this
# policy declares exempt.
test_allow_fmt_all_chained_with_scoped_clippy if {
	allowed("cargo fmt --all -- --check && cargo clippy -p ds2-rva -p ds2-menu-row -p ds2-item-warn --all-targets -- -D warnings")
}

# MEASURED REPRO 2, which no longer reproduced by the time the fix was written
# (quoted spans were already being deleted before the flag tests ran). Pinned so
# a later edit cannot re-deny the report of this guard's own misfire.
test_allow_bd_create_quoting_the_denied_command if {
	allowed("$HOME/.local/bin/bd create --type bug --title \"require-scoped-cargo misfires\" --description \"denied: cargo fmt --all -- --check && cargo clippy -p ds2-rva --all-targets\"")
}

# The same false deny spelled with a newline instead of `&&`. The engine
# collapses it to a space, so the two invocations arrive welded into one segment
# and only the per-invocation split can separate the formatter's flag from the
# build. Pinned as an ALLOW in scripts/test-cupcake-hook-shim.py since that file
# was written, and failing there until this change.
test_allow_fmt_all_then_scoped_check_on_the_next_line if {
	allowed("cargo fmt --all\ncargo check -p er-quickload --all-targets\necho built")
}

# ...and the same script before it was scoped stays denied, both as the raw text
# `opa test` sees and as the welded single line the engine produces.
test_deny_unscoped_check_on_the_next_line if {
	denied_cargo("cargo fmt --all\ncargo check --all-targets\necho built")
}

test_deny_unscoped_check_welded_onto_fmt if {
	denied_cargo("cargo fmt --all cargo check --all-targets echo built")
}

# A scope in one segment does not reach a build in another. The whole-line form
# allowed this, because `-p` was present SOMEWHERE.
test_deny_unscoped_build_after_a_scoped_one if {
	denied_cargo("cargo test -p ds2-rva && cargo build")
}

# ...and an unscoped clippy chained after the formatter is still denied, which is
# what keeps the repro above from being a blanket exemption for `&&`.
test_deny_unscoped_clippy_chained_after_fmt if {
	denied_cargo("cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings")
}

# `cargo fmt` is exempt WHATEVER flags it carries -- the subcommand is read from
# the invocation, so there is no flag it can hold that makes it a build.
test_allow_cargo_fmt_workspace if {
	allowed("cargo fmt --workspace")
}

# WHOLE-ARGUMENT flag matching. These two were NOT the live defect: both were
# measured ALLOW against the engine before the fix, because the flag regex
# already required whitespace or end-of-string after the alternation. Pinned
# anyway, because a later edit reaching for `contains(cmd, "--all")` would deny
# the flag that makes clippy lint the tests.
test_allow_all_targets_and_all_features_are_not_all if {
	allowed("cargo check -p ds2-rva --all-targets --all-features")
}

test_allow_xwin_clippy_with_all_targets if {
	allowed("cargo xwin clippy -p ds2-rva -p ds2-dialog-skip --all-targets --no-deps --target x86_64-pc-windows-msvc")
}

# A line continuation joins lines into ONE invocation; it is not a boundary, and
# the `-p` on its last line scopes the build on its first.
test_allow_line_continuation_within_one_invocation if {
	allowed("cargo xwin build --release \\\n  --target x86_64-pc-windows-msvc \\\n  -p er-quickload")
}

# Quoted spans are deleted before judging -- EXCEPT where the quotes are the
# shell's own and the payload really runs. Fail closed on a substitution, which
# is what the whole-line form caught and what must not be lost with it.
test_deny_command_substituted_cargo_in_a_quoted_argument if {
	denied_cargo("$HOME/.local/bin/bd close x --reason \"$(cargo test)\"")
}

# A path that merely CONTAINS `cargo` does not split an invocation, so the `-p`
# after it still counts.
test_allow_cargo_home_manifest_path_with_p if {
	allowed("cargo test --manifest-path ~/.cargo/registry/src/x/Cargo.toml -p ds2-rva")
}
