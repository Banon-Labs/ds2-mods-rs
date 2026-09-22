# OPA unit tests for the builtin protected-path policy.
# Run with:
#   opa test .cupcake/system/commands.rego \
#            .cupcake/policies/claude/builtins/protected_paths.rego \
#            .cupcake/tests/protected_paths_test.rego
package cupcake.policies.builtins.protected_paths_test

import rego.v1

import data.cupcake.policies.builtins.protected_paths as protected

bash_event(cmd, affected_dirs) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Bash",
	"tool_input": {"command": cmd, "timeout": 30000, "description": "test case"},
	"affected_parent_directories": affected_dirs,
	"builtin_config": {"protected_paths": {"message": "This path is read-only and cannot be modified", "paths": ["/System/", "/etc/", "~/.ssh/"]}},
}

rule_ids(denials) := {d.rule_id | some d in denials}

test_allow_mktemp_bd_comment_file_when_preprocessor_overapproximates_root if {
	cmd := concat("\n", [
		"tmp=$(mktemp)",
		"cat > \"$tmp\" <<'EOF'",
		"bd issue comment body",
		"EOF",
		"/home/banon/.local/bin/bd comment er-effects-rs-22h --file \"$tmp\" --json",
		"rm -f \"$tmp\"",
	])
	denials := protected.halt with input as bash_event(cmd, ["/"])
	count(denials) == 0
}

test_deny_literal_root_delete_even_with_mktemp_cleanup if {
	cmd := concat("\n", [
		"tmp=$(mktemp)",
		"cat > \"$tmp\" <<'EOF'",
		"bd issue comment body",
		"EOF",
		"/home/banon/.local/bin/bd comment er-effects-rs-22h --file \"$tmp\" --json",
		"rm -f \"$tmp\"",
		"rm -rf /",
	])
	denials := protected.halt with input as bash_event(cmd, ["/"])
	"BUILTIN-PROTECTED-PATHS-PARENT" in rule_ids(denials)
}

test_deny_literal_etc_delete_even_with_mktemp_cleanup if {
	cmd := concat("\n", [
		"tmp=$(mktemp)",
		"/home/banon/.local/bin/bd comment er-effects-rs-22h --file \"$tmp\" --json",
		"rm -f \"$tmp\" /etc/passwd",
	])
	denials := protected.halt with input as bash_event(cmd, ["/"])
	"BUILTIN-PROTECTED-PATHS-PARENT" in rule_ids(denials)
}

# A second destructive statement that names an absolute path keeps the root
# over-approximation usable, so the parent rule still denies.
test_deny_extra_absolute_delete_even_with_mktemp_cleanup if {
	cmd := concat("\n", [
		"tmp=$(mktemp)",
		"/home/banon/.local/bin/bd comment er-effects-rs-22h --file \"$tmp\" --json",
		"rm -f \"$tmp\"",
		"rm -rf /var/tmp/stale",
	])
	denials := protected.halt with input as bash_event(cmd, ["/"])
	"BUILTIN-PROTECTED-PATHS-PARENT" in rule_ids(denials)
}

# Contract change 2026-07-29: a destructive statement whose operands are all
# relative no longer inherits the preprocessor's root over-approximation. The
# old policy denied this because ANY destructive verb plus a reported `/` was
# enough; that is the same reasoning that denied a plain `git commit -F -`
# heredoc, and root is never the real target here (real preprocessing reports
# `<cwd>/relative-file`, not `/`).
test_allow_relative_delete_with_overapproximated_root if {
	cmd := concat("\n", [
		"tmp=$(mktemp)",
		"/home/banon/.local/bin/bd comment er-effects-rs-22h --file \"$tmp\" --json",
		"rm -f \"$tmp\"",
		"rm -f relative-file",
	])
	denials := protected.halt with input as bash_event(cmd, ["/"])
	count(denials) == 0
}

# --------------------------------------------------------------------------
# Heredoc payload is DATA, never a path operand.
#
# Regression for the 2026-07-29 false positive: `git add <paths> && git commit
# -q -F - <<'EOF' ...` was denied twice ("(/etc/ would be affected by operation
# on /)", "(/System/ ...)") because the preprocessor tokenized the commit
# message and reported `/` (from a bare slash between two code spans) as an
# affected parent directory, while the prose verb "install" satisfied the
# destructive-command check. Nothing in the command mutates anything but the
# index and the commit object.
# --------------------------------------------------------------------------

commit_message_heredoc_command := concat("\n", [
	"git add crates/er-quickload/src/lib.rs docs/save/save-machinery-1162.md && git commit -q -F - <<'EOF' && git log --oneline -4 && git status --porcelain",
	"save-flow: install the save-write suppression, making Save Game the only writer",
	"",
	"Both DLLs would detour `0x140e6fb50` / `0x140e6e430`, corrupting each other's",
	"trampolines, so driving System->Quit twice is the only supported path.",
	"Doc comments in scripts/check-save-disable-warnings.py were de-scoped 3/4.",
	"",
	"Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>",
	"EOF",
])

