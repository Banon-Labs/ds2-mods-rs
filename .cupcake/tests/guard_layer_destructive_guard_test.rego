# OPA unit tests for the guard-layer destructive-operation rule.
#
# Run with:
#   opa test .cupcake/system/commands.rego \
#            .cupcake/policies/claude/guard_layer_destructive_guard.rego \
#            .cupcake/tests/guard_layer_destructive_guard_test.rego
#
# (`opa test .cupcake/` picks it up with everything else.)
#
# EVERY DANGEROUS COMMAND HERE IS ASSEMBLED FROM TOKENS rather than written
# whole, and that is not decoration. The rule under test denies a Bash command
# that names `.cupcake` next to a destructive verb -- which is exactly the shape
# of a heredoc that writes this file. An agent editing it through
# `cat > ... <<'EOF'` would be blocked by the rule the file tests, because the
# engine collapses the heredoc's newlines onto the reader before any policy runs.
# Splitting the tokens is the difference between a file that can be maintained
# and one that can only be read. The same trick, for the same reason, is in
# scripts/test-cupcake-delivered-shape.py.
package cupcake.policies.claude.guard_layer_destructive_guard_test

import rego.v1

import data.cupcake.policies.claude.guard_layer_destructive_guard as guard

cup := concat("", [".", "cupcake"])

hooks := concat("", [".git/", "hooks"])

rm_rf := concat(" ", ["rm", "-rf"])

repo := "/home/banon/projects/er-mods-rs"

bash_event(command) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Bash",
	"tool_input": {"command": command, "timeout": 30000, "description": "test case"},
}

rule_ids(decisions) := {d.rule_id | some d in decisions}

denied(event) if {
	decisions := guard.halt with input as event
	"CLAUDE-GUARD-LAYER-DESTRUCTIVE" in rule_ids(decisions)
}

denied_bash(command) if {
	denied(bash_event(command))
}

# ---------------------------------------------------------------------------
# DENY: the guard layer is removed, relocated or reverted by a shell command
# ---------------------------------------------------------------------------

test_deny_bare_recursive_delete if {
	denied_bash(concat(" ", [rm_rf, cup]))
}

test_deny_delete_of_a_subdirectory if {
	denied_bash(concat(" ", [rm_rf, concat("", [cup, "/policies/claude"])]))
}

test_deny_delete_by_absolute_path if {
	denied_bash(concat(" ", [rm_rf, concat("/", [repo, cup])]))
}

# THE VERB reached by absolute/relative path (2026-08-31, bd er-effects-rs-5z75),
# distinct from the operand-side case above: `/bin/rm`, `/usr/bin/rm` and `./rm`
# were all measured ALLOWED before the fix, because a leading path component is
# none of command_position_prefix_pattern's anchor characters and the byte
# immediately before the verb is `/`.
test_deny_verb_invoked_by_bin_absolute_path if {
	denied_bash(concat(" ", [concat("", ["/bin/", "rm"]), "-rf", cup]))
}

test_deny_verb_invoked_by_usr_bin_absolute_path if {
	denied_bash(concat(" ", [concat("", ["/usr/bin/", "rm"]), "-rf", cup]))
}

test_deny_verb_invoked_by_dot_slash_relative_path if {
	denied_bash(concat(" ", [concat("", ["./", "rm"]), "-rf", cup]))
}

# The trailing-slash requirement on the path-prefix group is what keeps a word
# merely ENDING in the verb, reached through an unrelated path, from matching.
test_allow_path_prefixed_word_merely_ending_in_verb if {
	not denied_bash(concat(" ", ["scripts/confirm", cup]))
}

# The wrapper form AGENTS.md actively recommends for fish. `has_verb`'s own
# `(^|\s)` anchor cannot see past the opening quote, so this is only reachable
# through commands.executed_texts.
test_deny_inside_a_bash_wrapper_payload if {
	denied_bash(concat("", ["bash -c '", concat(" ", [rm_rf, cup]), "'"]))
}

test_deny_inside_a_fish_wrapper_payload if {
	denied_bash(concat("", ["fish -c \"", concat(" ", [rm_rf, cup]), "\""]))
}

test_deny_moved_aside if {
	denied_bash(concat(" ", ["mv", cup, concat("", [cup, ".off"])]))
}

# Every member of destructive_verbs has a case. `rmdir` earned one the hard way:
# a mutation control that DELETED it from the set left the whole suite green,
# which meant the set had a member nothing was asserting.
test_deny_rmdir_of_the_hooks_directory if {
	denied_bash(concat(" ", ["rmdir", hooks]))
}

