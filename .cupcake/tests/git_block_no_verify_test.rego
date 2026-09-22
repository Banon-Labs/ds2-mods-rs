# OPA unit tests for the builtin git no-verify policy.
# Run with:
#   opa test .cupcake/system/commands.rego \
#            .cupcake/policies/claude/builtins/git_block_no_verify.rego \
#            .cupcake/tests/git_block_no_verify_test.rego
package cupcake.policies.builtins.git_block_no_verify_test

import rego.v1

import data.cupcake.policies.builtins.git_block_no_verify as guard

bash_event(cmd) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Bash",
	"tool_input": {"command": cmd, "timeout": 30000},
}

rule_ids(denials) := {d.rule_id | some d in denials}

test_deny_git_commit_no_verify_long if {
	denials := guard.deny with input as bash_event("git commit --no-verify -m bad")
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_git_commit_no_verify_short if {
	denials := guard.deny with input as bash_event("git commit -nm bad")
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_git_dash_c_commit_no_verify_short if {
	denials := guard.deny with input as bash_event("git -C /tmp/repo commit -n -m bad")
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_allow_git_commit_help_piped_to_sed_n if {
	denials := guard.deny with input as bash_event("git commit -h 2>&1 | sed -n '1,80p'")
	count(denials) == 0
}

test_allow_git_commit_then_later_sed_n if {
	denials := guard.deny with input as bash_event("git commit -m ok && sed -n '1,80p' file")
	count(denials) == 0
}

# --- Shell-wrapper payloads (2026-08-26, bd er-effects-rs-dt2e) ---------------
#
# This policy recognises its verbs through commands.has_verb, whose
# `(^|\s)verb(\s|$)` boundary a quote does not satisfy -- so a hook bypass
# wrapped in `bash -c` produced ZERO denials, measured. The scan set is the
# executed-text decomposition UNION the raw command, so this is strictly
# additive: nothing this policy used to deny stopped being denied.
#
# The flag is assembled rather than written whole so that editing this file
# through a Bash command is not itself denied by the unanchored
# `contains(cmd, ...)` test these cases exercise -- which is this policy's own
# false positive, filed rather than changed here (it is a vendored builtin).
no_verify := concat("", ["--no", "-verify"])

test_deny_wrapped_no_verify_commit if {
	denials := guard.deny with input as bash_event(concat("", ["bash -c 'git commit ", no_verify, " -m bad'"]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_wrapped_no_verify_push if {
	denials := guard.deny with input as bash_event(concat("", [`sh -c "git push `, no_verify, ` origin x"`]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_wrapped_no_verify_merge if {
	denials := guard.deny with input as bash_event(concat("", ["bash -lc 'git merge ", no_verify, " topic'"]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_wrapped_short_no_verify_commit if {
	denials := guard.deny with input as bash_event("bash -c 'git commit -nm bad'")
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_nested_wrapped_no_verify_commit if {
	denials := guard.deny with input as bash_event(concat("", [`bash -c 'bash -c "git commit `, no_verify, ` -m bad"'`]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_eval_no_verify_commit if {
	denials := guard.deny with input as bash_event(concat("", [`eval "git commit `, no_verify, ` -m bad"`]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

# --- core.hooksPath: the two tokens must be ONE assignment (2026-08-31) -------
#
# The rule used to AND `contains(cmd, "core.hookspath")` with
# `contains(cmd, "/dev/null")` over the WHOLE command string, so a command that
# INSTALLED hooks and merely redirected something unrelated to /dev/null was
# denied as "Disabling git hooks is not permitted" -- measured live, while an
# agent was repairing a core.hooksPath that had silently disabled every hook in
# this repository since the er-effects-rs -> er-mods-rs rename.
#
# Both halves are tested, because dropping the `/dev/null` half would have been
# the easy wrong fix: `git config core.hooksPath /dev/null` really does disable
# hooks and must stay denied.
#
# The two tokens are ASSEMBLED for the same reason `no_verify` above is: written
# whole, they are exactly what this rule denies, so an agent editing this file
# through a Bash heredoc would be blocked by the rule the file tests. (The two
# pre-existing cases below were written whole and had that problem.)
hooks_path := concat("", ["core.hooks", "Path"])

dev_null := concat("", ["/dev/", "null"])

# The hooksPath rule shipped comparing a mixed-case literal against a lowercased
# command, so it could never fire. Pinned here now that it can.
test_deny_hooks_path_to_dev_null if {
	denials := guard.deny with input as bash_event(concat(" ", ["git config", hooks_path, dev_null]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_wrapped_hooks_path_to_dev_null if {
	denials := guard.deny with input as bash_event(concat("", [`bash -c "git config `, hooks_path, " ", dev_null, `"`]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_hooks_path_global_to_dev_null if {
	denials := guard.deny with input as bash_event(concat(" ", ["git config --global", hooks_path, dev_null]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_hooks_path_worktree_to_dev_null if {
	denials := guard.deny with input as bash_event(concat(" ", ["git config --worktree", hooks_path, dev_null]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

# A quoted value: commands.scan_text blanks a quoted span's ANCHOR characters but
# leaves the quote characters themselves in place, so the gap between key and
# value really does contain a `"` in the text the rule reads.
test_deny_hooks_path_quoted_value if {
	denials := guard.deny with input as bash_event(concat("", ["git config ", hooks_path, ` "`, dev_null, `"`]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

# A trailing backslash JOINS the two lines. scripts/cupcake-hook.sh leaves such a
# newline alone (it is not a command boundary) and the engine then collapses it to
# a space, so the key and the value are still one assignment.
test_deny_hooks_path_line_continuation if {
	denials := guard.deny with input as bash_event(concat("", ["git config ", hooks_path, " \\\n    ", dev_null]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

# `git -c key=value` carries no `config` verb, so the old rule ALLOWED it outright
# -- measured through the real binary before this change.
test_deny_git_dash_c_hooks_path_inline if {
	denials := guard.deny with input as bash_event(concat("", ["git -c ", hooks_path, "=", dev_null, " commit -m x"]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_absolute_git_dash_c_hooks_path_inline if {
	denials := guard.deny with input as bash_event(concat("", ["/usr/bin/git -c ", hooks_path, "=", dev_null, " push origin topic"]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

# The GIT_CONFIG_* environment form puts the key and the value in SEPARATE
# assignments, so no same-assignment pattern can reach it; it has its own clause,
# which requires the captured indices to match.
test_deny_git_config_env_same_index if {
	denials := guard.deny with input as bash_event(concat("", [
		"GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=", hooks_path,
		" GIT_CONFIG_VALUE_0=", dev_null, " git status",
	]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

# --- the repair direction, which the old rule denied --------------------------

test_allow_hooks_path_repair_with_unrelated_dev_null_redirect if {
	denials := guard.deny with input as bash_event(concat("", ["git config ", hooks_path, " scripts/hooks && git config --get ", hooks_path, " >", dev_null]))
	count(denials) == 0
}

test_allow_hooks_path_install_with_stderr_silenced if {
	denials := guard.deny with input as bash_event(concat("", ["git config ", hooks_path, " scripts/hooks 2>", dev_null]))
	count(denials) == 0
}

test_allow_hooks_path_read_redirected_to_dev_null if {
	denials := guard.deny with input as bash_event(concat("", ["git config --get ", hooks_path, " >", dev_null]))
	count(denials) == 0
}

test_allow_hooks_path_read_piped_to_tee_dev_null if {
	denials := guard.deny with input as bash_event(concat("", ["git config --get ", hooks_path, " | tee ", dev_null]))
	count(denials) == 0
}

# Two separate commands joined by the shim's `; `. A separator in the gap means
# the tokens belong to different commands, which is the whole distinction.
test_allow_hooks_path_repair_then_read_across_statements if {
	denials := guard.deny with input as bash_event(concat("", ["git config ", hooks_path, " scripts/hooks; git config --get ", hooks_path, " >", dev_null]))
	count(denials) == 0
}

test_allow_git_config_env_installing_hooks if {
	denials := guard.deny with input as bash_event(concat("", [
		"GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=", hooks_path,
		" GIT_CONFIG_VALUE_0=scripts/hooks git status",
	]))
	count(denials) == 0
}

# hooksPath is set at index 0 and an UNRELATED key is discarded at index 1. The
# index capture is what keeps this out of the deny set.
test_allow_git_config_env_mismatched_indices if {
	denials := guard.deny with input as bash_event(concat("", [
		"GIT_CONFIG_COUNT=2 GIT_CONFIG_KEY_0=", hooks_path,
		" GIT_CONFIG_VALUE_0=scripts/hooks GIT_CONFIG_KEY_1=core.pager GIT_CONFIG_VALUE_1=",
		dev_null, " git status",
	]))
	count(denials) == 0
}

# --- hook removal: the verb and the hook directory must be ONE statement -------
#   (2026-08-31, bd er-effects-rs-c0t9)
#
# Three arms of contains_hook_disable used to AND a verb test with
# `contains(cmd, "<hooks dir>")` over the WHOLE command string -- the same
# co-presence defect the core.hooksPath block above was narrowed to fix, one
# screen further down the same file. It denied an install:
#
#     rm -rf <unrelated dir> && mkdir -p <hooks dir> && cp shim <hooks dir>/pre-push
#
# and then denied the bug report about that denial, because the report quoted
# both tokens. Measured live, twice, on 2026-08-31.
#
# THE SENSITIVITY CASES COME FIRST ON PURPOSE. A false-positive fix that makes
# real hook removal allowable is a strictly worse bug than the one it fixes, so
# the deny direction is pinned in more detail than the allow direction, and two
# of these cases are here because they went RED against an earlier draft of the
# patch: `echo <hook> | xargs rm -f` and `find <hooks dir> | xargs rm` are real
# deletions whose path and verb sit in DIFFERENT pipeline stages, which is why
# the rule reads shell_statements (pipelines whole) rather than shell_segments.
#
# Assembled rather than written whole, for the reason given above `no_verify`.
hooks_dir := concat("", [".git/", "hooks"])

# ---- MUST STAY DENIED --------------------------------------------------------

test_deny_rm_the_hooks_directory if {
	denials := guard.deny with input as bash_event(concat("", ["rm -rf ", hooks_dir]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_rm_a_single_hook_file if {
	denials := guard.deny with input as bash_event(concat("", ["rm -f ", hooks_dir, "/pre-commit"]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

# has_command_verb steps over wrapper words, so tightening to command position
# cannot lose a real invocation hiding behind one.
test_deny_rm_behind_sudo if {
	denials := guard.deny with input as bash_event(concat("", ["sudo rm -rf ", hooks_dir]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_rm_behind_an_env_assignment if {
	denials := guard.deny with input as bash_event(concat("", ["TMPDIR=/x rm -rf ", hooks_dir]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

# The verb reached by absolute/relative path (2026-08-31, bd er-effects-rs-5z75):
# a leading path component is none of command_position_prefix_pattern's anchor
# characters, so `/bin/rm -rf <hooks dir>` was measured ALLOWED before the fix.
test_deny_rm_behind_bin_absolute_path if {
	denials := guard.deny with input as bash_event(concat("", ["/bin/rm -rf ", hooks_dir]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_rm_behind_usr_bin_absolute_path if {
	denials := guard.deny with input as bash_event(concat("", ["/usr/bin/rm -rf ", hooks_dir]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_rm_behind_dot_slash_relative_path if {
	denials := guard.deny with input as bash_event(concat("", ["./rm -rf ", hooks_dir]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

# The removal is in the SECOND statement. Per-statement matching must find it
# there, not only at the start of the command.
test_deny_rm_in_a_later_statement if {
	denials := guard.deny with input as bash_event(concat("", ["cd /tmp/lab && rm -rf ", hooks_dir]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

# The PATH is read from the raw statement, quotes and all, precisely so that
# quoting an operand -- which is ordinary -- does not hide it.
test_deny_rm_with_a_double_quoted_path if {
	denials := guard.deny with input as bash_event(concat("", [`rm -rf "$repo/`, hooks_dir, `"`]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_rm_with_a_single_quoted_path if {
	denials := guard.deny with input as bash_event(concat("", ["rm -rf '", hooks_dir, "'"]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_unlink_a_hook if {
	denials := guard.deny with input as bash_event(concat("", ["unlink ", hooks_dir, "/pre-push"]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_mv_the_hooks_directory_away if {
	denials := guard.deny with input as bash_event(concat("", ["mv ", hooks_dir, " ", hooks_dir, ".off"]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_chmod_a_hook_non_executable if {
	denials := guard.deny with input as bash_event(concat("", ["chmod -x ", hooks_dir, "/pre-commit"]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_rm_inside_a_subshell if {
	denials := guard.deny with input as bash_event(concat("", ["( rm -rf ", hooks_dir, " )"]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_mv_inside_a_loop_body if {
	denials := guard.deny with input as bash_event(concat("", ["for f in a; do mv ", hooks_dir, " /tmp/x; done"]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

# `fish -c` bypassed every executed-text guard until 2026-08-31; the statement
# set is built over commands.executed_texts so a wrapper payload is decomposed
# and then segmented on its own.
test_deny_rm_inside_a_fish_wrapper if {
	denials := guard.deny with input as bash_event(concat("", ["fish -c 'rm -rf ", hooks_dir, "'"]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_rm_inside_a_bash_wrapper if {
	denials := guard.deny with input as bash_event(concat("", [`bash -c "rm -rf `, hooks_dir, `"`]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_rm_inside_a_nested_wrapper if {
	denials := guard.deny with input as bash_event(concat("", [`bash -c 'bash -c "rm -rf `, hooks_dir, `"'`]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

# THE PIPELINE PAIR. Path in stage one, verb in stage two, hook deleted all the
# same. Both of these were ALLOWED by a segment-granularity draft of this patch,
# which is the whole reason shell_statements exists.
test_deny_echo_path_piped_to_xargs_rm if {
	denials := guard.deny with input as bash_event(concat("", ["echo ", hooks_dir, "/pre-commit | xargs rm -f"]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_find_piped_to_xargs_rm if {
	denials := guard.deny with input as bash_event(concat("", ["find ", hooks_dir, " -type f | xargs rm"]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

# `-exec rm` puts the verb after an option, where command position cannot see it.
# The old whole-string has_verb caught this shape by accident; the find arm
# catches it deliberately, and adds `-delete`, which nothing caught before.
test_deny_find_exec_rm if {
	denials := guard.deny with input as bash_event(concat("", ["find ", hooks_dir, ` -type f -exec rm {} \;`]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_deny_find_delete if {
	denials := guard.deny with input as bash_event(concat("", ["find ", hooks_dir, " -type f -delete"]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

# ---- MUST NOW BE ALLOWED -----------------------------------------------------

# THE REPORTED FALSE POSITIVE, verbatim in shape: a removal aimed at one path in
# one statement, a hooks directory named in two others, and the command as a
# whole INSTALLS a hook.
test_allow_unrelated_removal_then_installing_a_hook if {
	denials := guard.deny with input as bash_event(concat("", [
		"rm -rf /tmp/lab && mkdir -p /tmp/lab/", hooks_dir,
		" && cp shim /tmp/lab/", hooks_dir, "/pre-push",
	]))
	count(denials) == 0
}

# THE SECOND HIT: the bug report about the first. Both tokens appear, inside a
# quoted argument, and nothing executes -- the verb is read from
# commands.quotes_removed, so quoted prose is the data it is.
test_allow_the_bug_report_naming_both_tokens if {
	denials := guard.deny with input as bash_event(concat("", [
		"bd create --description 'denied: rm -rf /tmp/lab in one statement and ",
		hooks_dir, " in another'",
	]))
	count(denials) == 0
}

# STATEMENT BOUNDARIES, one case per separator: the same two tokens that deny in
# one statement must allow across two.
test_allow_removal_and_hooks_path_across_and_and if {
	denials := guard.deny with input as bash_event(concat("", ["rm -rf /tmp/x && ls ", hooks_dir]))
	count(denials) == 0
}

test_allow_removal_and_hooks_path_across_semicolon if {
	denials := guard.deny with input as bash_event(concat("", ["rm -rf /tmp/x; ls -l ", hooks_dir]))
	count(denials) == 0
}

test_allow_removal_and_hooks_path_across_or_or if {
	denials := guard.deny with input as bash_event(concat("", ["rm -rf /tmp/x || ls -l ", hooks_dir]))
	count(denials) == 0
}

# ...and the control that keeps the three above honest: put them in ONE
# statement and it denies. Without this pair the allow-cases could pass because
# the rule stopped working altogether.
test_deny_removal_naming_hooks_path_in_one_statement if {
	denials := guard.deny with input as bash_event(concat("", ["rm -rf /tmp/x ", hooks_dir]))
	"BUILTIN-GIT-BLOCK-NO-VERIFY" in rule_ids(denials)
}

test_allow_installing_a_hook_with_cp if {
	denials := guard.deny with input as bash_event(concat("", ["cp scripts/hooks/pre-push ", hooks_dir, "/pre-push"]))
	count(denials) == 0
}

# The find arm is scoped to an actual removal. Auditing the hook directory is
# not one, and denying it would be the same class of false positive again.
test_allow_find_exec_ls_over_the_hooks_directory if {
	denials := guard.deny with input as bash_event(concat("", ["find ", hooks_dir, " -type f -exec ls -l {} +"]))
	count(denials) == 0
}

# The word `rm` as an argument, not a program.
test_allow_reading_a_hook_and_grepping_for_the_word if {
	denials := guard.deny with input as bash_event(concat("", ["cat ", hooks_dir, "/pre-commit | grep -c rm"]))
	count(denials) == 0
}

# The shape that denied an inspection one-liner in the sibling guard: a python
# brace puts a quoted token in command position unless quoted spans are deleted.
test_allow_python_one_liner_inspecting_a_hook if {
	denials := guard.deny with input as bash_event(concat("", [
		`python3 -c "s={'rm'}; print(open('`, hooks_dir, `/pre-commit').read())"`,
	]))
	count(denials) == 0
}

test_allow_commit_message_naming_both_tokens if {
	denials := guard.deny with input as bash_event(concat("", [
		`git commit -m "stop rm -rf /tmp/lab from reading as a `, hooks_dir, ` removal"`,
	]))
	count(denials) == 0
}
