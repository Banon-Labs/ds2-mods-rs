# OPA unit tests for edit_no_tmp_scripts_guard.
#
# Run with:
#   opa test .cupcake/policies/claude/edit_no_tmp_scripts_guard.rego \
#            .cupcake/tests/edit_no_tmp_scripts_guard_test.rego
package cupcake.policies.claude.edit_no_tmp_scripts_guard_test

import rego.v1

import data.cupcake.policies.claude.edit_no_tmp_scripts_guard as guard

write_event(path) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "write",
	"tool_input": {"path": path, "content": "test"},
}

claude_write_event(path) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Write",
	"tool_input": {"file_path": path, "content": "test"},
}

bash_event(command) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Bash",
	"tool_input": {"command": command, "timeout": 30000, "description": "test case"},
}

rule_ids(denials) := {d.rule_id | some d in denials}

denied(input_event) if {
	denials := guard.deny with input as input_event
	"DS2-MODS-NO-TMP-SCRIPTS-GUARD" in rule_ids(denials)
}

test_deny_pi_write_tmp_ps1_path_key if {
	denied(write_event("/tmp/check-er-net-effects.ps1"))
}

test_deny_claude_write_tmp_python_file_path_key if {
	denied(claude_write_event("/tmp/ghidra_probe.py"))
}

test_deny_bash_cat_tmp_script if {
	denied(bash_event("cat > /tmp/check-er-net-effects.ps1 <<'EOF'\nWrite-Output hi\nEOF"))
}

test_deny_bash_python_tmp_script if {
	denied(bash_event("python3 - <<'PY'\nopen('/tmp/probe.py','w').write('print(1)')\nPY"))
}

test_allow_tmp_json_artifact if {
	not denied(write_event("/tmp/check-er-net-effects.json"))
}

test_allow_repo_script if {
	not denied(write_event("/home/choza/projects/er-mods-rs/scripts/check-er-net-effects.ps1"))
}

test_allow_bash_read_tmp_script_path_without_authoring_verb if {
	not denied(bash_event("powershell.exe -File /tmp/existing-user-script.ps1"))
}

test_allow_non_pretooluse_event if {
	denials := guard.deny with input as {
		"hook_event_name": "PostToolUse",
		"tool_name": "write",
		"tool_input": {"path": "/tmp/x.ps1"},
	}
	count(denials) == 0
}

# ---------------------------------------------------------------------------
# READS out of /tmp are not authoring. Every case below is a measured or exactly
# analogous false positive of the old "interpreter word anywhere + /tmp path
# anywhere" test (2026-08-25).
# ---------------------------------------------------------------------------

# The original report: a heredoc that only READS third-party js downloaded into /tmp.
test_allow_bash_heredoc_reading_tmp_js if {
	not denied(bash_event(concat("\n", [
		"python3 - <<'EOF'",
		"import re,glob",
		"for p in sorted(glob.glob('/tmp/planner/*.js')):",
		"    t=open(p,encoding='utf-8',errors='replace').read()",
		"    print(len(t))",
		"EOF",
	])))
}

# The second report: no interpreter at all -- "bun" matched inside "planner-bundle",
# and /tmp is the SOURCE of the copy, not its destination.
test_allow_bash_cp_out_of_tmp_into_repo if {
	not denied(bash_event("mkdir -p target/planner-bundle && cp -f /tmp/planner/*.js target/planner-bundle/ && ls target/planner-bundle"))
}

test_allow_bash_node_runs_tmp_script if {
	not denied(bash_event("node /tmp/x.js"))
}

test_allow_bash_python3_runs_tmp_script if {
	not denied(bash_event("python3 /tmp/x.py"))
}

test_allow_bash_cat_reads_tmp_script if {
	not denied(bash_event("cat /tmp/x.sh"))
}

test_allow_bash_grep_reads_tmp_script if {
	not denied(bash_event("grep -n pattern /tmp/x.py"))
}

test_allow_bash_glob_over_tmp_scripts if {
	not denied(bash_event("ls -la /tmp/planner/*.js"))
}

test_allow_bash_python_open_read_mode if {
	not denied(bash_event("python3 -c \"print(open('/tmp/planner/a.js').read()[:20])\""))
}

