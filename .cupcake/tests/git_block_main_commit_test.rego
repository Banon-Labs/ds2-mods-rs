# OPA unit tests for git_block_main_commit.
# Run with:
#   opa test .cupcake/system/commands.rego \
#     .cupcake/policies/claude/git_block_main_commit.rego \
#     .cupcake/tests/git_block_main_commit_test.rego
package cupcake.policies.claude.git_block_main_commit_test

import rego.v1

import data.cupcake.policies.claude.git_block_main_commit as guard

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

test_deny_git_commit_on_main_string_signal if {
	denials := guard.deny with input as bash_event("git commit -m 'bad'", "main\n")
	"DS2-MODS-BLOCK-MAIN-COMMIT" in rule_ids(denials)
}

test_deny_git_commit_on_main_object_signal if {
	denials := guard.deny with input as bash_event_object_signal("git commit --allow-empty -m bad", "main\n")
	"DS2-MODS-BLOCK-MAIN-COMMIT" in rule_ids(denials)
}

test_deny_git_c_commit_on_main if {
	denials := guard.deny with input as bash_event("git -C \"$repo\" commit -m bad", "main\n")
	"DS2-MODS-BLOCK-MAIN-COMMIT" in rule_ids(denials)
}

test_deny_git_add_then_commit_on_main if {
	denials := guard.deny with input as bash_event("git add . && git commit -m bad", "main")
	"DS2-MODS-BLOCK-MAIN-COMMIT" in rule_ids(denials)
}

test_deny_git_dash_c_commit_on_main if {
	denials := guard.deny with input as bash_event("git -C /tmp/repo commit -m bad", "main")
	"DS2-MODS-BLOCK-MAIN-COMMIT" in rule_ids(denials)
}

test_allow_git_branch_reset_with_commit_prose_on_main if {
	cmd := concat("\n", [
		"repo=/home/banon/projects/fromsoftware-rs",
		"archive_branch=archive/local-inputblocker-injected-key-20260617",
		"old_head=$(git -C \"$repo\" rev-parse HEAD)",
		"# Preserve the old commit under a named local branch before resetting main.",
		"git -C \"$repo\" branch \"$archive_branch\" \"$old_head\"",
		"git -C \"$repo\" reset --hard origin/main",
		"printf 'archive_commit=%s\\n' \"$old_head\"",
	])
	denials := guard.deny with input as bash_event(cmd, "main")
	count(denials) == 0
}

test_deny_git_commit_when_branch_signal_missing if {
	denials := guard.deny with input as bash_event_no_branch_signal("git commit -m 'bad'")
	"DS2-MODS-BLOCK-MAIN-COMMIT" in rule_ids(denials)
}

test_deny_git_c_commit_when_branch_signal_missing if {
	denials := guard.deny with input as bash_event_no_branch_signal("git -C /tmp/repo commit -m 'bad'")
	"DS2-MODS-BLOCK-MAIN-COMMIT" in rule_ids(denials)
}

test_deny_git_commit_when_branch_signal_empty if {
	denials := guard.deny with input as bash_event("git commit -m 'bad'", "\n")
	"DS2-MODS-BLOCK-MAIN-COMMIT" in rule_ids(denials)
}

test_allow_git_commit_on_feature_branch if {
	denials := guard.deny with input as bash_event("git commit -m 'ok'", "feature/title-fadein-gfx-flash\n")
	count(denials) == 0
}

test_allow_non_commit_git_on_main if {
	denials := guard.deny with input as bash_event("git status --short && git log --oneline -3", "main\n")
	count(denials) == 0
}

test_allow_commit_word_without_git_on_main if {
	denials := guard.deny with input as bash_event("echo commit", "main\n")
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
	"worktree /home/banon/projects/er-mods-rs/.worktrees/detached-probe",
	"HEAD 2222222222222222222222222222222222222222",
	"detached",
	"",
])

test_allow_git_c_commit_nonmain_worktree_from_main_session if {
	denials := guard.deny with input as bash_event_with_worktrees(
		"git -C /home/banon/projects/er-mods-rs/.worktrees/portrait-stats-crate commit -m ok",
		"main\n", worktree_fixture,
	)
	count(denials) == 0
}

test_allow_git_c_commit_quoted_worktree_path_from_main_session if {
	denials := guard.deny with input as bash_event_with_worktrees(
		"git -C \"/home/banon/projects/er-mods-rs/.worktrees/portrait-stats-crate\" commit -F - <<'EOF'\nsummary line\n\nbody text\nEOF",
		"main\n", worktree_fixture,
	)
	count(denials) == 0
}

