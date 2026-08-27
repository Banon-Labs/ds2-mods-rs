# METADATA
# scope: package
# title: Block Direct Pushes to Main
# authors: ["ds2-mods-rs agents"]
# custom:
#   severity: CRITICAL
#   id: DS2-MODS-BLOCK-MAIN-PUSH
#   routing:
#     required_events: ["PreToolUse"]
#     required_tools: ["Bash"]
#     required_signals: ["current_branch", "worktree_branches"]
package cupcake.policies.claude.git_block_main_push

import rego.v1

import data.cupcake.system.commands

# Agents must not push directly to main. Work lands on feature/tooling branches;
# main is updated only through the user's preferred review/merge path. If the
# branch signal is missing, fail closed for push commands because bare `git push`
# inherits the current branch/refspec from Git config.
#
# Worktree-target exception (2026-07-30, bd
# guard-blocks-worktree-commits-from-main-session-cwd-2026-07-29): the
# current_branch signal reads the session checkout's branch, so a push issued as
# `git -C <worktree> push` from a session whose primary checkout sits on main was
# denied even when the worktree is on a feature branch. Such a push is allowed
# when every push invocation targets a registered non-main worktree.
#
# Explicit-refspec exception (2026-08-15): the same stale signal can report main
# when this session itself is rooted in a non-main worktree. A command that names
# a non-main upstream destination (`git push -u origin feature/name`) is safe to
# permit: it cannot update remote main. Bare pushes and unparsed options remain
# fail-closed. Explicit main refspecs (push_targets_main) stay denied
# unconditionally. (Its name predates the exception below and is misleading: it
# parses `-u <remote> <branch>`, which carries no `<src>:<dst>` refspec at all.)
#
# Source:destination refspec exception (2026-08-25): renaming or republishing an
# already-pushed remote branch has no sanctioned form. From a session whose
# primary checkout sits on main, this was denied:
#
#     git push origin origin/refactor/drop-dead-gates:refs/heads/split/drop-dead-gates
#
# Neither side of that refspec is main, so the command cannot update remote main,
# yet it matched neither existing exception: `pushes_target_only_nonmain_worktrees`
# wants the `git -C <path> push` form and `pushes_target_only_explicit_nonmain_branches`
# wants `-u|--set-upstream <remote> <branch>`. Both parsers stop at the first
# `<src>:<dst>` token. `pushes_target_only_explicit_nonmain_refspecs` below permits
# a command only when EVERY push in it is `git push <remote> <src>:<dst>` with both
# sides non-empty and a destination that cannot be main. push_targets_main is a
# separate blocked_push_context rule and still denies `main`, `HEAD:main`,
# `heads/main` and `feature:refs/heads/main` regardless of any exception.
#
# Shell-wrapper decomposition (2026-08-26, bd er-effects-rs-dt2e): every pattern
# below used to run against the raw command string, which asks about lexical
# position rather than about what runs. `bash -c 'git push origin main'` produced
# ZERO denials -- the character before the verb is a quote, and a quote is not in
# the anchor class -- while a `bd remember` body or a doc that merely QUOTED the
# command on its own line WAS denied, because a newline is. The patterns are
# unchanged; what changed is the text they see. They now run over
# commands.executed_texts, which returns the command with quoted operand spans
# anchor-neutralised plus every shell-wrapper payload as its own text. See the
# header of .cupcake/system/commands.rego for the decomposition and its limits.
#
# Both the deny trigger and the three exceptions read that same set, so a wrapper
# cannot hide a push AND cannot borrow another push's exception: the count-match
# the exceptions require is taken across all executed texts at once, so one
# unrecognised push anywhere -- inside a wrapper included -- still denies.
deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"
	any_executed_push
	blocked_push_context

	decision := {
		"rule_id": "DS2-MODS-BLOCK-MAIN-PUSH",
		"reason": "Do not push directly to main. Push a feature/tooling branch and let main update through the review/merge path.",
		"severity": "CRITICAL",
	}
}

# Fail closed on a wrapper this guard cannot read. `bash -c $CMD` and
# `eval $CMD` hand a shell a program that is not in the command text, so no
# pattern can rule out a push to main. Gated on the command naming both `git`
# and `push` so an unrelated opaque wrapper is not answered with a push denial;
# a genuinely opaque payload that names neither is out of any text policy's
# reach and is documented as such rather than pretended away.
deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"
	opaque_push_payload

	decision := {
		"rule_id": "DS2-MODS-BLOCK-MAIN-PUSH",
		"reason": "This command wraps a shell payload the guard cannot read (an unquoted or substituted `-c`/`eval` argument) while naming git and push, so it cannot be shown not to push to main. Run the git command directly, or put the payload in a quoted argument.",
		"severity": "CRITICAL",
	}
}

executed_texts := commands.executed_texts(input.tool_input.command)

any_executed_push if {
	some text in executed_texts
	is_git_push(lower(text))
}