test_allow_git_commit_message_heredoc_with_prose_verb_and_bare_slash if {
	affected := [
		"/home/banon/projects/er-mods-rs/crates/er-quickload/src/lib.rs",
		"/home/banon/projects/er-mods-rs",
		"/",
		"/home/banon/projects/er-mods-rs/3/4",
	]
	denials := protected.halt with input as bash_event(commit_message_heredoc_command, affected)
	count(denials) == 0
}

# The command region AFTER a heredoc terminator is still shell: a destructive
# statement there must keep denying.
test_deny_root_delete_after_heredoc_payload if {
	cmd := concat("\n", [
		"git commit -q -F - <<'EOF'",
		"message text with a bare / in it",
		"EOF",
		"rm -rf /",
	])
	denials := protected.halt with input as bash_event(cmd, ["/"])
	"BUILTIN-PROTECTED-PATHS-PARENT" in rule_ids(denials)
}

# An unterminated heredoc must not leave payload lines readable as operands.
test_allow_unterminated_heredoc_payload if {
	cmd := concat("\n", [
		"git commit -q -F - <<'EOF'",
		"install the thing at / for good",
	])
	denials := protected.halt with input as bash_event(cmd, ["/"])
	count(denials) == 0
}

# A real destructive verb outside the heredoc, with only relative operands, is
# still not evidence of a root operation.
test_allow_relative_copy_with_overapproximated_root if {
	denials := protected.halt with input as bash_event("cp target/a.txt target/b.txt", ["/"])
	count(denials) == 0
}

# --------------------------------------------------------------------------
# Genuinely destructive operations stay blocked.
# --------------------------------------------------------------------------

test_deny_root_recursive_delete if {
	denials := protected.halt with input as bash_event("rm -rf /", ["/"])
	"BUILTIN-PROTECTED-PATHS-PARENT" in rule_ids(denials)
}

test_deny_root_glob_delete if {
	denials := protected.halt with input as bash_event("rm -rf /*", ["/"])
	"BUILTIN-PROTECTED-PATHS-PARENT" in rule_ids(denials)
}

test_deny_root_recursive_chmod if {
	denials := protected.halt with input as bash_event("chmod -R 777 /", ["/"])
	"BUILTIN-PROTECTED-PATHS-PARENT" in rule_ids(denials)
}

test_deny_etc_directory_delete if {
	denials := protected.halt with input as bash_event("rm -rf /etc", ["/etc"])
	"BUILTIN-PROTECTED-PATHS-PARENT" in rule_ids(denials)
}

test_deny_system_directory_recursive_chown if {
	denials := protected.halt with input as bash_event("chown -R root /System", ["/System"])
	"BUILTIN-PROTECTED-PATHS-PARENT" in rule_ids(denials)
}

# Real preprocessing reports the operand itself (`/etc/my.conf`), not `/etc`,
# for this shape, so the parent rule is silent and the protected-path REFERENCE
# rule owns the denial. Asserted here so the split stays visible.
test_deny_move_into_etc_by_reference_rule if {
	affected := ["/home/banon/projects/er-mods-rs/my.conf", "/etc/my.conf"]
	denials := protected.halt with input as bash_event("mv ./my.conf /etc/my.conf", affected)
	"BUILTIN-PROTECTED-PATHS" in rule_ids(denials)
}

test_deny_write_into_system_directory_by_reference_rule if {
	denials := protected.halt with input as bash_event("cp ./libfoo.dylib /System/Library/libfoo.dylib", [])
	"BUILTIN-PROTECTED-PATHS" in rule_ids(denials)
}

# Heredoc payload is excluded from OPERAND analysis only. An interpreter
# heredoc that writes a protected path is still a real write, so the reference
# rule must keep scanning body text.
test_deny_interpreter_heredoc_writing_etc if {
	cmd := concat("\n", [
		"python3 - <<'PY'",
		"open('/etc/passwd', 'w').write('x')",
		"PY",
	])
	denials := protected.halt with input as bash_event(cmd, ["/"])
	count(denials) > 0
}

# --------------------------------------------------------------------------
# Inline-script rule: repo-relative paths are not absolute protected paths.
#
# Regression for a same-family false positive hit while investigating the one
# above: reading the policy tree with `python3 -c` was denied with "(inline
# script mentions '/System/')" because `.cupcake/system/` contains "/system/"
# after case folding. The reference rule already required a path boundary; the
# inline-script rule used a bare substring test.
# --------------------------------------------------------------------------