test_deny_git_c_commit_main_worktree_from_main_session if {
	denials := guard.deny with input as bash_event_with_worktrees(
		"git -C /home/banon/projects/er-mods-rs commit -m bad",
		"main\n", worktree_fixture,
	)
	"DS2-MODS-BLOCK-MAIN-COMMIT" in rule_ids(denials)
}

test_deny_git_c_commit_unregistered_path_from_main_session if {
	denials := guard.deny with input as bash_event_with_worktrees(
		"git -C /tmp/not-a-worktree commit -m bad",
		"main\n", worktree_fixture,
	)
	"DS2-MODS-BLOCK-MAIN-COMMIT" in rule_ids(denials)
}

test_deny_git_c_commit_detached_worktree_from_main_session if {
	denials := guard.deny with input as bash_event_with_worktrees(
		"git -C /home/banon/projects/er-mods-rs/.worktrees/detached-probe commit -m bad",
		"main\n", worktree_fixture,
	)
	"DS2-MODS-BLOCK-MAIN-COMMIT" in rule_ids(denials)
}

test_deny_git_c_commit_chained_with_bare_commit_from_main_session if {
	denials := guard.deny with input as bash_event_with_worktrees(
		"git -C /home/banon/projects/er-mods-rs/.worktrees/portrait-stats-crate commit -m ok && git commit -m sneak",
		"main\n", worktree_fixture,
	)
	"DS2-MODS-BLOCK-MAIN-COMMIT" in rule_ids(denials)
}

test_deny_git_c_commit_variable_path_from_main_session if {
	denials := guard.deny with input as bash_event_with_worktrees(
		"git -C \"$repo\" commit -m bad",
		"main\n", worktree_fixture,
	)
	"DS2-MODS-BLOCK-MAIN-COMMIT" in rule_ids(denials)
}

test_deny_git_c_commit_without_worktree_signal_from_main_session if {
	denials := guard.deny with input as bash_event(
		"git -C /home/banon/projects/er-mods-rs/.worktrees/portrait-stats-crate commit -m bad",
		"main\n",
	)
	"DS2-MODS-BLOCK-MAIN-COMMIT" in rule_ids(denials)
}

test_allow_git_archive_reset_script_with_commit_comments_on_main if {
	cmd := `set -euo pipefail
repo=/home/banon/projects/fromsoftware-rs
archive_branch=archive/local-inputblocker-injected-key-20260617
old_head=$(git -C "$repo" rev-parse HEAD)
# Preserve the old commit under a named local branch before resetting main.
if git -C "$repo" show-ref --verify --quiet "refs/heads/$archive_branch"; then
  existing=$(git -C "$repo" rev-parse "$archive_branch")
  if [ "$existing" != "$old_head" ]; then
    echo "archive branch exists at different commit: $archive_branch $existing" >&2
    exit 1
  fi
else
  git -C "$repo" branch "$archive_branch" "$old_head"
fi
git -C "$repo" reset --hard origin/main
printf 'archive_branch=%s\n' "$archive_branch"
printf 'archive_commit=%s\n' "$old_head"
printf '\nstatus\n'
git -C "$repo" status --short --branch
printf '\nremotes\n'
git -C "$repo" remote -v
printf '\nrecent refs\n'
git -C "$repo" log --oneline --decorate --max-count=6 --graph --all --simplify-by-decoration`
	denials := guard.deny with input as bash_event(cmd, "main\n")
	count(denials) == 0
}

test_allow_git_operations_with_commit_variable_names_on_main if {
	cmd := "commit_hash=$(git rev-parse HEAD); git reset --hard origin/main; printf 'archive_commit=%s\\n' \"$commit_hash\""
	denials := guard.deny with input as bash_event(cmd, "main\n")
	count(denials) == 0
}

# --- Shell-wrapper payloads (2026-08-26, bd er-effects-rs-dt2e) ---------------
#
# This guard shared DS2-MODS-BLOCK-MAIN-PUSH's anchor construction and so
# shared its hole: every case below produced ZERO denials before the
# executed-text decomposition, because the character before the verb is a quote.

test_deny_single_quoted_bash_c_commit_on_main if {
	denials := guard.deny with input as bash_event("bash -c 'git commit -m bad'", "main\n")
	"DS2-MODS-BLOCK-MAIN-COMMIT" in rule_ids(denials)
}

