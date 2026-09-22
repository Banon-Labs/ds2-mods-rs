package cupcake.policies.claude.bash_no_python_file_write_test

import data.cupcake.policies.claude.bash_no_python_file_write
import rego.v1

bash(cmd) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Bash",
	"tool_input": {"command": cmd},
}

# --- the shape the user asked to be stopped -------------------------------

test_heredoc_rewriting_a_source_file_is_denied if {
	cmd := "python3 - <<'PY'\np='crates/er-invasion-warp/src/lib.rs'\ns=open(p,encoding='utf8').read()\ns=s.replace('a','b')\nopen(p,'w',encoding='utf8').write(s)\nPY"
	count(bash_no_python_file_write.deny) == 1 with input as bash(cmd)
}

test_inline_dash_c_write_is_denied if {
	cmd := `python3 -c "open('notes.md','w').write('x')"`
	count(bash_no_python_file_write.deny) == 1 with input as bash(cmd)
}

test_pathlib_write_text_is_denied if {
	cmd := "python3 - <<'PY'\nimport pathlib\npathlib.Path('a.rs').write_text('hi')\nPY"
	count(bash_no_python_file_write.deny) == 1 with input as bash(cmd)
}

test_uv_run_inline_write_is_denied if {
	cmd := "uv run --with capstone python3 - <<'PY'\nopen('out.txt','w').write('x')\nPY"
	count(bash_no_python_file_write.deny) == 1 with input as bash(cmd)
}

test_shutil_copy_from_inline_python_is_denied if {
	cmd := `python3 -c "import shutil; shutil.copy('a','b')"`
	count(bash_no_python_file_write.deny) == 1 with input as bash(cmd)
}

# --- reading stays cheap, which is the point ------------------------------

test_reading_a_file_is_allowed if {
	cmd := "python3 -c \"import re,glob; [print(f) for f in glob.glob('src/**/*.rs',recursive=True) if re.search('x',open(f,encoding='utf8').read())]\""
	count(bash_no_python_file_write.deny) == 0 with input as bash(cmd)
}

test_binary_read_is_allowed if {
	cmd := `python3 -c "d=open('image.bin','rb').read(); print(len(d))"`
	count(bash_no_python_file_write.deny) == 0 with input as bash(cmd)
}

test_open_with_no_mode_is_allowed if {
	cmd := `python3 -c "print(open('a.json').read())"`
	count(bash_no_python_file_write.deny) == 0 with input as bash(cmd)
}

# --- forms that stay visible in the command itself ------------------------

test_shell_redirection_is_allowed if {
	cmd := "cat > /tmp/x.txt <<'EOF'\nhello\nEOF"
	count(bash_no_python_file_write.deny) == 0 with input as bash(cmd)
}

test_committed_script_is_allowed if {
	cmd := "python3 scripts/er-dump-ersc-image.py --out vendor-archive/seamless/x.bin"
	count(bash_no_python_file_write.deny) == 0 with input as bash(cmd)
}

test_committed_script_under_uv_is_allowed if {
	cmd := "uv run --with capstone python3 scripts/ersc-xrefs.py --to 0xa96e0"
	count(bash_no_python_file_write.deny) == 0 with input as bash(cmd)
}

# A committed script named on the same line as an inline program is still an
# inline program, so the exemption must not rescue it.
test_committed_script_plus_inline_write_is_denied if {
	cmd := "python3 scripts/ok.py && python3 -c \"open('a','w').write('x')\""
	count(bash_no_python_file_write.deny) == 1 with input as bash(cmd)
}

# --- the committed-script exemption is scoped to scripts/, not any .py path -
#
# Guard gap measured 2026-09-17: the old exemption regex checked only the
# `.py` suffix, so a script run from outside the repo's tracked `scripts/`
# tree was waved through exactly like a reviewed file. Neither denied command
# below carries an inline `open(..., 'w')` on its own line -- that text lives
# inside the file being run -- which is precisely why the fix has to distrust
# the invocation itself rather than widen the text scan.

test_tmp_scratchpad_script_is_denied if {
	cmd := "python3 /tmp/claude-1000/-home-banon-projects-er-mods-rs/scratchpad/patch_actions.py"
	count(bash_no_python_file_write.deny) == 1 with input as bash(cmd)
}

