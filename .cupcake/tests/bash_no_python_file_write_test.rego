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

# --- the absolute-path form of a committed script is also allowed ---------
#
# Conflict measured 2026-09-22: AGENTS.md tells agents to use absolute paths
# because the harness resets Bash's cwd between calls, so `cd scripts &&
# python3 foo.py` from one call cannot be relied on for the next. The old
# committed-script regex only recognized the repo-relative spelling
# (`scripts/<name>.py`), so it blocked the exact invocation shape the
# environment's own instructions recommend, on a script this repo has
# already committed. The fix compares the absolute path against `input.cwd`
# (the session's actual repo root on this call) rather than a hardcoded
# guess, and only when that prefix is followed immediately by `/scripts/`.

repo_root := "/home/banon/projects/ds2-mods-rs"

bash_cwd(cmd, cwd) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Bash",
	"tool_input": {"command": cmd},
	"cwd": cwd,
}

test_absolute_repo_script_is_allowed if {
	cmd := sprintf("python3 %s/scripts/ds2-ebl.py info", [repo_root])
	count(bash_no_python_file_write.deny) == 0 with input as bash_cwd(cmd, repo_root)
}

test_absolute_repo_script_nested_is_allowed if {
	cmd := sprintf("python3 %s/scripts/ghidra/mcp_query.py getContext", [repo_root])
	count(bash_no_python_file_write.deny) == 0 with input as bash_cwd(cmd, repo_root)
}

test_absolute_repo_script_with_abutting_separator_is_allowed if {
	cmd := sprintf(`python3 %s/scripts/ds2-ebl.py; echo "exit=$?"`, [repo_root])
	count(bash_no_python_file_write.deny) == 0 with input as bash_cwd(cmd, repo_root)
}

# The existing repo-relative form must still be allowed once `cwd` is present
# on the input -- the new absolute-path branch must not have displaced it.
test_repo_relative_script_with_cwd_set_is_allowed if {
	cmd := "python3 scripts/ds2-ebl.py info"
	count(bash_no_python_file_write.deny) == 0 with input as bash_cwd(cmd, repo_root)
}

# A `/tmp` script must still be denied even once `cwd` is set on the input --
# the absolute-path exemption is prefix-scoped to `cwd`, not "any absolute
# path with a .py suffix".
test_absolute_tmp_script_with_cwd_set_is_denied if {
	cmd := "python3 /tmp/claude-scratch/patch.py"
	count(bash_no_python_file_write.deny) == 1 with input as bash_cwd(cmd, repo_root)
}

# A `~`-relative script must still be denied with `cwd` set.
test_home_relative_script_with_cwd_set_is_denied if {
	cmd := "python3 ~/foo.py"
	count(bash_no_python_file_write.deny) == 1 with input as bash_cwd(cmd, repo_root)
}

# Inline `-c` and heredoc programs must still be denied with `cwd` set -- the
# absolute-path exemption only ever widens FILE invocations, never inline
# code.
test_inline_dash_c_with_cwd_set_is_denied if {
	cmd := `python3 -c "open('notes.md','w').write('x')"`
	count(bash_no_python_file_write.deny) == 1 with input as bash_cwd(cmd, repo_root)
}

test_heredoc_with_cwd_set_is_denied if {
	cmd := "python3 - <<'PY'\nopen('a.txt','w').write('x')\nPY"
	count(bash_no_python_file_write.deny) == 1 with input as bash_cwd(cmd, repo_root)
}

# A path that merely CONTAINS "/scripts/" without being rooted at THIS
# session's cwd must not borrow the exemption -- otherwise any repo's
# `scripts/` tree would qualify, not just this one.
test_absolute_script_under_different_repo_is_denied if {
	cmd := "python3 /home/banon/projects/other-repo/scripts/patch.py"
	count(bash_no_python_file_write.deny) == 1 with input as bash_cwd(cmd, repo_root)
}