# Plain substrings, not word tokens: the payload is unreadable precisely because
# it is a variable, and `bash -c $GIT_PUSH_CMD` carries no standalone `git` or
# `push` word for a token pattern to find.
opaque_push_payload if {
	commands.unparsed_shell_payload(input.tool_input.command)
	lowered := lower(input.tool_input.command)
	contains(lowered, "git")
	contains(lowered, "push")
}

blocked_push_context if {
	current_branch == "main"
	not pushes_target_only_nonmain_worktrees
	not pushes_target_only_explicit_nonmain_branches
	not pushes_target_only_explicit_nonmain_refspecs
}

blocked_push_context if {
	current_branch == ""
	not pushes_target_only_nonmain_worktrees
	not pushes_target_only_explicit_nonmain_branches
	not pushes_target_only_explicit_nonmain_refspecs
}

blocked_push_context if {
	some text in executed_texts
	push_targets_main(lower(text))
}

# Match a real git push invocation, including common global-option forms such
# as `git -C <repo> push`.
git_push_command_pattern := `(^|[;&|(
])\s*(command\s+)?git([ \t]+((-c|--git-dir|--work-tree|--namespace|--config-env)(=|[ \t]+)("[^"\n]*"|'[^'\n]*'|[^ \t;&|()\n]+)|--(bare|no-pager|paginate|literal-pathspecs|no-replace-objects|exec-path)(=("[^"\n]*"|'[^'\n]*'|[^ \t;&|()\n]+))?))*[ \t]+push([ \t;&|)\n]|$)`

is_git_push(cmd) if {
	regex.match(git_push_command_pattern, cmd)
}

# Main-target forms we block even from a feature branch:
#   git push origin main
#   git push origin HEAD:main
#   git push origin HEAD:heads/main
#   git push origin feature:refs/heads/main
#   git -C repo push origin refs/heads/main
#
# `heads/main` was added on 2026-08-25. A destination that does not start with
# `refs/` is resolved against the remote's existing refs, and `refs/<dst>` is one
# of those resolution rules -- so `heads/main` reaches `refs/heads/main`. Verified
# against real repositories, not inferred: `git push origin HEAD:heads/main`
# fast-forwarded remote `refs/heads/main` and printed `HEAD -> main`, while
# `HEAD:refs/main` was refused by git (a `refs/`-prefixed destination is literal)
# and `HEAD:Main` created a distinct branch (refs are case-sensitive).
push_targets_main(cmd) if {
	regex.match(git_push_main_target_pattern, cmd)
}

# `(` joined the anchor class on 2026-08-26. is_git_push has always had it, so a
# subshell or command substitution running a push (`echo "$(git push origin
# main)"`) was recognised as A push but never as a MAIN push, and slipped through
# from any feature branch. It was left out before because widening this class on
# the raw command string widened the quoted-prose false positive with it; now
# that quoted spans are anchor-neutralised before matching, it does not.
git_push_main_target_pattern := `(?m)(^|[;&|(]\s*|\n)\s*(command\s+)?git(\s+((-c|--git-dir|--work-tree|--namespace|--config-env)(=|\s+)("[^"\n]*"|'[^'\n]*'|[^\s;&|()]+)|--(bare|no-pager|paginate|literal-pathspecs|no-replace-objects|exec-path)(=("[^"\n]*"|'[^'\n]*'|[^\s;&|()]+))?))*\s+push(\s+[^;&|\n]*)?(\s|:)((refs/)?heads/)?main(\s|$|[;&|)\n])`

current_branch := branch if {
	branch := trim(input.signals.current_branch, " \t\r\n")
} else := branch if {
	branch := trim(input.signals.current_branch.output, " \t\r\n")
} else := "" if {
	true
}

# --- Worktree-target exception helpers ---------------------------------------

# Case-insensitive find-all twin of git_push_command_pattern, applied to the
# RAW (unlowered) command so worktree path capitalization survives extraction.
git_push_findall_pattern := `(?i)(^|[;&|(\n])\s*(command\s+)?git([ \t]+((-c|--git-dir|--work-tree|--namespace|--config-env)(=|[ \t]+)("[^"\n]*"|'[^'\n]*'|[^ \t;&|()\n]+)|--(bare|no-pager|paginate|literal-pathspecs|no-replace-objects|exec-path)(=("[^"\n]*"|'[^'\n]*'|[^ \t;&|()\n]+))?))*[ \t]+push([ \t;&|)\n]|$)`

# Strict single-target form the exception recognizes: `git -C <path> push`.
# Group 2 captures the path token (optionally quoted).
git_c_push_extract_pattern := `(?i)(^|[;&|(\n])\s*(?:command\s+)?git[ \t]+-C[ \t]+("[^"\n]*"|'[^'\n]*'|[^ \t;&|()\n]+)[ \t]+push(?:[ \t;&|)\n]|$)`

# Every push invocation ACROSS ALL EXECUTED TEXTS, so a push smuggled into a
# `bash -c` payload counts towards the general tally and no exception can vouch
# for the command while leaving it unread.
extracted(pattern) := [m |
	some text in executed_texts
	matches := regex.find_all_string_submatch_n(pattern, text, -1)
	some m in matches
]