test_allow_inline_script_reading_repo_relative_cupcake_system if {
	cmd := "python3 -c \"import glob; print(glob.glob('.cupcake/system/commands.rego'))\""
	denials := protected.halt with input as bash_event(cmd, [])
	count(denials) == 0
}

test_deny_inline_script_writing_absolute_etc if {
	cmd := "python3 -c \"open('/etc/passwd', 'w').write('x')\""
	denials := protected.halt with input as bash_event(cmd, [])
	"BUILTIN-PROTECTED-PATHS-SCRIPT" in rule_ids(denials)
}

test_deny_inline_script_writing_home_ssh if {
	cmd := "python3 -c \"open('~/.ssh/authorized_keys', 'a').write('k')\""
	denials := protected.halt with input as bash_event(cmd, [])
	"BUILTIN-PROTECTED-PATHS-SCRIPT" in rule_ids(denials)
}

# --------------------------------------------------------------------------
# THE SHAPE THE ENGINE ACTUALLY DELIVERS (2026-08-31).
#
# Every command below is written the way it ARRIVES at the policy, not the way
# an agent typed it: the engine's `whitespace_normalization` enrichment replaces
# every newline with a space before any policy runs, so a heredoc body is welded
# onto the command that reads it. Measured with `cupcake eval --debug-files`:
#
#   typed     cat > crates/demo/src/lib.rs <<'EOF'
#             /// Install the detour into the game image.
#             EOF
#   delivered cat > crates/demo/src/lib.rs <<'EOF' /// Install the detour ... EOF
#             affected_parent_directories: [".../lib.rs", "/"]
#
# The multi-line tests above therefore describe a text production never sees,
# which is why command_operand_region's line split could be inert for a month
# while its own regressions stayed green. These cases pin the delivered form.
#
# The false positive that prompted this: an agent authoring a Rust file through
# a heredoc was denied "(/System/ would be affected by operation on /)" because
# the prose word "Install" in a `///` doc comment satisfied the destructive-verb
# test and the `///` satisfied the root-path test.
# --------------------------------------------------------------------------

lib_rs_affected := ["/home/banon/projects/er-mods-rs/crates/demo/src/lib.rs", "/"]

test_allow_delivered_rust_doc_heredoc_with_prose_verb if {
	cmd := "cat > crates/demo/src/lib.rs <<'EOF' /// Install the detour into the game image. pub fn install_hook() {} EOF"
	denials := protected.halt with input as bash_event(cmd, lib_rs_affected)
	count(denials) == 0
}

test_allow_delivered_rust_doc_heredoc_with_prose_verb_truncate if {
	cmd := "cat > crates/demo/src/lib.rs <<'EOF' /// Truncate the log before writing. pub fn f() {} EOF"
	denials := protected.halt with input as bash_event(cmd, lib_rs_affected)
	count(denials) == 0
}

test_allow_delivered_line_comment_heredoc_with_prose_verb if {
	cmd := "cat > crates/demo/src/lib.rs <<'EOF' // Install the thing pub fn f() {} EOF"
	denials := protected.halt with input as bash_event(cmd, lib_rs_affected)
	count(denials) == 0
}

test_allow_delivered_markdown_heredoc_naming_an_absolute_path_in_prose if {
	cmd := "cat > docs/demo.md <<'EOF' Install the binary to /usr/local/bin when you are done. EOF"
	denials := protected.halt with input as bash_event(cmd, ["/home/banon/projects/er-mods-rs/docs/demo.md", "/"])
	count(denials) == 0
}

test_allow_delivered_url_in_heredoc_with_prose_verb if {
	cmd := "cat > docs/demo.md <<'EOF' Install from https://example.com/pkg/ before starting. EOF"
	denials := protected.halt with input as bash_event(cmd, ["/home/banon/projects/er-mods-rs/docs/demo.md", "/"])
	count(denials) == 0
}

test_allow_delivered_glob_in_heredoc_with_prose_verb if {
	cmd := "cat > docs/demo.md <<'EOF' Install step: the pattern crates/**/*.rs matches everything. EOF"
	denials := protected.halt with input as bash_event(cmd, ["/home/banon/projects/er-mods-rs/docs/demo.md", "/"])
	count(denials) == 0
}

test_allow_commit_message_operand_with_prose_verb_and_bare_slash if {
	cmd := "git commit -m \"docs: install notes now cover the / prefix rule\""
	denials := protected.halt with input as bash_event(cmd, ["/"])
	count(denials) == 0
}

# --------------------------------------------------------------------------
# ...and the same delivered shape must keep DENYING what actually runs.
# --------------------------------------------------------------------------