test_allow_bash_python_open_encoding_kwarg_not_a_mode if {
	not denied(bash_event("python3 -c \"open('/tmp/planner/a.js', encoding='utf-8', errors='replace')\""))
}

test_allow_bash_mv_out_of_tmp if {
	not denied(bash_event("mv /tmp/x.py scripts/x.py"))
}

# Word-boundary regressions: an interpreter/command name buried in an unrelated word
# must not arm the rule even when a /tmp script path is present.
test_allow_bash_command_word_only_as_substring if {
	not denied(bash_event("cargo run --bin nodemon-concatenate -- --committee /tmp/planner/x.js"))
}

# Data ARTIFACTS into /tmp remain allowed -- only source/script extensions are blocked.
test_allow_bash_redirect_data_artifact_into_tmp if {
	not denied(bash_event("python3 scripts/probe.py > /tmp/out.json"))
}

# Measured 2026-09-24: one side of a merge conflict saved for comparison. Every byte came out of a
# tracked file; the .rs name describes the content, not a script being written.
test_allow_bash_sed_extract_into_tmp_rs if {
	not denied(bash_event("sed -n '/<<<<<<< /,/=======/p' crates/ds2-loader/src/lib.rs > /tmp/claude-1000/scratch/ours.rs"))
}

test_allow_bash_git_show_extract_into_tmp_rs if {
	not denied(bash_event("git -C /home/banon/projects/ds2-mods-rs show HEAD:crates/ds2-loader/src/lib.rs > /tmp/theirs.rs"))
}

test_allow_bash_head_extract_after_separator if {
	not denied(bash_event("mkdir -p /tmp/cmp && head -80 scripts/ds2-run.py > /tmp/cmp/run-head.py"))
}

# The carve-out must not become a way to author. Each of these still writes new bytes.
test_deny_bash_echo_piped_through_sed_into_tmp_script if {
	denied(bash_event("echo 'print(1)' | sed -n p > /tmp/probe.py"))
}

test_deny_bash_heredoc_into_sed_into_tmp_script if {
	denied(bash_event("sed -n p <<'EOF' > /tmp/probe.py\nprint(1)\nEOF"))
}

test_deny_bash_extract_then_append_authoring if {
	denied(bash_event("sed -n 1,5p a.py > /tmp/probe.py; echo 'print(1)' >> /tmp/probe.py"))
}

# ---------------------------------------------------------------------------
# WRITES into /tmp are still denied.
# ---------------------------------------------------------------------------

test_deny_bash_append_redirect_tmp_script if {
	denied(bash_event("printf 'set -e\\n' >> /tmp/x.sh"))
}

test_deny_bash_echo_redirect_tmp_script if {
	denied(bash_event("echo 'print(1)' > /tmp/probe.py"))
}

test_deny_bash_stderr_redirect_tmp_script if {
	denied(bash_event("./build.sh 2>/tmp/wrapper.sh"))
}

test_deny_bash_tee_tmp_script if {
	denied(bash_event("echo 'print(1)' | tee -a /tmp/probe.py"))
}

test_deny_bash_curl_output_flag_tmp_script if {
	denied(bash_event("curl -sL https://example.invalid/x.js -o /tmp/planner/x.js"))
}

test_deny_bash_curl_long_output_flag_tmp_script if {
	denied(bash_event("curl -sL https://example.invalid/x.js --output /tmp/planner/x.js"))
}

test_deny_bash_wget_output_flag_tmp_script if {
	denied(bash_event("wget -O /tmp/planner/x.js https://example.invalid/x.js"))
}

test_deny_bash_cp_into_tmp_script if {
	denied(bash_event("cp scripts/probe.py /tmp/probe.py"))
}

test_deny_bash_mv_into_tmp_script if {
	denied(bash_event("mv probe.sh /tmp/probe.sh && bash /tmp/probe.sh"))
}

test_deny_bash_install_into_tmp_script if {
	denied(bash_event("install -m 755 probe.sh /tmp/probe.sh"))
}

test_deny_bash_pathlib_write_text_tmp_script if {
	denied(bash_event("python3 -c \"from pathlib import Path; Path('/tmp/probe.py').write_text('print(1)')\""))
}
