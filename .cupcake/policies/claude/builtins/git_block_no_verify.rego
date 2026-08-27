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

contains_hook_disable(cmd) if {
	commands.has_verb(cmd, "git")
	commands.has_verb(cmd, "config")

	# Lowercased: every caller passes `lower(text)`, so the shipped
	# `core.hooksPath` spelling could never match and this rule was dead.
	contains(cmd, "core.hookspath")
	contains(cmd, "/dev/null")
}

contains_hook_disable(cmd) if {
	# Detect attempts to chmod hooks to non-executable
	commands.has_verb(cmd, "chmod")
	regex.match(`\.git/hooks`, cmd)
	regex.match(`-x|-[0-9]*0[0-9]*`, cmd) # Removing execute permission
}

contains_hook_disable(cmd) if {
	# Detect attempts to remove hook files
	contains(cmd, ".git/hooks")
	removal_cmds := {"rm", "unlink", "trash"}
	commands.has_dangerous_verb(cmd, removal_cmds)
}

contains_hook_disable(cmd) if {
	# Detect moving/renaming hooks to disable them
	commands.has_verb(cmd, "mv")
	contains(cmd, ".git/hooks")
}