general_pushes := extracted(git_push_findall_pattern)

# Every push invocation in the command must be the strict `git -C <path> push`
# form and every extracted path must be a registered worktree on a non-main
# branch. Explicit main refspecs are still denied by push_targets_main.
pushes_target_only_nonmain_worktrees if {
	strict := extracted(git_c_push_extract_pattern)
	count(general_pushes) > 0
	count(strict) == count(general_pushes)
	every m in strict {
		worktree_target_ok(m[2])
	}
}

worktree_target_ok(token) if {
	path := trim_right(trim(token, "\"'"), "/")
	branch := worktree_branch(path)
	branch != "main"
	branch != ""
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

# --- Explicit non-main destination exception ---------------------------------

# Deliberately narrow: this only recognizes the conventional explicit-upstream
# form used to publish a feature branch. Other push option arrangements fail
# closed until they have their own parser and regression coverage.
git_push_explicit_branch_extract_pattern := `(?i)(^|[;&|(\n])\s*(?:command\s+)?git\s+push\s+(?:-u|--set-upstream)\s+("[^"\n]*"|'[^'\n]*'|[^\s;&|()\n]+)\s+("[^"\n]*"|'[^'\n]*'|[^\s;&|()\n]+)(?:\s|$|[;&|)\n])`

# A stale/missing branch signal must not block a command when every push in it
# has the explicit, non-main destination form above. The independent
# push_targets_main rule still denies `main`, `HEAD:main`, and
# `feature:refs/heads/main` unconditionally.
pushes_target_only_explicit_nonmain_branches if {
	explicit := extracted(git_push_explicit_branch_extract_pattern)
	count(general_pushes) > 0
	count(explicit) == count(general_pushes)
	every m in explicit {
		explicit_nonmain_destination(m[3])
	}
}

explicit_nonmain_destination(token) if {
	destination := trim(token, "\"'")
	destination != ""
	not startswith(destination, "-")
}

# --- Explicit <src>:<dst> refspec exception ----------------------------------

# Strict single-target form: `git push <remote> <src>:<dst>`, nothing else on the
# invocation. Group 2 captures the remote token, group 3 the refspec token.
#
# The remote token's first character excludes `-`, so any option between `push`
# and the refspec (`--force`, `-f`, `--delete`, `-u`) fails to match here and the
# whole command falls back to denied. That is the intent: an option changes what
# the refspec MEANS, and none of them have a parser or regression coverage yet.
#
# DELETION PUSHES ARE DELIBERATELY OUT OF SCOPE, both spellings. `git push origin
# --delete foo` never matches (the option sits in the remote slot), and
# `git push origin :refs/heads/foo` is rejected below because its source side is
# empty. Deleting a non-main branch is safe, but the two forms need their own
# parser -- one that reads `--delete`'s operands as destinations rather than as a
# refspec -- and a wrong parser here deletes a branch nobody asked to delete. They
# stay fail-closed until someone writes that parser with its own tests.
#
# The trailing group anchors the refspec as the LAST token of the invocation, so
# a second operand (`git push origin a:b c:d`) does not match and the command
# falls back to denied. Without that anchor the parser would vouch for a refspec
# list it never looked at.
git_push_refspec_extract_pattern := `(?i)(^|[;&|(\n])\s*(?:command\s+)?git[ \t]+push[ \t]+("[^"\n]*"|'[^'\n]*'|[^-\s;&|()\n][^\s;&|()\n]*)[ \t]+("[^"\n]*"|'[^'\n]*'|[^\s;&|()\n]+)[ \t]*(?:[;&|)\n]|$)`

# Every push invocation in the command must be the strict form above and every
# extracted refspec must name a destination that cannot be main. One unrecognized
# push in the command (a bare `git push`, an option form, a second remote) makes
# the strict count differ from the general count and the exception does not apply.
# push_targets_main is evaluated independently and still denies explicit main
# destinations even when this rule holds.
pushes_target_only_explicit_nonmain_refspecs if {
	refspecs := extracted(git_push_refspec_extract_pattern)
	count(general_pushes) > 0
	count(refspecs) == count(general_pushes)
	every m in refspecs {
		refspec_nonmain_destination(m[3])
	}
}

# A destination is treated as main whenever its last path component is `main`.
# That is wider than git's own resolution -- it also refuses a genuinely distinct
# branch such as `refs/heads/split/main` -- and it is meant to be: this exception
# exists to unblock branch renames, not to adjudicate which spellings of `main`
# reach the protected ref. Anything it refuses simply falls back to the deny.
refspec_main_destination_pattern := `(?i)(^|/)main$`

refspec_nonmain_destination(token) if {
	spec := trim(token, "\"'")
	parts := split(spec, ":")
	count(parts) == 2
	source := parts[0]
	destination := parts[1]

	# Both sides must be present: `src:` and `:dst` (the deletion form) are not
	# what this parser was written for.
	source != ""
	destination != ""
	not startswith(source, "-")
	not startswith(destination, "-")
	not regex.match(refspec_main_destination_pattern, destination)
}