# A `scripts`-lookalike absolute path with no cwd on the input at all must
# fail closed: there is no known repo root to compare the prefix against, so
# no absolute path can be trusted, even one that would have matched had
# `cwd` been set.
test_absolute_repo_script_without_cwd_input_is_denied if {
	cmd := sprintf("python3 %s/scripts/ds2-ebl.py info", [repo_root])
	count(bash_no_python_file_write.deny) == 1 with input as bash(cmd)
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

# --- SPELLING IS NOT IDENTITY (2026-09-23) --------------------------------
#
# The exemption used to recognise one spelling of a committed script,
# `(./)?scripts/<name>.py`, and deny every other way of naming the SAME FILE.
# That collided head-on with a standing instruction in the user's global
# AGENTS.md about launch commands -- "use ABSOLUTE paths in every launch command
# -- never `./script`, because cwd is not yours and resets between calls" -- so
# the guard and the instructions could not both be obeyed. Measured through
# scripts/cupcake-hook.sh that day, the command in
# test_absolute_committed_script_launch_is_allowed below was DENIED as a python
# file write: a launch of a committed, reviewed, testimony-gated script.
#
# The widening is to the FILE, not to path shapes. Every test below that ALLOWS a
# new spelling has a sibling that DENIES the lookalike it resembles, because
# "tail looks like /scripts/<name>.py" is one `mkdir -p` away from being an
# escape hatch and that is the half of this change that can go wrong.

repo_root_fixture := "/home/banon/projects/ds2-mods-rs"

home_fixture := "/home/banon"

# A PreToolUse event as the harness and .cupcake/signals/repo_paths.sh actually
# deliver it: cwd from Claude Code, and the signal's two lines (repo root, $HOME).
bash_in_repo(cmd) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Bash",
	"cwd": repo_root_fixture,
	"signals": {"repo_paths": concat("\n", [repo_root_fixture, home_fixture])},
	"tool_input": {"command": cmd},
}

# Signal present, cwd absent -- the signal alone must carry the exemption, since
# it is the authoritative source (it resolves the checkout that owns the
# policies, not whichever directory the session happens to sit in).
bash_signal_only(cmd) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Bash",
	"signals": {"repo_paths": concat("\n", [repo_root_fixture, home_fixture])},
	"tool_input": {"command": cmd},
}

# cwd present, signal absent -- the degrade path. A signal that fails must not
# resurrect the deny that started this.
bash_cwd_only(cmd) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Bash",
	"cwd": repo_root_fixture,
	"tool_input": {"command": cmd},
}

# An arbitrary signal payload with no cwd, for the fail-closed cases.
bash_signal_text(cmd, text) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Bash",
	"signals": {"repo_paths": text},
	"tool_input": {"command": cmd},
}

# --- the exact command this change exists to unblock ----------------------

test_absolute_committed_script_launch_is_allowed if {
	cmd := "cd /home/banon/projects/ds2-mods-rs; python3 /home/banon/projects/ds2-mods-rs/scripts/ds2-run.py --rows load-character-from-file,save-game-to-file"
	count(bash_no_python_file_write.deny) == 0 with input as bash_in_repo(cmd)
}

test_absolute_committed_script_is_allowed_from_the_signal_alone if {
	cmd := "python3 /home/banon/projects/ds2-mods-rs/scripts/ds2-run.py"
	count(bash_no_python_file_write.deny) == 0 with input as bash_signal_only(cmd)
}

test_absolute_committed_script_is_allowed_from_cwd_alone if {
	cmd := "python3 /home/banon/projects/ds2-mods-rs/scripts/ds2-run.py"
	count(bash_no_python_file_write.deny) == 0 with input as bash_cwd_only(cmd)
}

# With NO root from either source the absolute spelling cannot be resolved, and
# an unresolvable path is refused rather than assumed. Pinned so the degrade
# direction is a measured property and not a hope.
test_absolute_committed_script_is_denied_with_no_root_at_all if {
	cmd := "python3 /home/banon/projects/ds2-mods-rs/scripts/ds2-run.py"
	count(bash_no_python_file_write.deny) == 1 with input as bash(cmd)
}

