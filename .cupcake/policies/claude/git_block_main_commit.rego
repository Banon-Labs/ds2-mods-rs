# METADATA
# scope: package
# title: Block Local Commits on Main
# authors: ["ds2-mods-rs agents"]
# custom:
#   severity: CRITICAL
#   id: DS2-MODS-BLOCK-MAIN-COMMIT
#   routing:
#     required_events: ["PreToolUse"]
#     required_tools: ["Bash"]
#     required_signals: ["current_branch", "worktree_branches", "commit_target_branches"]
package cupcake.policies.claude.git_block_main_commit

import rego.v1

import data.cupcake.system.commands

# Never allow local commits while the active branch is main. Agents must create a
# feature/tooling branch from the intended base first, then commit there. If the
# branch signal is missing, fail closed: a missing signal caused a live main
# commit to slip through this guard on 2026-07-13.
#
# Worktree-target exception (2026-07-30, bd
# guard-blocks-worktree-commits-from-main-session-cwd-2026-07-29): the
# current_branch signal reads the branch of the session checkout, not the tree a
# `git -C <path> commit` actually targets, so worktree-based feature commits were
# denied whenever the main checkout sat on main. A command whose every commit
# invocation explicitly targets a registered git worktree on a non-main branch is
# not a main commit and is allowed. Anything unparsed, unregistered, detached, or
# bare (`git commit` without -C) keeps the deny (fail closed).
#
# Shell-wrapper decomposition (2026-08-26, bd er-effects-rs-dt2e): this guard
# carried the same anchor construction as DS2-MODS-BLOCK-MAIN-PUSH and the same
# double defect. `bash -c 'git commit -m x'` from a main session produced ZERO
# denials, because the character before the verb is a quote and a quote is not in
# `(^|[;&|(\n])`; meanwhile a newline IS, so quoted prose that named the command
# on its own line was denied though nothing would run. The pattern is unchanged;
# it now runs over commands.executed_texts, which neutralises the anchors inside
# quoted operand spans and hands back every shell-wrapper payload as its own
# text. See the header of .cupcake/system/commands.rego.
#
# Another repository's checkout (2026-09-26, bd ds2-mods-rs-qzd): the worktree
# exception only knew THIS repository's `git worktree list`, so with the session
# checkout on main a commit in another repository's feature-branch worktree was
# denied -- `git -C /home/banon/projects/fromsoftware-rs-ds2-paramdefs commit`
# and `cd <that path> && git commit` both, though the target was on
# ds2-paramdefs. The commit_target_branches signal now resolves the branch of each
# absolute `-C`/`cd` operand IN THAT DIRECTORY, and a commit whose target it names
# is judged by that branch. The rule is the same everywhere: a target on `main`
# is denied whichever repository it belongs to, because "never commit on main" is
# about the branch, not about this repo. Two forms are recognized:
#
#   git -C <absolute path> commit ...        (any number, each target resolved)
#   cd <absolute path> && git commit ...     (the text STARTS with that cd, is
#                                             followed by `&&`, `;` or a newline,
#                                             and changes directory nowhere else)
#
# Anything else -- a relative path, a variable, a second cd/pushd/popd, a bare
# commit without a leading cd -- keeps the deny.
deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"
	any_executed_commit
	blocked_branch_context
	not commits_target_only_nonmain_worktrees

	decision := {
		"rule_id": "DS2-MODS-BLOCK-MAIN-COMMIT",
		"reason": "Do not commit unless the guard can confirm the active branch is not main. Create/switch to a feature or tooling branch based on the intended base (or target a registered non-main worktree explicitly: git -C <worktree-path> commit), and ensure the current_branch signal is available.",
		"severity": "CRITICAL",
	}
}

