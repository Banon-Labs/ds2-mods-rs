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

# ---------------------------------------------------------------------------
# 2026-11-02: a COPY of a tracked repo script into /tmp is not authoring, and a /tmp destination
# behind a shell variable assigned in the same command is still /tmp. The tmp_copy_sources signal
# is what vouches for "tracked under scripts/"; these fixtures hand the policy its output.
# ---------------------------------------------------------------------------

scratch := "/tmp/claude-1000/-home-banon-projects-ds2-mods-rs/sess/scratchpad"

frida_src := "scripts/frida/summon-sign-trace.js"

tracked_signal(tokens) := concat("\n", [concat("\t", ["tracked", t]) | some t in tokens])

bash_event_tracked(command, tokens) := object.union(bash_event(command), {"signals": {"tmp_copy_sources": tracked_signal(tokens)}})

# The measured denial, with its literal scratchpad destination.
test_allow_cp_tracked_repo_script_into_literal_scratchpad if {
	not denied(bash_event_tracked(
		sprintf("cd /home/banon/projects/ds2-mods-rs; node --check %s && cp %s %s/summon-sign-trace-frozen.js; sleep 5", [frida_src, frida_src, scratch]),
		[frida_src],
	))
}

test_allow_cp_tracked_repo_script_into_variable_scratchpad if {
	not denied(bash_event_tracked(
		sprintf("S=%s; cp %s $S/summon-sign-trace-frozen.js", [scratch, frida_src]),
		[frida_src],
	))
}

test_allow_cp_tracked_repo_script_into_braced_quoted_variable if {
	not denied(bash_event_tracked(
		sprintf("export S=\"%s\"; cp -f %s \"${S}/frozen.js\"", [scratch, frida_src]),
		[frida_src],
	))
}

test_allow_install_tracked_repo_script_with_mode_into_tmp if {
	not denied(bash_event_tracked("install -m 755 scripts/ds2-run.py /tmp/run.py", ["scripts/ds2-run.py"]))
}

test_allow_signal_object_shape if {
	event := object.union(bash_event("cp scripts/ds2-run.py /tmp/run.py"), {"signals": {"tmp_copy_sources": {"output": "tracked\tscripts/ds2-run.py"}}})
	not denied(event)
}

# The carve-out is cp/install of TRACKED sources only.
test_deny_cp_untracked_source_into_tmp if {
	denied(bash_event_tracked("cp scripts/new-probe.js /tmp/probe.js", [frida_src]))
}

test_deny_cp_without_signal_into_tmp if {
	denied(bash_event(sprintf("cp %s %s/frozen.js", [frida_src, scratch])))
}

test_deny_cp_non_repo_source_into_tmp if {
	denied(bash_event_tracked(sprintf("cp /home/banon/Downloads/x.js %s/x.js", [scratch]), [frida_src]))
}

test_deny_cp_tracked_plus_untracked_sources_into_tmp_dir_file if {
	denied(bash_event_tracked(sprintf("cp %s ../other/evil.js /tmp/x.js", [frida_src]), [frida_src]))
}

test_deny_cp_target_directory_flag if {
	denied(bash_event_tracked(sprintf("cp -t /tmp/out %s /tmp/x.js", [frida_src]), [frida_src]))
}

test_deny_mv_tracked_source_into_tmp if {
	denied(bash_event_tracked(sprintf("mv %s /tmp/x.js", [frida_src]), [frida_src]))
}

test_deny_cp_when_command_relinks if {
	denied(bash_event_tracked(sprintf("ln -sf /tmp/evil.js %s; cp %s /tmp/x.js", [frida_src, frida_src]), [frida_src]))
}

test_deny_cp_tracked_then_appended_in_same_command if {
	denied(bash_event_tracked(sprintf("cp %s /tmp/x.js; echo 'evil()' >> /tmp/x.js", [frida_src]), [frida_src]))
}

# Writing new script content into /tmp behind a variable is still authoring.
test_deny_heredoc_into_variable_tmp_script if {
	denied(bash_event("S=/tmp/x; cat > $S/evil.py <<'EOF'\nprint(1)\nEOF"))
}

test_deny_echo_redirect_into_quoted_variable_tmp_script if {
	denied(bash_event(sprintf("S=%s; echo 'print(1)' > \"$S/probe.py\"", [scratch])))
}

test_deny_printf_append_into_braced_variable_tmp_script if {
	denied(bash_event("D=/tmp; printf 'set -e\\n' >> ${D}/x.sh"))
}

test_deny_tee_into_variable_tmp_script if {
	denied(bash_event(sprintf("S=%s; echo 'print(1)' | tee $S/probe.py", [scratch])))
}

test_deny_python_open_write_into_variable_tmp_script if {
	denied(bash_event(sprintf("S=%s; python3 -c \"open('$S/probe.py','w').write('x')\"", [scratch])))
}

test_deny_sed_rewrite_into_variable_tmp_script if {
	denied(bash_event_tracked(sprintf("S=%s; sed 's/a/b/' %s > $S/frozen.js", [scratch, frida_src]), [frida_src]))
}

test_deny_cp_non_repo_source_into_variable_tmp if {
	denied(bash_event_tracked(sprintf("S=%s; cp ~/Downloads/x.js $S/x.js", [scratch]), [frida_src]))
}

test_deny_variable_reassigned_to_tmp if {
	denied(bash_event("S=/home/banon/out; S=/tmp/x; echo 'x' > $S/a.py"))
}

# A variable the command never assigns, or assigns outside /tmp, is not guessed at.
test_allow_redirect_into_unassigned_variable if {
	not denied(bash_event("echo 'x' > $CLAUDE_PROJECT_DIR/scripts/a.py"))
}

test_allow_redirect_into_variable_assigned_outside_tmp if {
	not denied(bash_event("S=/home/banon/projects/ds2-mods-rs/scripts; echo 'x' > $S/a.py"))
}

test_allow_data_artifact_into_variable_tmp if {
	not denied(bash_event(sprintf("S=%s; python3 scripts/probe.py > $S/out.json", [scratch])))
}

test_allow_sed_extract_into_variable_tmp_rs if {
	not denied(bash_event(sprintf("S=%s; sed -n 1,40p crates/ds2-loader/src/lib.rs > $S/ours.rs", [scratch])))
}