test_deny_delivered_destructive_after_heredoc_terminator if {
	cmd := "git commit -q -F - <<'EOF' message text with a bare / in it EOF; rm -rf /"
	denials := protected.halt with input as bash_event(cmd, ["/"])
	"BUILTIN-PROTECTED-PATHS-PARENT" in rule_ids(denials)
}

# The heredoc OPENER line keeps running after the tag, and once newlines are gone
# there is nothing to tell that remainder apart from the body. Any fix that
# located and deleted the body would have to guess, and would lose this. Command
# position does not have to guess: `rm` sits behind `&&` either way.
test_deny_delivered_destructive_on_heredoc_opener_line if {
	cmd := "cat > crates/demo/src/lib.rs <<'EOF' && rm -rf / /// Install the detour EOF"
	denials := protected.halt with input as bash_event(cmd, ["/"])
	"BUILTIN-PROTECTED-PATHS-PARENT" in rule_ids(denials)
}

test_deny_sudo_prefixed_root_delete if {
	denials := protected.halt with input as bash_event("sudo rm -rf /", ["/"])
	"BUILTIN-PROTECTED-PATHS-PARENT" in rule_ids(denials)
}

test_deny_sudo_with_option_argument_prefixed_root_delete if {
	denials := protected.halt with input as bash_event("sudo -u root rm -rf /", ["/"])
	"BUILTIN-PROTECTED-PATHS-PARENT" in rule_ids(denials)
}

test_deny_env_assignment_prefixed_root_delete if {
	denials := protected.halt with input as bash_event("env TMPDIR=/x rm -rf /", ["/"])
	"BUILTIN-PROTECTED-PATHS-PARENT" in rule_ids(denials)
}

# Absolute/relative path invocation (2026-08-31, bd er-effects-rs-5z75): the verb
# was invisible to command position when reached through a leading path component,
# so `/bin/rm -rf /` and `/usr/bin/rm -rf /` were allowed outright. Measured
# against this exact policy before the fix.
test_deny_bin_absolute_path_root_delete if {
	denials := protected.halt with input as bash_event("/bin/rm -rf /", ["/"])
	"BUILTIN-PROTECTED-PATHS-PARENT" in rule_ids(denials)
}

test_deny_usr_bin_absolute_path_root_delete if {
	denials := protected.halt with input as bash_event("/usr/bin/rm -rf /", ["/"])
	"BUILTIN-PROTECTED-PATHS-PARENT" in rule_ids(denials)
}

test_deny_dot_slash_relative_path_root_delete if {
	denials := protected.halt with input as bash_event("./rm -rf /", ["/"])
	"BUILTIN-PROTECTED-PATHS-PARENT" in rule_ids(denials)
}

# A word merely ending in the verb, reached through a path prefix, must still
# not be read as invoking it -- the trailing-slash requirement on the prefix
# group is what keeps this allowed.
test_allow_path_prefixed_word_merely_ending_in_verb if {
	denials := protected.halt with input as bash_event("scripts/confirm /", ["/"])
	count(denials) == 0
}

test_deny_root_delete_inside_command_substitution if {
	denials := protected.halt with input as bash_event("echo $(rm -rf /)", ["/"])
	"BUILTIN-PROTECTED-PATHS-PARENT" in rule_ids(denials)
}

test_deny_delivered_find_delete_rooted_at_root if {
	denials := protected.halt with input as bash_event("find / -name '*.conf' -delete", ["/"])
	"BUILTIN-PROTECTED-PATHS-PARENT" in rule_ids(denials)
}

# --------------------------------------------------------------------------
# A reported parent directory with the shell separator stuck to it.
#
# scripts/cupcake-hook.sh rewrites an unquoted newline to `; ` so line 2 is
# visible to anchored patterns; the Rust preprocessor then reports `/;` as the
# affected directory, which is neither a parent nor a child of anything. `rm -rf
# /` ALONE was denied while the same delete followed by a second line was
# ALLOWED -- measured live 2026-08-31, before and after the command-position
# change, so this hole is independent of it.
# --------------------------------------------------------------------------

test_deny_root_delete_when_affected_dir_carries_shell_separator if {
	cmd := "rm -rf /; cat > docs/demo.md <<'EOF' harmless EOF"
	denials := protected.halt with input as bash_event(cmd, ["/;"])
	"BUILTIN-PROTECTED-PATHS-PARENT" in rule_ids(denials)
}

test_deny_shell_read_heredoc_root_delete_with_separator_suffixed_dir if {
	denials := protected.halt with input as bash_event("bash <<'EOF'; rm -rf /; EOF", ["/;"])
	"BUILTIN-PROTECTED-PATHS-PARENT" in rule_ids(denials)
}