test_deny_shred_of_the_rulebook if {
	denied_bash(concat(" ", ["shred", concat("", [cup, "/rulebook.yml"])]))
}

test_deny_truncate_of_a_policy if {
	denied_bash(concat(" ", [
		"truncate", "-s", "0",
		concat("", [cup, "/policies/claude/git_block_main_push.rego"]),
	]))
}

# Discarding uncommitted guard work is destruction in a different spelling, and
# AGENTS.md already forbids these verbs in prose. This is that made executable.
test_deny_git_checkout_revert if {
	denied_bash(concat(" ", ["git", "checkout", "--", cup]))
}

test_deny_git_restore_with_dash_c if {
	denied_bash(concat(" ", ["git", "-C", repo, "restore", concat("", [cup, "/policies"])]))
}

test_deny_git_clean if {
	denied_bash(concat(" ", ["git", "clean", "-fdx", cup]))
}

test_deny_find_delete if {
	denied_bash(concat(" ", ["find", cup, "-name", "'*.rego'", "-delete"]))
}

test_deny_symlink_swap if {
	denied_bash(concat(" ", ["ln", "-sfn", "/tmp/empty", cup]))
}

test_deny_behind_a_sudo_wrapper if {
	denied_bash(concat(" ", ["sudo", rm_rf, cup]))
}

# The separator makes this a second STATEMENT, which is what the hook shim
# manufactures out of a multi-line command.
test_deny_as_a_later_statement if {
	denied_bash(concat(" ", ["echo hi &&", rm_rf, cup]))
}

# Quoting a path is ordinary spelling, not evasion. These are the reason the path
# test reads the RAW segment while the verb test reads only the unquoted part.
test_deny_double_quoted_path_operand if {
	denied_bash(concat("", [rm_rf, " \"", cup, "\""]))
}

test_deny_single_quoted_path_operand if {
	denied_bash(concat("", [rm_rf, " '", cup, "'"]))
}

test_deny_quoted_path_behind_a_variable if {
	denied_bash(concat("", [rm_rf, " \"$repo/", cup, "\""]))
}

test_deny_git_hooks_directory_delete if {
	denied_bash(concat(" ", [rm_rf, hooks]))
}

test_deny_git_hooks_file_delete_by_absolute_path if {
	denied_bash(concat(" ", ["rm", "-f", concat("/", [repo, hooks, "pre-commit"])]))
}

# ---------------------------------------------------------------------------
# ALLOW: everything the maintenance of this layer actually requires
#
# These are the cases that make the rule usable rather than theatre. Enabling
# upstream's rulebook_security_guardrails builtin would have denied the first
# four outright -- which is why it is off. See the block at the bottom of
# .cupcake/rulebook.yml.
# ---------------------------------------------------------------------------

test_allow_running_the_policy_suite if {
	not denied_bash(concat(" ", ["opa", "test", concat("", [cup, "/"])]))
}

test_allow_committing_a_policy_change if {
	not denied_bash(concat(" ", [
		"git", "commit", "-F", "/tmp/msg.txt", "--",
		concat("", [cup, "/rulebook.yml"]),
	]))
}

test_allow_reading_a_policy_with_an_inline_script if {
	not denied_bash(concat("", [
		"python3 -c \"print(open('", cup, "/rulebook.yml').read())\"",
	]))
}

# THE REGRESSION THAT WAS CAUGHT IN VIVO (2026-08-31). An inline python script
# that names a destructive verb inside a set literal AND opens a file in the
# policy tree. `{` is a command-position anchor and survives quoted-span
# blanking, so the earlier draft of this rule read `{ rm }` as a running program
# and denied the command that was auditing it. A guard that blocks inspection of
# itself is the exact defect this policy replaced.
test_allow_inline_python_naming_a_verb_and_a_policy_path if {
	not denied_bash(concat("", [
		"python3 -c \"s={'", "rm", "'}; open('",
		cup, "/policies/claude/x.rego')\"",
	]))
}

# The same shape with a dict literal, which is how the verb ends up beside the
# path with only a colon between them.
test_allow_inline_python_dict_pairing_a_verb_with_a_policy_path if {
	not denied_bash(concat("", [
		"python3 -c \"d={'", "mv", "': '", cup, "/rulebook.yml'}\"",
	]))
}

test_allow_grepping_the_policy_tree if {
	not denied_bash(concat(" ", ["grep", "-rn", "halt", concat("", [cup, "/policies"])]))
}

# `git mv` is the supported rename: `git` is not one of the wrapper words
# has_command_verb steps over, so the `mv` behind it is not in command position.
test_allow_git_mv_rename if {
	not denied_bash(concat(" ", [
		"git", "mv",
		concat("", [cup, "/policies/claude/a.rego"]),
		concat("", [cup, "/policies/claude/b.rego"]),
	]))
}