test_deny_double_quoted_sh_c_commit_on_main if {
	denials := guard.deny with input as bash_event(`sh -c "git commit -m bad"`, "main\n")
	"DS2-MODS-BLOCK-MAIN-COMMIT" in rule_ids(denials)
}

test_deny_login_shell_flag_commit_on_main if {
	denials := guard.deny with input as bash_event("bash -lc 'git commit -m bad'", "main\n")
	"DS2-MODS-BLOCK-MAIN-COMMIT" in rule_ids(denials)
}

test_deny_zsh_c_commit_when_branch_signal_missing if {
	denials := guard.deny with input as bash_event_no_branch_signal(`zsh -c "git commit -m bad"`)
	"DS2-MODS-BLOCK-MAIN-COMMIT" in rule_ids(denials)
}

test_deny_eval_commit_on_main if {
	denials := guard.deny with input as bash_event(`eval "git commit -m bad"`, "main\n")
	"DS2-MODS-BLOCK-MAIN-COMMIT" in rule_ids(denials)
}

test_deny_nested_wrapper_commit_on_main if {
	denials := guard.deny with input as bash_event(`bash -c 'bash -c "git commit -m bad"'`, "main\n")
	"DS2-MODS-BLOCK-MAIN-COMMIT" in rule_ids(denials)
}

test_allow_wrapped_commit_on_feature_branch if {
	denials := guard.deny with input as bash_event("bash -c 'git commit -m ok'", "feature/x\n")
	count(denials) == 0
}

# The worktree exception may not vouch for a command that also hides a bare
# commit in a wrapper.
test_deny_worktree_commit_chained_with_wrapped_commit if {
	denials := guard.deny with input as bash_event_with_worktrees(
		"git -C /home/banon/projects/er-mods-rs/.worktrees/portrait-stats-crate commit -m ok && bash -c 'git commit -m bad'",
		"main\n", worktree_fixture,
	)
	"DS2-MODS-BLOCK-MAIN-COMMIT" in rule_ids(denials)
}

# ... and it still works THROUGH a wrapper.
test_allow_wrapped_worktree_commit_from_main_session if {
	denials := guard.deny with input as bash_event_with_worktrees(
		"bash -c 'git -C /home/banon/projects/er-mods-rs/.worktrees/portrait-stats-crate commit -m ok'",
		"main\n", worktree_fixture,
	)
	count(denials) == 0
}

# --- Unreadable payloads fail closed ------------------------------------------

test_deny_unquoted_wrapper_payload_naming_git_and_commit_on_main if {
	denials := guard.deny with input as bash_event("bash -c $GIT_COMMIT_CMD", "main\n")
	"DS2-MODS-BLOCK-MAIN-COMMIT" in rule_ids(denials)
}

test_allow_unquoted_wrapper_payload_naming_git_and_commit_on_feature if {
	denials := guard.deny with input as bash_event("bash -c $GIT_COMMIT_CMD", "feature/x\n")
	count(denials) == 0
}

test_allow_unquoted_wrapper_payload_unrelated_to_git_on_main if {
	denials := guard.deny with input as bash_event("bash -c $BUILD_CMD", "main\n")
	count(denials) == 0
}

# --- Quoted TEXT is not an executed payload -----------------------------------

test_allow_quoted_prose_naming_a_commit_on_main if {
	denials := guard.deny with input as bash_event(`echo "then run git commit -m x"`, "main\n")
	count(denials) == 0
}

test_allow_bd_memory_body_quoting_a_commit_on_main if {
	denials := guard.deny with input as bash_event(
		"$HOME/.local/bin/bd remember --key k \"before\ngit commit -m x\nafter\"",
		"main\n",
	)
	count(denials) == 0
}

test_allow_heredoc_documenting_a_commit_on_main if {
	denials := guard.deny with input as bash_event(
		"cat > docs/guards.md <<'EOF'\ngit commit -m x\nEOF",
		"main\n",
	)
	count(denials) == 0
}

# A heredoc a SHELL reads is a program, not data.
test_deny_shell_read_heredoc_commit_on_main if {
	denials := guard.deny with input as bash_event(
		"bash <<'EOF'\ngit commit -m bad\nEOF",
		"main\n",
	)
	"DS2-MODS-BLOCK-MAIN-COMMIT" in rule_ids(denials)
}