# Fail closed on a wrapper whose payload cannot be read while the command names
# git and commit: `bash -c $CMD` hands a shell a program the guard cannot see,
# and a blind spot must not read as an allow.
deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"
	opaque_commit_payload
	blocked_branch_context

	decision := {
		"rule_id": "DS2-MODS-BLOCK-MAIN-COMMIT",
		"reason": "This command wraps a shell payload the guard cannot read (an unquoted or substituted `-c`/`eval` argument) while naming git and commit, and the active branch cannot be confirmed as non-main. Run the git command directly, or put the payload in a quoted argument.",
		"severity": "CRITICAL",
	}
}

executed_texts := commands.executed_texts(input.tool_input.command)

any_executed_commit if {
	some text in executed_texts
	is_git_commit(lower(text))
}

opaque_commit_payload if {
	commands.unparsed_shell_payload(input.tool_input.command)
	lowered := lower(input.tool_input.command)
	contains(lowered, "git")
	contains(lowered, "commit")
}

blocked_branch_context if {
	current_branch == "main"
}

blocked_branch_context if {
	current_branch == ""
}

# Match a real git commit invocation instead of any command text containing both
# words. This avoids false positives from shell comments, printf labels, and
# variable names such as archive_commit while still blocking direct git commit
# calls, including common global-option forms such as `git -C <repo> commit`.
git_commit_command_pattern := `(^|[;&|(\n])\s*(command\s+)?(?:[^\s;&|()"']*/)?git([ \t]+((-c|--git-dir|--work-tree|--namespace|--config-env)(=|[ \t]+)("[^"\n]*"|'[^'\n]*'|[^ \t;&|()\n]+)|--(bare|no-pager|paginate|literal-pathspecs|no-replace-objects|exec-path)(=("[^"\n]*"|'[^'\n]*'|[^ \t;&|()\n]+))?))*[ \t]+commit([ \t;&|)\n]|$)`

is_git_commit(cmd) if {
	regex.match(git_commit_command_pattern, cmd)
}

current_branch := branch if {
	branch := trim(input.signals.current_branch, " \t\r\n")
} else := branch if {
	branch := trim(input.signals.current_branch.output, " \t\r\n")
} else := "" if {
	true
}

# --- Worktree-target exception helpers ---------------------------------------

# Judged PER EXECUTED TEXT and PER SIMPLE COMMAND. Each text (the command itself
# and every shell-wrapper payload) is cut at `;`, `&&`, `||`, `&`, `|`, `(`, `)`
# and newlines into an ordered list of simple commands, and every commit among
# them has to be vouched for on its own. The previous find-all tally compared two
# match counts over the whole text, and a match that consumed the separator in
# front of the next commit (`git -C <wt> commit;git commit`) hid that second
# commit from both counts, so the counts agreed and the bare commit rode along.
# Anchoring each pattern at the start of its own segment cannot consume a
# neighbour. executed_texts has already blanked separators inside quoted spans, so
# the cut does not split a quoted path or message (the contract shell_segments in
# commands.rego documents).
simple_commands(text) := [s |
	replaced := replace(replace(replace(replace(replace(replace(replace(text, "||", "\n"), "&&", "\n"), ";", "\n"), "&", "\n"), "|", "\n"), "(", "\n"), ")", "\n")
	some raw in split(replaced, "\n")
	s := trim_space(raw)
	s != ""
]

# Any git commit, whatever its global options (the segment-anchored twin of
# git_commit_command_pattern).
seg_commit_pattern := `(?i)^(command[ \t]+)?(?:[^\s;&|()"']*/)?git([ \t]+((-c|--git-dir|--work-tree|--namespace|--config-env)(=|[ \t]+)("[^"\n]*"|'[^'\n]*'|[^ \t;&|()\n]+)|--(bare|no-pager|paginate|literal-pathspecs|no-replace-objects|exec-path)(=("[^"\n]*"|'[^'\n]*'|[^ \t;&|()\n]+))?))*[ \t]+commit([ \t]|$)`

