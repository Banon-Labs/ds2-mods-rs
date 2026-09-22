# OPA unit tests for require_scoped_cargo.
#
# Not loaded by the cupcake engine (which scans .cupcake/policies/<harness>/
# and .cupcake/system/ only). Run with:
#   opa test .cupcake/policies/claude/require_scoped_cargo.rego \
#            .cupcake/tests/require_scoped_cargo_test.rego
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

# ...and an unquoted token in a bd command is a real build, not prose.
test_deny_bd_with_unquoted_cargo if {
	denied_cargo("$HOME/.local/bin/bd close x --reason cargo test")
}