# A reported directory that is nothing BUT separators must be DROPPED, not
# normalised to the empty string. An empty affected_dir makes every protected
# path look like its child (`startswith("/etc/", "/")`) and makes every absolute
# operand look like it targets it, so the delete below -- which endangers nothing
# protected -- would be denied.
test_allow_absolute_delete_outside_protected_paths_when_affected_dir_is_only_a_separator if {
	denials := protected.halt with input as bash_event("rm -rf /home/banon/scratch", [";"])
	count(denials) == 0
}

# --------------------------------------------------------------------------
# SHELL-WRAPPER PAYLOADS (2026-08-31, BUILTIN-PROTECTED-PATHS-WRAPPER).
#
# `bash -c "rm -rf /"` was ALLOWED live before this rule -- measured through the
# real binary across 17 wrapper spellings, every one of them an allow. Two
# independent causes, both of which the rule has to answer at once:
#
#   * the verb is not in the OUTER command's command position (`bash` is), and
#     commands.has_verb could not see it either, since its `(^|\s)` anchor never
#     matched `"rm`;
#   * the engine's own `affected_parent_directories` does not contain the
#     payload's target. The preprocessor reads the quoted payload as a PATH
#     operand of `bash`, so the event carries `["<cwd>/rm -rf "]`, never `/`.
#
# The second is why these cases pass affected_dirs the parent rule cannot use:
# the wrapper rule derives the endangered directory from the payload text and
# never consults the list. The fixtures below are the values the ENGINE actually
# synthesises for these commands, measured with `cupcake eval --debug-files`, so
# the interpreter is being asked the same question production asks.
# --------------------------------------------------------------------------

wrapper_root_affected := ["/home/banon/projects/er-mods-rs/rm -rf "]

root_delete := concat(" ", ["rm", "-rf", "/"])

wrapped(prefix, suffix) := concat("", [prefix, root_delete, suffix])

test_deny_bash_c_double_quoted_root_delete if {
	denials := protected.halt with input as bash_event(wrapped(`bash -c "`, `"`), wrapper_root_affected)
	"BUILTIN-PROTECTED-PATHS-WRAPPER" in rule_ids(denials)
}

test_deny_bash_c_single_quoted_root_delete if {
	denials := protected.halt with input as bash_event(wrapped("bash -c '", "'"), wrapper_root_affected)
	"BUILTIN-PROTECTED-PATHS-WRAPPER" in rule_ids(denials)
}

test_deny_sh_c_root_delete if {
	denials := protected.halt with input as bash_event(wrapped("sh -c '", "'"), wrapper_root_affected)
	"BUILTIN-PROTECTED-PATHS-WRAPPER" in rule_ids(denials)
}

test_deny_zsh_c_root_delete if {
	denials := protected.halt with input as bash_event(wrapped("zsh -c '", "'"), wrapper_root_affected)
	"BUILTIN-PROTECTED-PATHS-WRAPPER" in rule_ids(denials)
}

# fish is the wrapper AGENTS.md tells agents to use, so it is the one that would
# have been reached for.
test_deny_fish_c_root_delete if {
	denials := protected.halt with input as bash_event(wrapped("fish -c '", "'"), wrapper_root_affected)
	"BUILTIN-PROTECTED-PATHS-WRAPPER" in rule_ids(denials)
}

test_deny_sudo_wrapped_root_delete if {
	denials := protected.halt with input as bash_event(wrapped("sudo bash -c '", "'"), wrapper_root_affected)
	"BUILTIN-PROTECTED-PATHS-WRAPPER" in rule_ids(denials)
}

test_deny_nohup_wrapped_root_delete if {
	denials := protected.halt with input as bash_event(wrapped("nohup sh -c '", "'"), wrapper_root_affected)
	"BUILTIN-PROTECTED-PATHS-WRAPPER" in rule_ids(denials)
}

test_deny_env_assignment_wrapped_root_delete if {
	denials := protected.halt with input as bash_event(wrapped("env FOO=1 bash -c '", "'"), wrapper_root_affected)
	"BUILTIN-PROTECTED-PATHS-WRAPPER" in rule_ids(denials)
}

test_deny_xargs_wrapped_root_delete if {
	denials := protected.halt with input as bash_event(wrapped("xargs -I{} sh -c '", "'"), wrapper_root_affected)
	"BUILTIN-PROTECTED-PATHS-WRAPPER" in rule_ids(denials)
}

test_deny_eval_wrapped_root_delete if {
	denials := protected.halt with input as bash_event(wrapped("eval '", "'"), wrapper_root_affected)
	"BUILTIN-PROTECTED-PATHS-WRAPPER" in rule_ids(denials)
}

