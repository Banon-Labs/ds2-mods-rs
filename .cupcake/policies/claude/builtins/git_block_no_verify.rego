# METADATA
# scope: package
# title: Git Block No-Verify - Builtin Policy
# authors: ["Cupcake Builtins"]
# custom:
#   severity: HIGH
#   id: BUILTIN-GIT-BLOCK-NO-VERIFY
#   routing:
#     required_events: ["PreToolUse"]
#     required_tools: ["Bash"]
package cupcake.policies.builtins.git_block_no_verify

import rego.v1

import data.cupcake.system.commands

# Shell-wrapper decomposition (2026-08-26, bd er-effects-rs-dt2e). This policy
# recognises its verbs through commands.has_verb, whose `(^|\s)verb(\s|$)`
# boundary a quote does not satisfy -- so a hook bypass wrapped as
# `bash -c 'git commit --no-verify -m x'` produced ZERO denials, measured.
#
# The scan set below is STRICTLY ADDITIVE on purpose: it is the executed-text
# decomposition (which makes wrapper payloads visible) UNION the raw command
# exactly as this policy saw it before. Nothing this builtin used to deny can
# stop being denied. That is deliberately narrower than the repo-owned git
# guards, which also had their quoted-prose false positives fixed: this file is
# a vendored Cupcake builtin, and its own false-positive surface -- an unanchored
# `contains(cmd, "--no-verify")` that fires on a doc, commit message or heredoc
# merely NAMING the flag -- is a separate defect, filed rather than changed here.
no_verify_scan_texts := commands.executed_texts(input.tool_input.command) | {input.tool_input.command}

# Block git commands that bypass verification hooks
deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"

	# Every text this command actually executes, plus the raw command itself
	some text in no_verify_scan_texts
	command := lower(text)

	# Check if it's a git command with --no-verify flag
	contains_git_no_verify(command)

	decision := {
		"rule_id": "BUILTIN-GIT-BLOCK-NO-VERIFY",
		"reason": "Git operations with --no-verify are not permitted. Commit hooks must run for code quality and security checks.",
		"severity": "HIGH",
	}
}

# Check if command contains git with --no-verify flag
# Uses helper library to prevent spacing bypass (TOB-EQTY-LAB-CUPCAKE-3)
contains_git_no_verify(cmd) if {
	# Check for git commit with --no-verify
	commands.has_verb(cmd, "git")
	commands.has_verb(cmd, "commit")
	contains(cmd, "--no-verify")
}

contains_git_no_verify(cmd) if {
	# Check for git commit with -n (shorthand for --no-verify), but only
	# inside the actual git-commit command segment. A later command like
	# `| sed -n ...` must not make `git commit -h | sed -n` look like a
	# no-verify commit.
	regex.match(git_commit_short_no_verify_pattern, cmd)
}

git_commit_short_no_verify_pattern := `(?m)(^|[;&|]\s*|\n)\s*git(\s+(?:-C|-c)\s+\S+|\s+--git-dir=\S+|\s+--work-tree=\S+|\s+--namespace=\S+)*\s+commit(\s+[^;&|\n]*)?\s+-[a-z]*n[a-z]*(\s|$|[;&|])`

contains_git_no_verify(cmd) if {
	# Check for git push with --no-verify
	commands.has_verb(cmd, "git")
	commands.has_verb(cmd, "push")
	contains(cmd, "--no-verify")
}

contains_git_no_verify(cmd) if {
	# Check for git merge with --no-verify
	commands.has_verb(cmd, "git")
	commands.has_verb(cmd, "merge")
	contains(cmd, "--no-verify")
}

# Also block attempts to disable hooks via config
deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"

	some text in no_verify_scan_texts
	command := lower(text)

	# Check if trying to disable hooks via git config
	contains_hook_disable(command)

	decision := {
		"rule_id": "BUILTIN-GIT-BLOCK-NO-VERIFY",
		"reason": "Disabling git hooks is not permitted. Hooks are required for code quality and security.",
		"severity": "HIGH",
	}
}