# Segment isolation: a destructive verb in one statement must not borrow a path
# operand from another.
test_allow_unrelated_delete_beside_a_policy_read if {
	not denied_bash(concat(" ", [
		"rm", "-f", "/tmp/scratch.json", "&&",
		"opa", "test", concat("", [cup, "/"]),
	]))
}

# Quoted prose that merely NAMES the command executes nothing. This is the false
# positive that has bitten every guard in this repo at least once.
#
# THE LEADING PROSE IS LOad-BEARING, and it is here because a mutation control
# caught this test passing for the wrong reason. Written as
# `echo "rm -rf .cupcake"`, the verb sits immediately after the opening quote --
# so even commands.has_verb, which asks only whether the word appears after
# WHITESPACE, misses it. The test then stayed green under a mutant that replaced
# has_command_verb with has_verb, and was proving nothing about command position.
# With a word in front of the verb, the space is there, has_verb matches, and
# only the command-position requirement keeps this allowed.
test_allow_quoted_prose_naming_the_command if {
	not denied_bash(concat("", [
		"echo \"cleanup step: ",
		concat(" ", [rm_rf, cup]),
		"\"",
	]))
}

test_allow_bd_memory_body_naming_the_command if {
	not denied_bash(concat("", [
		"$HOME/.local/bin/bd remember --key k \"the guard denies ",
		concat(" ", [rm_rf, cup]),
		" now\"",
	]))
}

# A path that merely ENDS in the same letters is a different file.
test_allow_delete_of_a_similarly_named_file if {
	not denied_bash(concat(" ", ["rm", "-f", concat("", ["notes", cup])]))
}

test_allow_delete_of_the_hook_shim if {
	not denied_bash("rm -f scripts/cupcake-hook.sh")
}

test_allow_delete_of_a_stale_agent_worktree if {
	not denied_bash(concat(" ", [rm_rf, ".claude/worktrees/agent-a0a7d0da7a1ffa8ff"]))
}

# DOCUMENTED GAP, pinned so it stays a known fact rather than folklore: chmod is
# not in the verb set, because `chmod +x` on a new signal script is ordinary
# authoring and the disarming form cannot be told from it without parsing mode
# operands.
test_allow_chmod_on_a_signal_script_known_gap if {
	not denied_bash(concat(" ", ["chmod", "+x", concat("", [cup, "/signals/new.sh"])]))
}

# KNOWN-OPEN BYPASSES, pinned as ALLOW on purpose. This rule stops a blunt
# instrument, not a determined agent, and the two cheapest evasions are recorded
# here so nobody mistakes it for a lockdown: a glob that never spells the path,
# and a delete run from inside the directory. Closing either needs filesystem
# knowledge a policy engine does not have.
test_known_open_glob_spelling_is_not_caught if {
	not denied_bash(concat(" ", [rm_rf, ".cup*"]))
}

test_known_open_delete_from_inside_the_directory_is_not_caught if {
	not denied_bash(concat(" ", ["cd", cup, "&&", rm_rf, "."]))
}

# ---------------------------------------------------------------------------
# ROUTING: Bash only, PreToolUse only
#
# Edit/Write to `.cupcake/` are ALLOWED BY DESIGN. That is the whole difference
# between this rule and the lockdown it replaces: a guard change should arrive as
# a reviewable diff, and an agent that cannot edit the policies cannot repair
# them either.
# ---------------------------------------------------------------------------

test_allow_write_tool_targeting_the_policy_tree if {
	decisions := guard.halt with input as {
		"hook_event_name": "PreToolUse",
		"tool_name": "Write",
		"tool_input": {
			"file_path": concat("/", [repo, cup, "policies/claude/new.rego"]),
			"content": "package x",
		},
		"resolved_file_path": concat("/", [repo, cup, "policies/claude/new.rego"]),
	}
	count(decisions) == 0
}

test_allow_read_tool_targeting_the_policy_tree if {
	decisions := guard.halt with input as {
		"hook_event_name": "PreToolUse",
		"tool_name": "Read",
		"tool_input": {"file_path": concat("/", [repo, cup, "rulebook.yml"])},
		"resolved_file_path": concat("/", [repo, cup, "rulebook.yml"]),
	}
	count(decisions) == 0
}

test_allow_post_tool_use_event if {
	decisions := guard.halt with input as {
		"hook_event_name": "PostToolUse",
		"tool_name": "Bash",
		"tool_input": {"command": concat(" ", [rm_rf, cup])},
	}
	count(decisions) == 0
}