# The strict single-target form: `git -C <path> commit`. Group 1 is the path
# token (optionally quoted). Any other global-option arrangement fails it.
seg_c_commit_pattern := `(?i)^(?:command[ \t]+)?(?:[^\s;&|()"']*/)?git[ \t]+-C[ \t]+("[^"\n]*"|'[^'\n]*'|[^ \t;&|()\n]+)[ \t]+commit(?:[ \t]|$)`

# A commit with no global options at all, which runs wherever the shell is.
seg_bare_commit_pattern := `(?i)^(?:command[ \t]+)?(?:[^\s;&|()"']*/)?git[ \t]+commit(?:[ \t]|$)`

# Anything that moves the shell.
seg_dir_change_pattern := `(?i)^(?:(?:builtin|command)[ \t]+)?(?:cd|pushd|popd)(?:[ \t]|$)`

# The text OPENS with `cd <path>` joined to what follows by `&&`, `;` or a
# newline. Not `||` (the rest runs when the cd FAILED), not `|` or `&` (the cd
# runs in a subshell and moves nothing).
leading_cd_pattern := `^[ \t\n]*cd[ \t]+("[^"\n]*"|'[^'\n]*'|[^ \t;&|()\n]+)[ \t]*(?:&&|;|\n)`

commit_commands(text) := [s |
	some s in simple_commands(text)
	regex.match(seg_commit_pattern, s)
]

commits_target_only_nonmain_worktrees if {
	count([s | some text in executed_texts; some s in commit_commands(text)]) > 0
	every text in executed_texts {
		text_commits_ok(text)
	}
}

text_commits_ok(text) if {
	count(commit_commands(text)) == 0
	not is_git_commit(lower(text))
}

text_commits_ok(text) if {
	commits := commit_commands(text)
	count(commits) > 0
	every s in commits {
		commit_ok(text, s)
	}
}

commit_ok(_, s) if {
	m := regex.find_all_string_submatch_n(seg_c_commit_pattern, s, 1)
	count(m) == 1
	worktree_target_ok(m[0][1])
}

commit_ok(text, s) if {
	regex.match(seg_bare_commit_pattern, s)
	cd_scoped_to_nonmain(text)
}

cd_scoped_to_nonmain(text) if {
	m := regex.find_all_string_submatch_n(leading_cd_pattern, text, 1)
	count(m) == 1
	worktree_target_ok(m[0][1])
	count([s | some s in simple_commands(text); regex.match(seg_dir_change_pattern, s)]) == 1
}

worktree_target_ok(token) if {
	path := trim_right(trim(token, "\"'"), "/")
	branch := target_branch(path)
	branch != "main"
	branch != ""
}

# The branch resolved in the target directory itself (commit_target_branches),
# else the session repository's own worktree list.
target_branch(path) := branch if {
	branch := resolved_target_branch(path)
} else := branch if {
	branch := worktree_branch(path)
}

resolved_target_branch(path) := branch if {
	some line in split(commit_target_signal, "\n")
	parts := split(line, "\t")
	count(parts) == 2
	parts[0] == path
	branch := trim_space(parts[1])
}

commit_target_signal := out if {
	out := input.signals.commit_target_branches
	is_string(out)
} else := out if {
	out := input.signals.commit_target_branches.output
	is_string(out)
} else := "" if {
	true
}

# Resolve a worktree path to its branch from `git worktree list --porcelain`
# output. Detached worktrees have no `branch ` line and resolve to nothing
# (fail closed).
worktree_branch(path) := branch if {
	lines := split(worktree_branches_signal, "\n")
	some i, j
	lines[i] == concat("", ["worktree ", path])
	j > i
	startswith(lines[j], "branch refs/heads/")
	not worktree_entry_between(lines, i, j)
	branch := trim_space(trim_prefix(lines[j], "branch refs/heads/"))
}

worktree_entry_between(lines, i, j) if {
	some k
	k > i
	k < j
	startswith(lines[k], "worktree ")
}

worktree_branches_signal := out if {
	out := input.signals.worktree_branches
	is_string(out)
} else := out if {
	out := input.signals.worktree_branches.output
} else := "" if {
	true
}