test_absolute_committed_script_with_abutting_separator_is_allowed if {
	some cmd in [
		`python3 /home/banon/projects/ds2-mods-rs/scripts/check-comment-caps.py; echo "exit=$?"`,
		"python3 /home/banon/projects/ds2-mods-rs/scripts/ds2-teardown.py > /dev/null 2>&1; python3 /home/banon/projects/ds2-mods-rs/scripts/ds2-run.py",
		"(python3 /home/banon/projects/ds2-mods-rs/scripts/ds2-run.py)",
	]
	count(bash_no_python_file_write.deny) == 0 with input as bash_in_repo(cmd)
}

test_absolute_committed_script_in_a_subdir_is_allowed if {
	cmd := "python3 /home/banon/projects/ds2-mods-rs/scripts/ghidra/mcp_query.py getContext"
	count(bash_no_python_file_write.deny) == 0 with input as bash_in_repo(cmd)
}

test_absolute_committed_script_under_uv_is_allowed if {
	cmd := "uv run --with capstone python3 /home/banon/projects/ds2-mods-rs/scripts/ds2-xrefs.py --to 0xa96e0"
	count(bash_no_python_file_write.deny) == 0 with input as bash_in_repo(cmd)
}

# --- the other spellings of the same file ---------------------------------

# $CLAUDE_PROJECT_DIR is the harness's own name for the repo root, so it resolves
# symbolically -- no signal, no cwd, nothing for a root lookup to disagree with.
test_claude_project_dir_spelling_is_allowed if {
	some cmd in [
		"python3 $CLAUDE_PROJECT_DIR/scripts/ds2-run.py",
		"python3 ${CLAUDE_PROJECT_DIR}/scripts/ghidra/mcp_query.py getContext",
		`python3 "$CLAUDE_PROJECT_DIR/scripts/ds2-run.py" --rows a,b`,
	]
	count(bash_no_python_file_write.deny) == 0 with input as bash(cmd)
}

test_home_relative_committed_script_is_allowed if {
	cmd := "python3 ~/projects/ds2-mods-rs/scripts/ds2-run.py --rows a,b"
	count(bash_no_python_file_write.deny) == 0 with input as bash_in_repo(cmd)
}

# $HOME comes only from the signal, so `~` cannot be expanded without it and the
# spelling denies. Same degrade direction as the absolute case above.
test_home_relative_committed_script_is_denied_without_the_signal if {
	cmd := "python3 ~/projects/ds2-mods-rs/scripts/ds2-run.py"
	count(bash_no_python_file_write.deny) == 1 with input as bash_cwd_only(cmd)
}

# A quote belongs to the shell, not to the path.
test_quoted_committed_script_is_allowed if {
	some cmd in [
		`python3 'scripts/ds2-run.py'`,
		`python3 "/home/banon/projects/ds2-mods-rs/scripts/ds2-run.py"`,
	]
	count(bash_no_python_file_write.deny) == 0 with input as bash_in_repo(cmd)
}

# `..` is now refused on the resolved path instead of anywhere in the command, so
# a committed script may take one as an ARGUMENT -- it writes nothing this guard
# can see, and denying it was the guard fighting the work it exempts.
test_dotdot_in_an_argument_to_a_committed_script_is_allowed if {
	cmd := "python3 scripts/ds2-dump.py --out ../build/dump.bin"
	count(bash_no_python_file_write.deny) == 0 with input as bash_in_repo(cmd)
}

# --- the lookalikes, which is the half that matters ----------------------

# One `mkdir -p /tmp/ds2-mods-rs/scripts` is all a tail-shaped exemption would
# have cost. The root is compared as a PREFIX, so this is not this repo.
test_lookalike_repo_name_outside_the_repo_is_denied if {
	some cmd in [
		"python3 /tmp/ds2-mods-rs/scripts/patch.py",
		"python3 /tmp/claude-1000/-home-banon-projects-ds2-mods-rs/scratchpad/scripts/patch.py",
		"python3 ~/ds2-mods-rs/scripts/patch.py",
	]
	count(bash_no_python_file_write.deny) == 1 with input as bash_in_repo(cmd)
}