# Nesting terminates: shell_payloads_deep unrolls three levels, and a fourth
# cannot exist because escaped quotes are stripped before the split.
test_deny_nested_wrapper_root_delete if {
	cmd := concat("", [`bash -c "bash -c '`, root_delete, `'"`])
	denials := protected.halt with input as bash_event(cmd, ["/home/banon/projects/er-mods-rs/bash -c 'rm -rf /'"])
	"BUILTIN-PROTECTED-PATHS-WRAPPER" in rule_ids(denials)
}

test_deny_wrapped_root_glob_delete if {
	denials := protected.halt with input as bash_event("bash -c 'rm -rf /*'", wrapper_root_affected)
	"BUILTIN-PROTECTED-PATHS-WRAPPER" in rule_ids(denials)
}

test_deny_wrapped_double_slash_delete if {
	denials := protected.halt with input as bash_event("bash -c 'rm -rf //'", wrapper_root_affected)
	"BUILTIN-PROTECTED-PATHS-WRAPPER" in rule_ids(denials)
}

test_deny_wrapped_root_recursive_chmod if {
	denials := protected.halt with input as bash_event("bash -c 'chmod -R 777 /'", ["/home/banon/projects/er-mods-rs/chmod -R 777 "])
	"BUILTIN-PROTECTED-PATHS-WRAPPER" in rule_ids(denials)
}

test_deny_wrapped_find_root_delete if {
	denials := protected.halt with input as bash_event("bash -c 'find / -name x -delete'", ["/home/banon/projects/er-mods-rs/find / -name x -delete"])
	"BUILTIN-PROTECTED-PATHS-WRAPPER" in rule_ids(denials)
}

# A payload's OWN separators still work, so the delete does not have to be first.
test_deny_wrapped_root_delete_in_second_segment if {
	cmd := concat("", ["bash -c 'echo hi; ", root_delete, "'"])
	denials := protected.halt with input as bash_event(cmd, ["/home/banon/projects/er-mods-rs/echo hi; rm -rf "])
	"BUILTIN-PROTECTED-PATHS-WRAPPER" in rule_ids(denials)
}

# --------------------------------------------------------------------------
# ...and the over-approximation this rule must NOT make.
#
# A quoted operand that is not a program stays an operand, and a payload that
# destroys something OUTSIDE every protected path stays allowed. The second half
# is what stops the ancestor `/` from turning every absolute operand into a root
# operation -- the failure mode a prefix match would have caused.
# --------------------------------------------------------------------------

test_allow_wrapped_read_of_root if {
	denials := protected.halt with input as bash_event("bash -c 'ls /'", ["/home/banon/projects/er-mods-rs/ls "])
	count(denials) == 0
}

test_allow_wrapped_relative_delete if {
	denials := protected.halt with input as bash_event("bash -c 'rm -rf target/x'", ["/home/banon/projects/er-mods-rs/rm -rf target/x"])
	count(denials) == 0
}

test_allow_wrapped_absolute_delete_outside_protected_paths if {
	denials := protected.halt with input as bash_event("bash -c 'rm -rf /home/banon/scratch'", ["/home/banon/projects/er-mods-rs/rm -rf /home/banon/scratch"])
	count(denials) == 0
}

test_allow_wrapped_absolute_copy_outside_protected_paths if {
	denials := protected.halt with input as bash_event("bash -c 'cp a.txt /home/banon/b.txt'", ["/home/banon/projects/er-mods-rs/cp a.txt /home/banon/b.txt"])
	count(denials) == 0
}

test_allow_wrapped_build if {
	denials := protected.halt with input as bash_event("bash -c 'cargo build --release'", [])
	count(denials) == 0
}

# The allow case that made command position necessary in the first place: a
# quoted MESSAGE naming a destructive word and a bare slash is not a payload,
# because `git commit -m ` is not a shell awaiting its program.
test_allow_commit_message_naming_a_verb_and_a_slash_is_not_a_payload if {
	cmd := "git commit -m \"docs: install notes now cover the / prefix rule\""
	denials := protected.halt with input as bash_event(cmd, ["/"])
	count(denials) == 0
}

# Same shape, with the delete spelled out inside an ordinary echo.
test_allow_echo_of_a_root_delete_is_not_a_payload if {
	cmd := concat("", ["echo '", root_delete, "'"])
	denials := protected.halt with input as bash_event(cmd, ["/home/banon/projects/er-mods-rs/rm -rf "])
	count(denials) == 0
}

# `python3 -c` takes a Python program, not shell: its argument is not a payload.
test_allow_python_dash_c_string_naming_a_root_delete if {
	cmd := concat("", ["python3 -c \"print('", root_delete, "')\""])
	denials := protected.halt with input as bash_event(cmd, ["/home/banon/projects/er-mods-rs/print('rm -rf /')"])
	count(denials) == 0
}