# ---------------------------------------------------------------------------
# core.hooksPath (2026-08-31, bd committed-state-compile-gate-2026-08-31)
#
# This rule used to AND `contains(cmd, "core.hookspath")` with
# `contains(cmd, "/dev/null")` over the WHOLE command string, so the two only had
# to be CO-PRESENT, not related. That denied
#
#     git config core.hooksPath scripts/hooks && git config --get core.hooksPath >/dev/null
#
# -- a command that INSTALLS hooks and merely silences an unrelated read -- with
# "Disabling git hooks is not permitted". Measured live, and it fired on an agent
# REPAIRING core.hooksPath after the er-effects-rs -> er-mods-rs rename left it
# pointing at an absolute path that no longer existed, so git had silently run NO
# hooks since 39a919e0. A guard that blocks the repair of its own total failure is
# worse than a guard that is merely noisy.
#
# The fix is not to drop the `/dev/null` half -- `git config core.hooksPath
# /dev/null` is a real way to disable hooks and must stay denied. It is to require
# the two to be THE SAME ASSIGNMENT: only whitespace, an `=`, quotes, or a
# line-continuation backslash may stand between the key and the value. A `>`, a
# `2>`, a `|` or a `;` in that gap means they belong to different commands, which
# is exactly what the false positive was.
#
# WHY THOSE GAP CHARACTERS AND NO OTHERS, against the shape the engine actually
# delivers (bd cupcake-tests-vs-delivered-input-shape-and-wrapper-payload-hole):
#   * space/tab -- `git config core.hooksPath /dev/null`, and the engine's
#     whitespace_normalization has already collapsed any run of them to one;
#   * `=` -- the `git -c core.hooksPath=/dev/null <cmd>` form, which the old rule
#     ALLOWED outright because it carries no `config` verb (measured);
#   * quotes -- `core.hooksPath "/dev/null"`; commands.scan_text blanks a quoted
#     span's ANCHORS but leaves the quote characters themselves in place;
#   * backslash + newline -- a continued command line, `git config core.hooksPath \`
#     newline `/dev/null`, which the shim leaves as a newline (it is a join, not a
#     boundary) and the engine then collapses to `\ `.
#
# The `git` requirement is kept, loosened to admit `/usr/bin/git`: it is what stops
# prose that merely NAMES the setting from being read as an attempt to apply it.
# The trailing-slash form is written the same way as commands.shell_name_pattern so
# a word merely ENDING in git (`legit`) cannot satisfy it.
hooks_path_dev_null_pattern := `core\.hookspath[ \t\r\n\\'"]*=?[ \t\r\n\\'"]*/dev/null`

git_invocation_pattern := `(^|[ \t;&|(){}])([^ \t;&|(){}'"]*/)?git([ \t]|$)`

contains_hook_disable(cmd) if {
	regex.match(git_invocation_pattern, cmd)

	# Lowercased: every caller passes `lower(text)`, so the shipped
	# `core.hooksPath` spelling could never match and this rule was dead.
	regex.match(hooks_path_dev_null_pattern, cmd)
}

# The GIT_CONFIG_* environment form is the documented way to set a config value
# without `git config` and without `-c`, and nothing here reasoned about it before:
#
#     GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=core.hooksPath GIT_CONFIG_VALUE_0=/dev/null git commit
#
# Its key and value live in SEPARATE assignments, so no same-assignment pattern can
# reach it; it needs its own clause. The captured index must MATCH on both sides, so
# a command that legitimately sets core.hooksPath at index 0 and some unrelated key
# to /dev/null at index 1 is not swept up. No `git` requirement here: the variable
# names already say git, and requiring the verb would miss `/usr/bin/git`.
git_config_env_key_pattern := `git_config_key_([0-9]+)[ \t]*=[ \t]*['"]*core\.hookspath`

git_config_env_value_pattern := `git_config_value_([0-9]+)[ \t]*=[ \t]*['"]*/dev/null`

contains_hook_disable(cmd) if {
	some key_match in regex.find_all_string_submatch_n(git_config_env_key_pattern, cmd, -1)
	some value_match in regex.find_all_string_submatch_n(git_config_env_value_pattern, cmd, -1)
	key_match[1] == value_match[1]
}