# A sibling directory whose name merely STARTS with the root's. The trailing
# slash on the prefix is what makes this a deny; without it, `<root>-evil` reads
# as inside the repo.
test_sibling_directory_sharing_the_root_prefix_is_denied if {
	cmd := "python3 /home/banon/projects/ds2-mods-rs-evil/scripts/patch.py"
	count(bash_no_python_file_write.deny) == 1 with input as bash_in_repo(cmd)
}

# The 2026-09-17 cases again, this time with the roots actually resolved -- the
# condition under which the new spellings are accepted. Their bare-input
# siblings above prove nothing about this one.
test_scratchpad_and_home_scripts_stay_denied_with_roots_resolved if {
	some cmd in [
		"python3 /tmp/claude-1000/-home-banon-projects-ds2-mods-rs/scratchpad/patch_actions.py",
		"python3 /home/banon/scripts/patch.py",
		"python3 ~/scratch/patch.py",
		`python3 /tmp/x/patch.py; echo "exit=$?"`,
		"uv run --with capstone python3 /tmp/scratch/patch.py",
	]
	count(bash_no_python_file_write.deny) == 1 with input as bash_in_repo(cmd)
}

# Inside the repo but outside scripts/: the exemption is for the reviewed script
# tree, not for the checkout.
test_repo_path_outside_scripts_is_denied if {
	some cmd in [
		"python3 /home/banon/projects/ds2-mods-rs/crates/tool/patch.py",
		"python3 /home/banon/projects/ds2-mods-rs/scriptsx/patch.py",
		"python3 $CLAUDE_PROJECT_DIR/crates/tool/patch.py",
	]
	count(bash_no_python_file_write.deny) == 1 with input as bash_in_repo(cmd)
}

# `..` still escapes, in every spelling that now resolves.
test_dotdot_escape_is_denied_in_every_spelling if {
	some cmd in [
		"python3 scripts/../../../tmp/patch.py",
		"python3 /home/banon/projects/ds2-mods-rs/scripts/../../../tmp/patch.py",
		"python3 $CLAUDE_PROJECT_DIR/scripts/../../tmp/patch.py",
		"python3 ~/projects/ds2-mods-rs/scripts/../../tmp/patch.py",
	]
	count(bash_no_python_file_write.deny) == 1 with input as bash_in_repo(cmd)
}

# A shell variable this policy cannot resolve is not a resolved path. Only
# $CLAUDE_PROJECT_DIR is understood, because only it is defined to BE the root.
test_unresolvable_variable_prefix_is_denied if {
	some cmd in [
		"python3 $HOME/scripts/patch.py",
		"python3 $REPO/scripts/patch.py",
	]
	count(bash_no_python_file_write.deny) == 1 with input as bash_in_repo(cmd)
}

# `/` as a root would exempt every /<anything>/scripts/*.py on the filesystem, so
# a signal emitting it contributes no root at all. Same for a signal emitting an
# error message instead of a path -- a broken signal must lose spellings, never
# gain them.
test_degenerate_or_broken_signal_contributes_no_root if {
	some text in ["/\n/", "fatal: not a git repository\n", "\n", ""]
	count(bash_no_python_file_write.deny) == 1 with input as bash_signal_text("python3 /scripts/patch.py", text)
	count(bash_no_python_file_write.deny) == 1 with input as bash_signal_text("python3 /home/banon/projects/ds2-mods-rs/scripts/ds2-run.py", text)
}

# --- EVERY path must be committed, not merely one of them -----------------
#
# The old rule asked "does this command name a committed script ANYWHERE", which
# a second invocation sails past. That hole predates the widening; widening the
# accepted spellings is what would have made it comfortable to walk through.

