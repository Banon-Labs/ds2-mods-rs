# OPA unit tests for no_grep_for_build_errors.
#
# The case that created the policy is `test_the_2026_09_03_command_is_denied`: that exact pipeline
# reported a FAILED cargo check as a clean build. Everything else here pins the boundary, because a
# guard that also blocks excerpting output or reading a log would just get worked around.
#
# Run with:
#   opa test .cupcake/system/commands.rego \
#     .cupcake/policies/claude/no_grep_for_build_errors.rego \
#     .cupcake/tests/no_grep_for_build_errors_test.rego
package cupcake.policies.claude.no_grep_for_build_errors_test

import rego.v1

import data.cupcake.policies.claude.no_grep_for_build_errors as guard

bash(cmd) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Bash",
	"tool_input": {"command": cmd},
}

denied(cmd) if {
	some d in guard.deny with input as bash(cmd)
	d.rule_id == "DS2-MODS-NO-GREP-FOR-BUILD-ERRORS"
}

# --- the incident ------------------------------------------------------------------------------

test_the_2026_09_03_command_is_denied if {
	denied("timeout 28 cargo check -p er-quickload 2>&1 | grep -E 'error' -A6 | head -20; echo \"--- clean ---\"")
}

# --- the same mistake with more plumbing -------------------------------------------------------

test_tee_before_the_matcher_is_still_denied if {
	denied("cargo build --release 2>&1 | tee build.log | grep error")
}

test_stderr_merge_shorthand_is_denied if {
	denied("cargo xwin build --release |& grep -c error")
}

test_ripgrep_counts_as_a_matcher if {
	denied("cargo test -p er-game-base 2>&1 | rg '^error'")
}

test_repo_build_script_is_covered if {
	denied("bash scripts/er-build-dlls.sh er-quickload 2>&1 | grep -i failed")
}

test_other_build_tools_are_covered if {
	denied("opa test .cupcake/policies | grep FAIL")
	denied("make -j8 2>&1 | egrep 'Error'")
	denied("pytest -q 2>&1 | grep -E 'failed|error'")
}

# --- the boundary: shaping output is not adjudicating it ---------------------------------------

test_tail_is_allowed if {
	not denied("timeout 28 cargo check -p er-quickload 2>&1 | tail -20")
}

test_head_and_sed_and_awk_are_allowed if {
	not denied("cargo build 2>&1 | head -40")
	not denied("cargo build 2>&1 | sed -n '1,20p'")
	not denied("cargo build 2>&1 | awk '{print $1}'")
}

# The exit code being consulted is the whole point, and must never be blocked.
test_checking_the_exit_code_is_allowed if {
	not denied("cargo check -p er-quickload; echo \"exit=$?\"")
	not denied("cargo build --release || tail -40 build.log")
}

# Grepping a log ALREADY on disk is reading evidence after the exit code was believed, not
# substituting for it.
test_grepping_a_log_file_is_allowed if {
	not denied("grep -n 'E0603' build4.log")
	not denied("rg 'could not compile' /tmp/build.log")
}

# No build verb at all -> nothing to adjudicate.
test_unrelated_grep_pipeline_is_allowed if {
	not denied("cat er-invasion-warp.log | grep heartbeat")
	not denied("ls crates | grep quickload")
}

# A verb that merely CONTAINS a build word is not a build verb.
test_lookalike_verbs_do_not_match if {
	not denied("mycargo status | grep error")
	not denied("./not-make.sh | grep error")
}

# --- command position: an OPERAND that names a build script is not a build -----------------------
#
# The 2026-09-21 false positive, verbatim. Nothing is built and nothing is adjudicated: the script
# path is an argument to grep, and the second grep narrows the first grep's output. Under the
# any-whitespace anchor this file inherited from er-mods-rs, every one of these was denied.
test_reading_a_build_script_with_grep_is_allowed if {
	not denied("grep -n 'cupcake' scripts/test-cupcake-policies.py | grep -nE 'eval|policy-dir' | head -20")
	not denied("cat scripts/check.sh | grep -n signal")
	not denied("sed -n '1,40p' scripts/check-rust-build.sh | grep cargo")
	not denied("ls scripts/ | grep test")
}

# The same token in a git/bd operand, which is how these files get committed and described.
test_naming_a_build_script_in_an_operand_is_allowed if {
	not denied("git log --oneline -- scripts/test-cupcake-policies.py | grep -c fix")
	not denied("git diff scripts/check.sh | grep '^+'")
}

# ... and the coverage that motivated the arm in the first place must survive the narrowing.
test_running_a_build_script_is_still_denied if {
	denied("scripts/check.sh 2>&1 | grep -i failed")
	denied("./scripts/check-rust-build.sh | grep error")
	denied("bash scripts/build-all.sh | grep -c error")
}

# An interpreter in command position promotes its script argument to the program.
test_interpreter_invoked_gate_is_still_denied if {
	denied("python3 scripts/check-cupcake-wasm-builtins.py | grep -i fail")
	denied("python3 -u scripts/test-cupcake-policies.py 2>&1 | grep AssertionError")
}

# But an interpreter that merely QUOTES a script name is not running it.
test_interpreter_quoting_a_script_name_is_allowed if {
	not denied("bash -c 'grep -n x scripts/test-y.py' | grep z")
}

# A build verb after a real separator is still in command position.
test_build_verb_after_a_separator_is_denied if {
	denied("cd crates && cargo build 2>&1 | grep error")
	denied("(cargo check) | grep error")
}