# A separator abutting the path is still the same invocation.
#
# Measured against the live policy 2026-09-17, immediately after the location
# fix above landed: the bare form denied and this one did not, because the
# trailing context was whitespace-or-end and `;` is neither. The suffix is one
# the harness actively encourages -- another guard here asks for a command's
# exit code to be read as `; echo "exit=$?"` -- so this was the shape most
# likely to be typed, not an exotic one.
test_tmp_script_with_abutting_separator_is_denied if {
	some cmd in [
		`python3 /tmp/x/patch.py; echo "exit=$?"`,
		"python3 /tmp/x/patch.py&& echo hi",
		"python3 /tmp/x/patch.py|tee log",
		"(python3 /tmp/x/patch.py)",
	]
	count(bash_no_python_file_write.deny) == 1 with input as bash(cmd)
}

# The same abutting separator on a committed script must still be exempt, or
# widening the trailing class above would deny every legitimate scoped gate.
test_committed_script_with_abutting_separator_is_allowed if {
	some cmd in [
		`python3 scripts/check-comment-caps.py; echo "exit=$?"`,
		"python3 scripts/er-teardown.py > /dev/null 2>&1; python3 scripts/er-run-branch.py",
	]
	count(bash_no_python_file_write.deny) == 0 with input as bash(cmd)
}

test_home_scratch_script_is_denied if {
	cmd := "python3 ~/scratch/patch.py"
	count(bash_no_python_file_write.deny) == 1 with input as bash(cmd)
}

test_absolute_scripts_lookalike_path_is_denied if {
	# Starts with "scripts/" only after an absolute prefix -- still not
	# repo-relative, so it must not borrow the exemption.
	cmd := "python3 /home/banon/scripts/patch.py"
	count(bash_no_python_file_write.deny) == 1 with input as bash(cmd)
}

test_dotdot_escape_from_scripts_is_denied if {
	cmd := "python3 scripts/../../../tmp/patch.py"
	count(bash_no_python_file_write.deny) == 1 with input as bash(cmd)
}

test_uv_run_tmp_script_is_denied if {
	cmd := "uv run --with capstone python3 /tmp/scratch/patch.py"
	count(bash_no_python_file_write.deny) == 1 with input as bash(cmd)
}

test_relative_scripts_dir_script_is_allowed if {
	cmd := "python3 ./scripts/foo.py"
	count(bash_no_python_file_write.deny) == 0 with input as bash(cmd)
}

test_nested_scripts_subdir_script_is_allowed if {
	cmd := "python3 scripts/ghidra/mcp_query.py getContext"
	count(bash_no_python_file_write.deny) == 0 with input as bash(cmd)
}

# --- not a python command at all ------------------------------------------

test_non_python_command_is_allowed if {
	count(bash_no_python_file_write.deny) == 0 with input as bash("cargo test -p er-invasion-warp")
}

test_other_tools_are_untouched if {
	count(bash_no_python_file_write.deny) == 0 with input as {
		"hook_event_name": "PreToolUse",
		"tool_name": "Edit",
		"tool_input": {"file_path": "a.rs", "old_string": "x", "new_string": "y"},
	}
}

# --- keyword arguments are not modes -------------------------------------
#
# Added 2026-09-16, one command after the guard landed: it refused a pure read
# because `errors='replace'` begins with `r`, which a loose mode regex read as a
# python mode. A guard that blocks the work it exempts is a guard gap.

test_errors_replace_is_not_a_write if {
	cmd := `python3 -c "print(open(p, encoding='utf8', errors='replace').read())"`
	count(bash_no_python_file_write.deny) == 0 with input as bash(cmd)
}

test_encoding_keyword_is_not_a_write if {
	cmd := `python3 -c "open('a.log', encoding='utf8').read()"`
	count(bash_no_python_file_write.deny) == 0 with input as bash(cmd)
}

test_errors_replace_alongside_a_real_write_is_denied if {
	cmd := `python3 -c "s=open(p,encoding='utf8',errors='replace').read(); open(p,'w').write(s)"`
	count(bash_no_python_file_write.deny) == 1 with input as bash(cmd)
}

test_read_plus_is_a_write if {
	cmd := `python3 -c "f=open('a.bin','r+b')"`
	count(bash_no_python_file_write.deny) == 1 with input as bash(cmd)
}

test_append_mode_is_a_write if {
	cmd := `python3 -c "open('a.log','a').write('x')"`
	count(bash_no_python_file_write.deny) == 1 with input as bash(cmd)
}
