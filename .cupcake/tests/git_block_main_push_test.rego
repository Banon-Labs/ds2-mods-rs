# OPA unit tests for git_block_main_push.
# Run with:
#   opa test .cupcake/system/commands.rego \
#     .cupcake/policies/claude/git_block_main_push.rego \
#     .cupcake/tests/git_block_main_push_test.rego
package cupcake.policies.claude.git_block_main_push_test

import rego.v1

import data.cupcake.policies.claude.git_block_main_push as guard

bash_event(cmd, branch) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Bash",
	"tool_input": {"command": cmd, "timeout": 30000},
	"signals": {"current_branch": branch},
}

bash_event_object_signal(cmd, branch) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Bash",
	"tool_input": {"command": cmd, "timeout": 30000},
	"signals": {"current_branch": {"output": branch, "exit_code": 0}},
}

bash_event_no_branch_signal(cmd) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Bash",
	"tool_input": {"command": cmd, "timeout": 30000},
	"signals": {},
}

rule_ids(denials) := {d.rule_id | some d in denials}

test_deny_bare_git_push_on_main if {
	denials := guard.deny with input as bash_event("git push", "main\n")
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_deny_git_push_when_branch_signal_missing if {
	denials := guard.deny with input as bash_event_no_branch_signal("git push")
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_deny_git_push_when_branch_signal_empty if {
	denials := guard.deny with input as bash_event("git push", "\n")
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_deny_git_push_main_from_feature_branch if {
	denials := guard.deny with input as bash_event("git push origin main", "feature/no-main-push")
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_deny_git_c_push_main_from_feature_branch if {
	denials := guard.deny with input as bash_event("git -C /tmp/repo push origin main", "feature/no-main-push")
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_deny_git_push_head_to_main_from_feature_branch if {
	denials := guard.deny with input as bash_event("git push origin HEAD:main", "feature/no-main-push")
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_deny_git_push_ref_heads_main_from_feature_branch if {
	denials := guard.deny with input as bash_event("git push origin feature/no-main-push:refs/heads/main", "feature/no-main-push")
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_deny_object_branch_signal_on_main if {
	denials := guard.deny with input as bash_event_object_signal("git push", "main\n")
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_allow_git_push_feature_branch if {
	denials := guard.deny with input as bash_event("git push -u origin guard/no-direct-main-push", "guard/no-direct-main-push\n")
	count(denials) == 0
}

test_allow_git_push_head_to_feature_branch if {
	denials := guard.deny with input as bash_event("git push origin HEAD:refs/heads/guard/no-direct-main-push", "guard/no-direct-main-push\n")
	count(denials) == 0
}

# current_branch can describe the parent checkout rather than this session's
# worktree. The explicit non-main upstream destination must therefore override
# that stale main signal, including when followed by harmless status inspection.
test_allow_explicit_nonmain_upstream_when_branch_signal_stale_main if {
	denials := guard.deny with input as bash_event(
		"git push -u origin fix/title-accept-in-update && git status --short --branch",
		"main\n",
	)
	count(denials) == 0
}

test_allow_explicit_nonmain_upstream_when_branch_signal_missing if {
	denials := guard.deny with input as bash_event_no_branch_signal(
		"git push --set-upstream origin feature/no-direct-main-push",
	)
	count(denials) == 0
}

test_deny_explicit_main_upstream_when_branch_signal_stale_main if {
	denials := guard.deny with input as bash_event("git push -u origin main", "main\n")
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_deny_explicit_head_to_main_upstream_when_branch_signal_stale_main if {
	denials := guard.deny with input as bash_event("git push -u origin HEAD:main", "main\n")
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_deny_explicit_nonmain_upstream_chained_with_bare_push_when_branch_signal_stale_main if {
	denials := guard.deny with input as bash_event(
		"git push -u origin feature/no-direct-main-push && git push",
		"main\n",
	)
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_allow_non_push_git_on_main if {
	denials := guard.deny with input as bash_event("git status --short && git log --oneline -3", "main\n")
	count(denials) == 0
}

test_allow_push_word_without_git_on_main if {
	denials := guard.deny with input as bash_event("echo push main", "main\n")
	count(denials) == 0
}

# --- Worktree-target exception ------------------------------------------------

bash_event_with_worktrees(cmd, branch, worktrees) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Bash",
	"tool_input": {"command": cmd, "timeout": 30000},
	"signals": {"current_branch": branch, "worktree_branches": worktrees},
}

worktree_fixture := concat("\n", [
	"worktree /home/banon/projects/er-mods-rs",
	"HEAD 0000000000000000000000000000000000000000",
	"branch refs/heads/main",
	"",
	"worktree /home/banon/projects/er-mods-rs/.worktrees/portrait-stats-crate",
	"HEAD 1111111111111111111111111111111111111111",
	"branch refs/heads/feature/portrait-stats-crate",
	"",
])

test_allow_git_c_push_feature_from_nonmain_worktree_main_session if {
	denials := guard.deny with input as bash_event_with_worktrees(
		"git -C /home/banon/projects/er-mods-rs/.worktrees/portrait-stats-crate push -u origin feature/portrait-stats-crate",
		"main\n", worktree_fixture,
	)
	count(denials) == 0
}

test_deny_git_c_push_main_refspec_from_nonmain_worktree if {
	denials := guard.deny with input as bash_event_with_worktrees(
		"git -C /home/banon/projects/er-mods-rs/.worktrees/portrait-stats-crate push origin HEAD:main",
		"main\n", worktree_fixture,
	)
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_deny_git_c_push_unregistered_path_from_main_session if {
	denials := guard.deny with input as bash_event_with_worktrees(
		"git -C /tmp/not-a-worktree push -u origin feature/foo",
		"main\n", worktree_fixture,
	)
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_deny_git_c_push_main_worktree_from_main_session if {
	denials := guard.deny with input as bash_event_with_worktrees(
		"git -C /home/banon/projects/er-mods-rs push -u origin feature/foo",
		"main\n", worktree_fixture,
	)
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_deny_git_c_push_chained_with_bare_push_from_main_session if {
	denials := guard.deny with input as bash_event_with_worktrees(
		"git -C /home/banon/projects/er-mods-rs/.worktrees/portrait-stats-crate push -u origin feature/portrait-stats-crate && git push",
		"main\n", worktree_fixture,
	)
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

# --- Explicit <src>:<dst> refspec exception (2026-08-25) ----------------------

# The verbatim command that motivated the exception: renaming an already-pushed
# remote feature branch from a session whose primary checkout sits on main.
test_allow_refspec_rename_when_branch_signal_stale_main if {
	denials := guard.deny with input as bash_event(
		"git push origin origin/refactor/drop-dead-gates:refs/heads/split/drop-dead-gates",
		"main\n",
	)
	count(denials) == 0
}

test_allow_refspec_rename_when_branch_signal_missing if {
	denials := guard.deny with input as bash_event_no_branch_signal(
		"git push origin origin/refactor/drop-dead-gates:refs/heads/split/drop-dead-gates",
	)
	count(denials) == 0
}

test_allow_refspec_rename_followed_by_status_inspection if {
	denials := guard.deny with input as bash_event(
		"git push origin origin/refactor/drop-dead-gates:refs/heads/split/drop-dead-gates && git status --short --branch",
		"main\n",
	)
	count(denials) == 0
}

test_allow_two_refspec_renames_in_one_command if {
	denials := guard.deny with input as bash_event(
		"git push origin origin/a:refs/heads/split/a && git push origin origin/b:refs/heads/split/b",
		"main\n",
	)
	count(denials) == 0
}

# Pushing main's own commits ONTO a non-main branch cannot update remote main.
test_allow_refspec_from_main_source_to_nonmain_destination if {
	denials := guard.deny with input as bash_event(
		"git push origin refs/heads/main:refs/heads/backup/main-snapshot-2026-08-25",
		"main\n",
	)
	count(denials) == 0
}

# A `+`-prefixed (force) refspec still cannot update remote main, so THIS guard
# permits it. Force pushes are the other guard's business:
# DS2-MODS-REQUIRE-FRESH-ORIGIN-MAIN matches `push ... +<refspec>` and demands a
# verified-fresh origin/main. Pinned here so the split of responsibility is
# deliberate rather than accidental.
test_allow_force_refspec_to_nonmain_destination if {
	denials := guard.deny with input as bash_event(
		"git push origin +origin/refactor/drop-dead-gates:refs/heads/split/drop-dead-gates",
		"main\n",
	)
	count(denials) == 0
}

# --- ... and every spelling of a main DESTINATION stays denied ----------------

test_deny_refspec_destination_bare_main if {
	denials := guard.deny with input as bash_event(
		"git push origin origin/refactor/drop-dead-gates:main",
		"main\n",
	)
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_deny_refspec_destination_refs_heads_main if {
	denials := guard.deny with input as bash_event(
		"git push origin origin/refactor/drop-dead-gates:refs/heads/main",
		"main\n",
	)
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

# `heads/main` resolves to refs/heads/main on the remote (verified against real
# repositories, see the policy comment) and push_targets_main now names it.
test_deny_refspec_destination_heads_main if {
	denials := guard.deny with input as bash_event(
		"git push origin origin/refactor/drop-dead-gates:heads/main",
		"main\n",
	)
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_deny_refspec_destination_heads_main_from_feature_branch if {
	denials := guard.deny with input as bash_event(
		"git push origin HEAD:heads/main",
		"feature/no-main-push\n",
	)
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

# Conservative by design: a genuinely distinct branch whose last path component
# is `main` is refused rather than adjudicated.
test_deny_refspec_destination_nested_main_component if {
	denials := guard.deny with input as bash_event(
		"git push origin origin/refactor/drop-dead-gates:refs/heads/split/main",
		"main\n",
	)
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

# --- Mixed and unparsed forms fail closed -------------------------------------

test_deny_refspec_rename_chained_with_main_push if {
	denials := guard.deny with input as bash_event(
		"git push origin a:refs/heads/b && git push origin main",
		"main\n",
	)
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

# ... and the same mixture is denied from a feature branch too, where only
# push_targets_main can catch it.
test_deny_refspec_rename_chained_with_main_push_from_feature_branch if {
	denials := guard.deny with input as bash_event(
		"git push origin a:refs/heads/b && git push origin main",
		"feature/no-main-push\n",
	)
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_deny_refspec_rename_chained_with_bare_push if {
	denials := guard.deny with input as bash_event(
		"git push origin a:refs/heads/b && git push",
		"main\n",
	)
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_deny_refspec_rename_chained_with_explicit_upstream_push if {
	denials := guard.deny with input as bash_event(
		"git push origin a:refs/heads/b && git push -u origin feature/x",
		"main\n",
	)
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

# Deletion pushes are deliberately out of scope: both spellings fail closed.
test_deny_deletion_refspec_empty_source if {
	denials := guard.deny with input as bash_event("git push origin :refs/heads/foo", "main\n")
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_deny_delete_option_push if {
	denials := guard.deny with input as bash_event("git push origin --delete foo", "main\n")
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

# An option between `push` and the refspec is unparsed, so it fails closed.
test_deny_force_option_before_refspec if {
	denials := guard.deny with input as bash_event(
		"git push --force origin origin/a:refs/heads/split/a",
		"main\n",
	)
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_deny_refspec_with_trailing_third_operand if {
	denials := guard.deny with input as bash_event(
		"git push origin origin/a:refs/heads/split/a origin/b:refs/heads/split/b",
		"main\n",
	)
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

# A source with no colon is not a refspec; it must not fall through to this
# exception (the `-u` exception owns that shape, and only with `-u`).
test_deny_two_operand_push_without_colon if {
	denials := guard.deny with input as bash_event("git push origin feature/x", "main\n")
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

# --- Shell-wrapper payloads (2026-08-26, bd er-effects-rs-dt2e) ---------------
#
# Every case below produced ZERO denials before the executed-text decomposition
# landed: the character in front of the verb is a quote, and a quote was not in
# the anchor class. AGENTS.md tells agents to wrap commands as `bash -c "<cmd>"`
# for fish, so this was the bypass form the repo's own guidance recommends.

test_deny_single_quoted_bash_c_push_main if {
	denials := guard.deny with input as bash_event("bash -c 'git push origin main'", "feature/no-main-push\n")
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_deny_double_quoted_sh_c_push_main if {
	denials := guard.deny with input as bash_event(`sh -c "git push origin main"`, "feature/no-main-push\n")
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_deny_login_shell_flag_push_head_to_main if {
	denials := guard.deny with input as bash_event("bash -lc 'git push origin HEAD:main'", "feature/no-main-push\n")
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_deny_zsh_c_push_main if {
	denials := guard.deny with input as bash_event(`zsh -c "git push origin main"`, "feature/no-main-push\n")
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_deny_ksh_c_push_main if {
	denials := guard.deny with input as bash_event("ksh -c 'git push origin main'", "feature/no-main-push\n")
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_deny_dash_c_push_main if {
	denials := guard.deny with input as bash_event("dash -c 'git push origin main'", "feature/no-main-push\n")
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_deny_absolute_shell_path_with_options_push_main if {
	denials := guard.deny with input as bash_event("/bin/bash --norc -c 'git push origin main'", "feature/no-main-push\n")
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_deny_env_prefixed_shell_push_main if {
	denials := guard.deny with input as bash_event("env FOO=1 bash -c 'git push origin main'", "feature/no-main-push\n")
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_deny_eval_push_main if {
	denials := guard.deny with input as bash_event(`eval "git push origin main"`, "feature/no-main-push\n")
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_deny_xargs_wrapped_shell_push_main if {
	denials := guard.deny with input as bash_event("echo x | xargs -I{} bash -c 'git push origin main'", "feature/no-main-push\n")
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

# Two levels: the deepest literal quoting reaches without escapes.
test_deny_nested_wrapper_push_main if {
	denials := guard.deny with input as bash_event(`bash -c 'bash -c "git push origin main"'`, "feature/no-main-push\n")
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

# A BARE push inside a wrapper is the same fail-closed case as a bare push
# outside one: the refspec comes from git config, so the branch signal decides.
test_deny_wrapped_bare_push_on_main if {
	denials := guard.deny with input as bash_event("bash -c 'git push'", "main\n")
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_allow_wrapped_bare_push_on_feature_branch if {
	denials := guard.deny with input as bash_event("bash -c 'git push'", "feature/no-main-push\n")
	count(denials) == 0
}

# An exception may not vouch for a command that also hides a push in a wrapper:
# the count-match is taken across every executed text at once.
test_deny_explicit_upstream_push_chained_with_wrapped_bare_push if {
	denials := guard.deny with input as bash_event(
		"git push -u origin feature/x && bash -c 'git push'",
		"main\n",
	)
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

# ... and the exceptions still work THROUGH a wrapper, which is what makes the
# decomposition symmetric rather than merely stricter.
test_allow_wrapped_refspec_rename_when_branch_signal_stale_main if {
	denials := guard.deny with input as bash_event(
		"bash -c 'git push origin origin/a:refs/heads/split/a'",
		"main\n",
	)
	count(denials) == 0
}

test_deny_wrapped_refspec_to_main_destination if {
	denials := guard.deny with input as bash_event(
		"bash -c 'git push origin origin/a:refs/heads/main'",
		"main\n",
	)
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_allow_wrapped_explicit_upstream_when_branch_signal_stale_main if {
	denials := guard.deny with input as bash_event(
		`sh -c "git push -u origin feature/no-direct-main-push"`,
		"main\n",
	)
	count(denials) == 0
}

# --- Unreadable payloads fail closed ------------------------------------------

test_deny_unquoted_wrapper_payload_naming_git_and_push if {
	denials := guard.deny with input as bash_event("bash -c $GIT_PUSH_CMD", "feature/no-main-push\n")
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_deny_eval_with_unquoted_payload_alongside_a_push if {
	denials := guard.deny with input as bash_event("eval $x && git push origin foo", "feature/no-main-push\n")
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

# An opaque wrapper that names neither git nor push is out of this guard's
# jurisdiction; answering it with a push denial would be noise, not safety.
test_allow_unquoted_wrapper_payload_unrelated_to_git if {
	denials := guard.deny with input as bash_event("bash -c $BUILD_CMD", "feature/no-main-push\n")
	count(denials) == 0
}

# --- Quoted TEXT is not an executed payload -----------------------------------
#
# The mirror-image defect, and the reason a wider anchor class was not the fix:
# `\n` IS in the class, so a memory body, a commit message or a doc that merely
# QUOTES the guarded command on its own line was denied while nothing would run.
# A guard whose own documentation cannot be written in the repo that enforces it
# is unwritable, so these are requirements, not niceties.

test_allow_bd_memory_body_quoting_the_command if {
	denials := guard.deny with input as bash_event(
		"$HOME/.local/bin/bd remember --key k \"before\ngit push origin main\nafter\"",
		"feature/no-main-push\n",
	)
	count(denials) == 0
}

test_allow_commit_message_naming_the_rule if {
	denials := guard.deny with input as bash_event(
		`git commit -m "guard: block git push origin main via wrappers"`,
		"feature/no-main-push\n",
	)
	count(denials) == 0
}

test_allow_commit_message_with_the_command_on_its_own_line if {
	denials := guard.deny with input as bash_event(
		"git commit -m \"guard: close the wrapper bypass\n\ngit push origin main was invisible\n\"",
		"feature/no-main-push\n",
	)
	count(denials) == 0
}

test_allow_heredoc_documenting_the_command if {
	denials := guard.deny with input as bash_event(
		"cat > docs/guards.md <<'EOF'\ngit push origin main\nEOF",
		"feature/no-main-push\n",
	)
	count(denials) == 0
}

test_allow_echo_of_the_command if {
	denials := guard.deny with input as bash_event(`echo "git push origin main"`, "feature/no-main-push\n")
	count(denials) == 0
}

# `python3 -c` takes a Python program, not shell: its argument is a string
# literal in another language, and treating it as shell would deny prose again.
test_allow_python_dash_c_string_literal if {
	denials := guard.deny with input as bash_event(
		`python3 -c 'print("git push origin main")'`,
		"feature/no-main-push\n",
	)
	count(denials) == 0
}

# --- Quoted operands still parse ----------------------------------------------
#
# Neutralising a quoted span means blanking its command-position characters, NOT
# deleting it: a scrub would leave `git -C  push` and take the worktree
# exception's path operand with it.
test_allow_git_c_push_with_quoted_worktree_path if {
	denials := guard.deny with input as bash_event_with_worktrees(
		`git -C "/home/banon/projects/er-mods-rs/.worktrees/portrait-stats-crate" push -u origin feature/portrait-stats-crate`,
		"main\n", worktree_fixture,
	)
	count(denials) == 0
}

# --- Fail-closed fallbacks ----------------------------------------------------
#
# Command substitution runs even inside double quotes, so a text containing `$(`
# keeps its raw form and the pre-existing anchors keep matching.
test_deny_command_substitution_running_a_push_on_main if {
	denials := guard.deny with input as bash_event(`echo "$(git push origin main)"`, "feature/no-main-push\n")
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

test_deny_unbalanced_quotes_around_a_push if {
	denials := guard.deny with input as bash_event(`echo 'unclosed ; git push origin main`, "feature/no-main-push\n")
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}

# A heredoc read by a SHELL is a program, not data, so its body keeps its
# command positions.
test_deny_shell_read_heredoc_push_main if {
	denials := guard.deny with input as bash_event(
		"bash <<'EOF'\ngit push origin main\nEOF",
		"feature/no-main-push\n",
	)
	"DS2-MODS-BLOCK-MAIN-PUSH" in rule_ids(denials)
}