# --------------------------------------------------------------------------
# The ancestor set: bounded, and empty for anything not absolute.
# --------------------------------------------------------------------------

test_ancestors_of_an_absolute_directory if {
	protected.protected_path_ancestors("/etc/") == {"/", "/etc"}
}

test_ancestors_of_a_deep_absolute_directory if {
	protected.protected_path_ancestors("/a/b/c/") == {"/", "/a", "/a/b", "/a/b/c"}
}

test_ancestors_of_root_itself if {
	protected.protected_path_ancestors("/") == {"/"}
}

# A `~`-rooted pattern contributes NOTHING, so `bash -c 'cp file ~'` cannot start
# denying while the identical unwrapped command stays allowed.
test_no_ancestors_for_a_tilde_rooted_pattern if {
	not protected.protected_path_ancestors("~/.ssh/")
}

test_no_ancestors_for_a_relative_pattern if {
	not protected.protected_path_ancestors("src/legacy/")
}

test_allow_wrapped_copy_into_home if {
	denials := protected.halt with input as bash_event("bash -c 'cp file ~'", [])
	count(denials) == 0
}

# --------------------------------------------------------------------------
# A command-substitution delete, with the affected directory the ENGINE really
# reports rather than the one this file used to assume.
#
# test_deny_root_delete_inside_command_substitution above hand-feeds ["/"] and
# passes. The real preprocessor reports ["<cwd>/$(rm", "/)"] for that command,
# and `/)` is a parent of nothing -- so the very command the suite pinned as
# denied was ALLOWED live until separator_trimmed_dir learned to trim `)`.
# Measured 2026-08-31 through the real binary, both verdicts.
# --------------------------------------------------------------------------

test_deny_root_delete_inside_command_substitution_with_engine_reported_dirs if {
	cmd := concat("", ["echo $(", root_delete, ")"])
	denials := protected.halt with input as bash_event(cmd, ["/home/banon/projects/er-mods-rs/$(rm", "/)"])
	"BUILTIN-PROTECTED-PATHS-PARENT" in rule_ids(denials)
}

# --------------------------------------------------------------------------
# THE SHARED CASE TABLE -- one set of cases, two runners.
#
# Every entry is a command in the shape the ENGINE DELIVERS, paired with the
# `affected_parent_directories` the ENGINE actually synthesises for it (measured
# with `cupcake eval --debug-files`, never assumed) and the verdict it must get.
#
# Consumed by:
#   * test_delivered_case_table_denies / _allows below, in the OPA interpreter;
#   * scripts/test-cupcake-delivered-shape.py, which sends the same commands
#     through the REAL cupcake binary and lets the engine compute `affected`
#     itself, then asserts the fixture recorded here equals what it computed.
#
# WHY THE TABLE EXISTS. `opa test` feeds a policy whatever text the test author
# types, and this engine does not deliver that text: it collapses unquoted
# whitespace, and it OVERWRITES affected_parent_directories with its own
# preprocessor's answer whenever that answer is non-empty. So an interpreter test
# can pass on an input production never produces, and two rules in this very file
# did exactly that for a month -- command_operand_region's line split (dead,
# because no newline survives) and test_deny_root_delete_inside_command_
# substitution (green on a hand-fed ["/"] while the same command was ALLOWED
# live, because the real preprocessor reports ["<cwd>/$(rm", "/)"]).
#
# Adding a case here rather than as a lone `test_...` rule is what buys the
# second runner. A case that only ever runs in the interpreter proves nothing
# about the guard an agent actually meets.
# --------------------------------------------------------------------------

wrapper_root_dirs := ["/home/banon/projects/er-mods-rs/rm -rf "]