test_committed_script_beside_a_scratch_script_is_denied if {
	some cmd in [
		"python3 scripts/ok.py; python3 /tmp/patch.py",
		"python3 /home/banon/projects/ds2-mods-rs/scripts/ds2-run.py && python3 /tmp/patch.py",
		"python3 /tmp/patch.py && python3 $CLAUDE_PROJECT_DIR/scripts/ok.py",
	]
	count(bash_no_python_file_write.deny) == 1 with input as bash_in_repo(cmd)
}

# --- an inline program is never exempt, in any spelling -------------------

test_absolute_committed_script_plus_inline_write_is_denied if {
	some cmd in [
		"python3 /home/banon/projects/ds2-mods-rs/scripts/ds2-run.py && python3 -c \"open('a','w').write('x')\"",
		"python3 $CLAUDE_PROJECT_DIR/scripts/ds2-run.py; python3 -c \"import pathlib; pathlib.Path('a.rs').write_text('x')\"",
		"python3 ~/projects/ds2-mods-rs/scripts/ds2-run.py && python3 -c \"import shutil; shutil.copy('a','b')\"",
	]
	count(bash_no_python_file_write.deny) == 1 with input as bash_in_repo(cmd)
}

test_absolute_committed_script_plus_heredoc_write_is_denied if {
	cmd := "python3 /home/banon/projects/ds2-mods-rs/scripts/ds2-run.py; python3 - <<'PY'\np='crates/ds2-loader/src/lib.rs'\nopen(p,'w').write('x')\nPY"
	count(bash_no_python_file_write.deny) == 1 with input as bash_in_repo(cmd)
}

test_absolute_committed_script_plus_stdin_program_is_denied if {
	cmd := `echo "open('a','w').write('x')" | python3 - ; python3 /home/banon/projects/ds2-mods-rs/scripts/ds2-run.py`
	count(bash_no_python_file_write.deny) == 1 with input as bash_in_repo(cmd)
}

# KNOWN OPEN, and older than this change: a program piped in from a FILE
# (`cat /tmp/patch.py | python3 -`) carries no write text on the command line and
# is not a `.py` file operand either, so neither deny rule can see it. The
# `python3 -` disqualifier only stops such a command from borrowing the
# exemption -- it does not deny it. Filed as ds2-mods-rs-9m8. Pinned as a
# FAILING-CLOSED-NOWHERE fact rather than left to be rediscovered: this test
# asserts today's behaviour, so closing the gap will turn it red on purpose.
test_stdin_program_piped_from_a_file_is_a_known_gap if {
	cmd := "cat /tmp/patch.py | python3 -"
	count(bash_no_python_file_write.deny) == 0 with input as bash_in_repo(cmd)
}

# The other two gaps of the same age and the same kind -- an invocation the
# script-file rule never SEES, as opposed to a path it wrongly exempts. Pinned
# for the same reason: a red test is how the next change announces it closed one.
#
#   * a flag between the interpreter and the path displaces the operand
#     script_file_pattern reads. Widening it to skip flags would read
#     `python3 -m pytest tests/x.py` as a script-file invocation and deny an
#     ordinary test run, so it needs a module-aware form, not a looser one.
#   * the `-c` disqualifier is command-WIDE, so a trailing ` -c x` -- which may be
#     the SCRIPT's own flag -- disqualifies the whole command from the rule.
test_invocations_the_script_file_rule_does_not_see_are_known_gaps if {
	some cmd in [
		"python3 -u /tmp/patch.py",
		"python3 /tmp/patch.py -c fixtures.json",
	]
	count(bash_no_python_file_write.deny) == 0 with input as bash_in_repo(cmd)
}

# Reading stays cheap under the widened exemption too -- the write patterns are
# what deny, and a read names no mode.
test_reading_a_file_stays_allowed_with_roots_resolved if {
	cmd := `python3 -c "print(open('/home/banon/projects/ds2-mods-rs/Cargo.toml').read())"`
	count(bash_no_python_file_write.deny) == 0 with input as bash_in_repo(cmd)
}