# ---------------------------------------------------------------------------
# HOOK REMOVAL: THE VERB AND THE HOOK DIRECTORY MUST BE THE SAME COMMAND
# (2026-08-31, bd er-effects-rs-c0t9)
#
# Three arms used to live in contains_hook_disable, each ANDing a verb test with
# `contains(cmd, ".git/hooks")` over the WHOLE command string. Co-presence, not
# relation -- the identical defect the core.hooksPath block above was narrowed to
# fix, in the identical file, one screen further down. So this
#
#     rm -rf <an unrelated dir> && mkdir -p <hooks dir> && cp shim <hooks dir>/pre-push
#
# was denied with "Disabling git hooks is not permitted" while INSTALLING a hook,
# because a removal verb aimed at one path in one statement met a hooks-directory
# operand in a different statement. Measured live on 2026-08-31 -- and then again
# on the bug report about it, which was denied for quoting both tokens. A guard
# that blocks its own bug report has stopped being evidence about commands.
#
# THE SHAPE, taken from guard_layer_destructive_guard.rego rather than reinvented
# (its segmenter now lives in commands.rego so there is exactly one):
#
#   * PER STATEMENT, so tokens in different statements cannot combine. Statement
#     and not segment: a PIPE carries data, so `echo <hooks dir>/pre-commit |
#     xargs rm -f` really does delete a hook with the path in one stage and the
#     verb in the next. The old whole-string test caught that by accident and
#     segment granularity would have dropped it -- a false-positive fix that
#     opens a hole is a worse bug than the one it fixes. The cost is a narrow
#     over-denial (`rm -rf /tmp/x | tee <hooks dir>/log`), which is affordable
#     only because these verb sets are tiny; see the note in commands.rego for
#     why the destructive guard takes the opposite trade;
#   * over commands.executed_texts, so a `fish -c '<payload>'` wrapper is still
#     decomposed and its payload split into statements on its own;
#   * VERB from commands.quotes_removed(statement) in COMMAND POSITION, so a `rm`
#     inside a quoted argument -- a commit message, a bd description, a python
#     one-liner -- is the data it is, not a program. This is what un-denies the
#     bug report;
#   * PATH from the RAW statement, quotes and all, because quoting a path is
#     ordinary: `rm -rf "$repo/.git/hooks"` must still be caught.
#
# WHAT WAS DELIBERATELY NOT WIDENED. The verb set stays exactly {rm, unlink,
# trash} + mv + chmod: `rmdir`/`shred` were never caught here and adding them is
# a different change with a different risk. The one addition is the find arm
# below, and it exists to PREVENT a regression rather than to extend reach.
hooks_dir := ".git/hooks"

hook_scan_statements := {statement |
	some text in commands.executed_texts(input.tool_input.command)
	some statement in commands.shell_statements(lower(text))
}

deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"

	some statement in hook_scan_statements
	hook_removal_statement(statement)

	decision := {
		"rule_id": "BUILTIN-GIT-BLOCK-NO-VERIFY",
		"reason": "Disabling git hooks is not permitted. Hooks are required for code quality and security.",
		"severity": "HIGH",
	}
}

# Removing hook files.
hook_removal_statement(statement) if {
	contains(statement, hooks_dir)
	some verb in {"rm", "unlink", "trash"}
	commands.has_command_verb(commands.quotes_removed(statement), verb)
}

# Moving/renaming the directory to disable it.
hook_removal_statement(statement) if {
	contains(statement, hooks_dir)
	commands.has_command_verb(commands.quotes_removed(statement), "mv")
}

# chmod to non-executable.
hook_removal_statement(statement) if {
	contains(statement, hooks_dir)
	commands.has_command_verb(commands.quotes_removed(statement), "chmod")
	regex.match(`-x|-[0-9]*0[0-9]*`, statement) # Removing execute permission
}

# `find <hooks dir> -exec rm {}` puts the removal verb after an option, where
# COMMAND POSITION cannot see it -- the old whole-string has_verb caught that
# shape by accident, and tightening to command position would have dropped it.
# So it is caught deliberately here, together with `-delete`, which nothing
# caught before.
#
# Scoped to an actual removal rather than to `-exec` in general: the destructive
# guard's broader `-(delete|exec|execdir)` arm would also deny
# `find .git/hooks -type f -exec ls -l {} +`, and denying an audit of the hook
# directory is the same class of false positive this block exists to remove.
find_removal_pattern := `(^|[ \t])(-delete([ \t]|$)|-(exec|execdir|ok)[ \t]+(rm|unlink|trash|shred)([ \t]|$))`

hook_removal_statement(statement) if {
	contains(statement, hooks_dir)
	unquoted := commands.quotes_removed(statement)
	commands.has_command_verb(unquoted, "find")
	regex.match(find_removal_pattern, unquoted)
}