delivered_cases := [
	{
		"name": "wrapper-bash-c-double-quoted-root-delete",
		"command": concat("", [`bash -c "`, root_delete, `"`]),
		"affected": wrapper_root_dirs,
		"expect": "deny",
		"rule": "BUILTIN-PROTECTED-PATHS-WRAPPER",
	},
	{
		"name": "wrapper-fish-c-root-delete",
		"command": concat("", ["fish -c '", root_delete, "'"]),
		"affected": wrapper_root_dirs,
		"expect": "deny",
		"rule": "BUILTIN-PROTECTED-PATHS-WRAPPER",
	},
	{
		"name": "wrapper-nested-root-delete",
		"command": concat("", [`bash -c "bash -c '`, root_delete, `'"`]),
		"affected": ["/home/banon/projects/er-mods-rs/bash -c 'rm -rf /'"],
		"expect": "deny",
		"rule": "BUILTIN-PROTECTED-PATHS-WRAPPER",
	},
	{
		"name": "wrapper-root-glob-delete",
		"command": "bash -c 'rm -rf /*'",
		"affected": wrapper_root_dirs,
		"expect": "deny",
		"rule": "BUILTIN-PROTECTED-PATHS-WRAPPER",
	},
	{
		"name": "wrapper-find-root-delete",
		"command": "bash -c 'find / -name x -delete'",
		"affected": ["/home/banon/projects/er-mods-rs/find / -name x -delete"],
		"expect": "deny",
		"rule": "BUILTIN-PROTECTED-PATHS-WRAPPER",
	},
	{
		"name": "wrapper-absolute-delete-outside-protected-paths",
		"command": "bash -c 'rm -rf /home/banon/scratch'",
		"affected": ["/home/banon/projects/er-mods-rs/rm -rf /home/banon/scratch"],
		"expect": "allow",
		"rule": "",
	},
	{
		"name": "wrapper-read-of-root",
		"command": "bash -c 'ls /'",
		"affected": ["/home/banon/projects/er-mods-rs/ls "],
		"expect": "allow",
		"rule": "",
	},
	{
		"name": "wrapper-copy-into-home",
		"command": "bash -c 'cp file ~'",
		"affected": [],
		"expect": "allow",
		"rule": "",
	},
	{
		"name": "unwrapped-root-delete",
		"command": root_delete,
		"affected": ["/"],
		"expect": "deny",
		"rule": "BUILTIN-PROTECTED-PATHS-PARENT",
	},
	{
		"name": "command-substitution-root-delete",
		"command": concat("", ["echo $(", root_delete, ")"]),
		"affected": ["/home/banon/projects/er-mods-rs/$(rm", "/)"],
		"expect": "deny",
		"rule": "BUILTIN-PROTECTED-PATHS-PARENT",
	},
	{
		"name": "delivered-rust-doc-heredoc-with-prose-verb",
		"command": "cat > crates/demo/src/lib.rs <<'EOF' /// Install the detour into the game image. pub fn install_hook() {} EOF",
		"affected": ["/home/banon/projects/er-mods-rs/crates/demo/src/lib.rs", "/"],
		"expect": "allow",
		"rule": "",
	},
	{
		"name": "commit-message-with-prose-verb-and-bare-slash",
		"command": "git commit -m \"docs: install notes now cover the / prefix rule\"",
		"affected": ["/home/banon/projects/er-mods-rs/docs: install notes now cover the / prefix rule"],
		"expect": "allow",
		"rule": "",
	},
]

failed_delivered_denies contains case.name if {
	some case in delivered_cases
	case.expect == "deny"
	denials := protected.halt with input as bash_event(case.command, case.affected)
	not case.rule in rule_ids(denials)
}

failed_delivered_allows contains case.name if {
	some case in delivered_cases
	case.expect == "allow"
	denials := protected.halt with input as bash_event(case.command, case.affected)
	count(denials) > 0
}

test_delivered_case_table_denies if {
	failed_delivered_denies == set()
}

test_delivered_case_table_allows if {
	failed_delivered_allows == set()
}

# --------------------------------------------------------------------------
# KNOWN-OPEN RESIDUE OF THE WRAPPER RULE, PINNED SO IT IS VISIBLE.
#
# A guard that HALF-catches wrapper payloads is worse than one that visibly does
# not, because it invites reliance. These shapes are still ALLOWED on purpose.
# If one goes red the rule got stronger and the pin should FLIP -- it must never
# be deleted to keep the suite quiet.
# --------------------------------------------------------------------------

# Three levels of ESCAPED nesting. shell_payloads_deep unrolls three levels, but
# escaped quotes are stripped before the split, so the innermost payload loses
# its quoting and never becomes a text of its own. The decomposition TERMINATES
# -- which is the property that matters -- and yields `bash -c rm -rf /`, whose
# verb sits behind a `bash -c` that command position does not follow.
test_known_open_triple_nested_escaped_wrapper_payload if {
	cmd := concat("", [`bash -c "bash -c 'bash -c \"`, root_delete, `\"'"`])
	denials := protected.halt with input as bash_event(cmd, [])
	count(denials) == 0
}

# `ssh host '<program>'` is not decomposed at all: the payload runs on ANOTHER
# machine, so denying it for endangering THIS host's protected paths would be a
# guard that is wrong on purpose.
test_known_open_remote_shell_payload if {
	cmd := concat("", ["ssh host '", root_delete, "'"])
	denials := protected.halt with input as bash_event(cmd, [])
	count(denials) == 0
}

# A tilde target, wrapped or not. The preprocessor does not expand the tilde, so
# the UNWRAPPED command is allowed too; the wrapper rule declines to be stricter
# than the command it wraps.
test_known_open_wrapped_tilde_delete if {
	denials := protected.halt with input as bash_event("bash -c 'rm -rf ~'", [])
	count(denials) == 0
}
