# OPA unit tests for git_require_runtime_test_before_push.
# Run with:
#   opa test .cupcake/system/commands.rego \
#     .cupcake/policies/claude/git_require_runtime_test_before_push.rego \
#     .cupcake/tests/git_require_runtime_test_before_push_test.rego
#
# The guard shipped with no tests at all, which is the state in which a Rego policy is
# indistinguishable from a comment: an unrouted rule, an unprobed builtin and a rule whose body can
# never be satisfied all return ALLOW at exit 0 and read exactly like a guard that is holding.
# These pin the three fields of the signal against the two things the guard must never do -- let
# unrun game code through, and deny work it has no jurisdiction over.
package cupcake.policies.claude.git_require_runtime_test_before_push_test

import rego.v1

import data.cupcake.policies.claude.git_require_runtime_test_before_push as guard

# The signal's own output format, verbatim:
#   RUNTIME|game_code=<0|1>|attached=<0|1>|fresh=<0|1>|dll_match=<0|1>|head=<epoch>|log=<epoch>
# The epochs are carried for the human reading a denial, not for the decision, so they are only
# internally consistent here.

# Game code, the DLL has been loaded, the log postdates HEAD and the staged binary, and the staged
# binary is the one this checkout built. The one satisfying state.
proven := "RUNTIME|game_code=1|attached=1|fresh=1|dll_match=1|head=1758600000|log=1758600900"

# Game code and no `ds2-loader: attach` line anywhere: the DLL has never been in a process.
never_attached := "RUNTIME|game_code=1|attached=0|fresh=0|dll_match=1|head=1758600000|log=0"

# Game code, the DLL HAS loaded at some point, but the log predates HEAD -- a run of older code.
stale_log := "RUNTIME|game_code=1|attached=1|fresh=0|dll_match=1|head=1758600000|log=1758500000"

# THE SHAPE THE CLOCK ALONE CANNOT SEE. A launch happened after the commit and a DLL loaded, so
# `attached` and `fresh` both say yes -- but the bytes in the game directory are not the bytes this
# checkout built. That is a parallel worktree's run, or a plain Steam launch of a stale DLL. The
# clock-only version of this guard allowed it; see the measurement in the signal's header.
foreign_binary := "RUNTIME|game_code=1|attached=1|fresh=1|dll_match=0|head=1758600000|log=1758600900"

# A branch touching neither `crates/` nor the launcher. Out of jurisdiction, so the runtime fields
# are deliberately as bad as they get: policies and docs must push with no game launch at all.
no_game_code := "RUNTIME|game_code=0|attached=0|fresh=0|dll_match=0|head=1758600000|log=0"

# A signal in the format that predates `dll_match`: every field it does carry says the run was clean.
# Requiring the field means an old or replaced signal script denies rather than vouching.
pre_dll_match_format := "RUNTIME|game_code=1|attached=1|fresh=1|head=1758600000|log=1758600900"

# The signal could not tell, and said so by saying nothing. Every `exit 0` path in the script that
# does not reach the final printf produces this.
silent := ""

# Output that is not this signal's at all -- a stray error line on stdout, a renamed signal, a
# different script answering to the name.
foreign := "fatal: not a git repository"

# THE SHAPE A NAIVE PREFIX TEST LETS THROUGH. It begins `RUNTIME|`, so "is the signal there?"
# answers yes, while not one field can be read. Before the game_code field was required to be
# readable this was ALLOWED: `game_code` came back "" so the game-code arm did not fire, and the
# missing-signal arm did not fire either because the prefix was present. A guard is only fail-closed
# if unreadable and absent are the same verdict.
truncated := "RUNTIME|game_code=|attached=|fresh="

event(command, signal) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Bash",
	"tool_input": {"command": command, "timeout": 30000},
	"signals": {"runtime_evidence_for_head": signal},
}

# Cupcake delivers a signal either as a bare string or as `{"output": ..., "exit_code": ...}`
# depending on how it was declared, and a guard that reads only one of them is inert under the
# other. Both shapes are exercised.
event_object_signal(command, signal) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Bash",
	"tool_input": {"command": command, "timeout": 30000},
	"signals": {"runtime_evidence_for_head": {"output": signal, "exit_code": 0}},
}

event_no_signal(command) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Bash",
	"tool_input": {"command": command, "timeout": 30000},
	"signals": {},
}

rule_ids(denials) := {d.rule_id | some d in denials}

blocked(command, signal) if {
	denials := guard.deny with input as event(command, signal)
	"DS2-MODS-REQUIRE-RUNTIME-BEFORE-PUSH" in rule_ids(denials)
}

allowed(command, signal) if {
	denials := guard.deny with input as event(command, signal)
	count(denials) == 0
}

reasons(command, signal) := {d.reason | some d in guard.deny with input as event(command, signal)}

# --- The three fields decide -------------------------------------------------

test_allow_push_of_game_code_that_has_been_run if {
	allowed("git push origin quit-menu-file-rows", proven)
}

test_deny_push_of_game_code_that_has_never_loaded if {
	blocked("git push origin quit-menu-file-rows", never_attached)
}

test_deny_push_of_game_code_whose_last_run_predates_head if {
	blocked("git push origin quit-menu-file-rows", stale_log)
}

test_deny_push_when_the_run_used_a_binary_this_checkout_did_not_build if {
	blocked("git push origin quit-menu-file-rows", foreign_binary)
}

test_deny_push_when_the_signal_predates_the_binary_check if {
	blocked("git push origin quit-menu-file-rows", pre_dll_match_format)
}

# The carve-out that keeps the guard from being routed around: a Rego fix, a doc or the beads export
# must not require a game launch. This is asserted with the WORST possible runtime evidence, so a
# pass here cannot come from the runtime fields accidentally looking satisfied.
test_allow_push_with_no_game_code_and_no_runtime_evidence_at_all if {
	allowed("git push origin cupcake-guard-layer-port", no_game_code)
}

# --- Fail closed on anything it cannot read ----------------------------------

test_deny_push_when_signal_key_is_absent if {
	denials := guard.deny with input as event_no_signal("git push origin quit-menu-file-rows")
	"DS2-MODS-REQUIRE-RUNTIME-BEFORE-PUSH" in rule_ids(denials)
}

test_deny_push_when_signal_is_silent if {
	blocked("git push origin quit-menu-file-rows", silent)
}

test_deny_push_when_signal_output_is_foreign if {
	blocked("git push origin quit-menu-file-rows", foreign)
}

test_deny_push_when_signal_has_the_prefix_but_no_readable_fields if {
	blocked("git push origin quit-menu-file-rows", truncated)
}

test_deny_push_when_object_form_signal_is_unrun if {
	denials := guard.deny with input as event_object_signal("git push origin quit-menu-file-rows", never_attached)
	"DS2-MODS-REQUIRE-RUNTIME-BEFORE-PUSH" in rule_ids(denials)
}

test_allow_push_when_object_form_signal_is_proven if {
	denials := guard.deny with input as event_object_signal("git push origin quit-menu-file-rows", proven)
	count(denials) == 0
}

# --- The denial has to say WHICH field failed --------------------------------
#
# The reason string is the only part of this guard an agent reads, and the two failures send you to
# different places: one means launch the game, the other means you already did but before you
# committed. An else-chain that answers the wrong one is a silent regression `count(denials)` cannot
# see.

test_reason_names_the_missing_attach_line if {
	some reason in reasons("git push origin quit-menu-file-rows", never_attached)
	contains(reason, "ds2-loader: attach")
}

test_reason_names_the_stale_log_when_the_dll_has_loaded_before if {
	some reason in reasons("git push origin quit-menu-file-rows", stale_log)
	contains(reason, "predates HEAD")
}

test_reason_names_the_binary_mismatch_when_the_clock_looks_fine if {
	some reason in reasons("git push origin quit-menu-file-rows", foreign_binary)
	contains(reason, "not the one this checkout built")
}

test_reason_names_unreadable_evidence_when_the_signal_is_silent if {
	some reason in reasons("git push origin quit-menu-file-rows", silent)
	contains(reason, "No runtime evidence could be read")
}

# The cheap fix, which is an ordering and not a code change, and which this guard cost three game
# restarts in one session for want of saying. Commit, then build, then launch: the floor is
# `max(HEAD commit time, staged DLL mtime)`, so a run taken before the commit can never clear it no
# matter how recent it is. Asserted on the stale-log arm because that is the failure it answers.
test_reason_tells_you_to_commit_before_launching if {
	some reason in reasons("git push origin quit-menu-file-rows", stale_log)
	contains(reason, "Commit before you launch")
}

# --- Jurisdiction: only a push, and only one that runs -----------------------

test_allow_non_push_git_commands_when_unrun if {
	allowed("git status --short && git log --oneline -3", never_attached)
	allowed("git fetch origin main", never_attached)
	allowed("git commit -m 'wip'", never_attached)
}

test_allow_rebase_onto_origin_main_when_unrun if {
	allowed("git rebase origin/main", never_attached)
}

test_allow_a_push_that_is_not_gits_when_unrun if {
	allowed("cargo publish --dry-run", never_attached)
	allowed("docker push ghcr.io/banon-labs/x", never_attached)
}

test_deny_git_dash_c_push_when_unrun if {
	blocked("git -C /home/banon/projects/ds2-mods-rs push origin quit-menu-file-rows", never_attached)
}

test_deny_push_chained_behind_an_innocent_command_when_unrun if {
	blocked("./scripts/check.sh && git push origin quit-menu-file-rows", never_attached)
}

# --- Quoted TEXT is not an executed push -------------------------------------
#
# This guard's own documentation, and every beads note about it, has to be writable from inside the
# repo that enforces it. A guard that denies prose quoting the command it blocks cannot be
# documented, and an undocumentable guard gets deleted.

test_allow_prose_quoting_a_push_when_unrun if {
	allowed(`echo "run the game before you git push"`, never_attached)
}

test_allow_beads_memory_body_quoting_a_push_when_unrun if {
	allowed("$HOME/.local/bin/bd remember --key k \"before\ngit push origin main\nafter\"", never_attached)
}

test_allow_heredoc_documenting_a_push_when_unrun if {
	allowed("cat > docs/guards.md <<'EOF'\ngit push origin quit-menu-file-rows\nEOF", never_attached)
}

test_allow_commit_message_naming_the_rule_when_unrun if {
	allowed(`git commit -m "guard: no push until the DLL has been in a process"`, never_attached)
}

# --- Shell-wrapper payloads ---------------------------------------------------
#
# AGENTS.md tells agents to wrap non-trivial commands as `bash -c "<cmd>"` because the interactive
# shell here is fish. That makes the wrapper the repo's own recommended spelling, not an exotic
# bypass, so the guard has to read through it in both directions.

test_deny_wrapped_push_when_unrun if {
	blocked("bash -c 'git push origin quit-menu-file-rows'", never_attached)
	blocked(`sh -c "git push"`, never_attached)
	blocked("fish -c 'git push origin quit-menu-file-rows'", never_attached)
}

test_deny_nested_wrapped_push_when_unrun if {
	blocked(`bash -c 'bash -c "git push"'`, never_attached)
}

test_deny_shell_read_heredoc_push_when_unrun if {
	blocked("bash <<'EOF'\ngit push origin quit-menu-file-rows\nEOF", never_attached)
}

test_allow_wrapped_push_when_proven if {
	allowed("bash -c 'git push origin quit-menu-file-rows'", proven)
}

# --- Unreadable payloads fail closed, but only inside the jurisdiction -------
#
# A payload the guard cannot decompose is indistinguishable from one that pushes, so it is refused
# on the same evidence a readable push would be refused on -- and permitted on the same evidence.
# Denying it while the evidence is GOOD would be the guard answering a question nobody asked, which
# is how a guard earns the reputation that gets it switched off.

test_deny_unreadable_wrapper_payload_naming_git_and_push_when_unrun if {
	blocked("bash -c $GIT_PUSH_CMD", never_attached)
	blocked("eval $PUSH_THE_GIT_BRANCH", never_attached)
}

test_deny_unreadable_wrapper_payload_naming_git_and_push_when_signal_silent if {
	blocked("bash -c $GIT_PUSH_CMD", silent)
}

test_allow_unreadable_wrapper_payload_when_proven if {
	allowed("bash -c $GIT_PUSH_CMD", proven)
}

test_allow_unreadable_wrapper_payload_when_branch_has_no_game_code if {
	allowed("bash -c $GIT_PUSH_CMD", no_game_code)
}

test_allow_unreadable_wrapper_payload_outside_the_jurisdiction if {
	allowed("bash -c $BUILD_CMD", never_attached)
	allowed("bash -c $GIT_STATUS_CMD", never_attached)
}

# --- Routing is part of the rule ---------------------------------------------
#
# The metadata routes this to PreToolUse/Bash only. If the body ever stops checking that itself, a
# Stop-event transcript mentioning a push would be denied.

test_allow_push_text_on_a_non_pretooluse_event if {
	denials := guard.deny with input as {
		"hook_event_name": "Stop",
		"tool_name": "Bash",
		"tool_input": {"command": "git push origin quit-menu-file-rows"},
		"signals": {"runtime_evidence_for_head": never_attached},
	}
	count(denials) == 0
}

test_allow_push_text_from_a_non_bash_tool if {
	denials := guard.deny with input as {
		"hook_event_name": "PreToolUse",
		"tool_name": "Write",
		"tool_input": {"file_path": "/home/banon/notes.md", "content": "git push origin main\n"},
		"signals": {"runtime_evidence_for_head": never_attached},
	}
	count(denials) == 0
}

# --- a commit chained in front of the push (2026-09-25) -------------------------------------------
# The command that walked through, verbatim. The hook fired before the commit existed, HEAD was
# still origin/main, and the signal reported game_code=0. The signal now reads the command and
# reports `pending=1` plus the working tree's changes; every runtime field below is as good as it
# gets, because none of them can vouch for a commit that has not been made.
chained_commit_command := `git commit -qam "$(printf 'chore(scripts): the launcher takes no switch to play online\n')" && git push -u origin launcher-drop-flag 2>&1 | tail -3`

pending_game_code := "RUNTIME|game_code=1|attached=1|fresh=1|dll_match=1|pending=1|head=1758600000|log=1758600900"

pending_docs_only := "RUNTIME|game_code=0|attached=0|fresh=0|dll_match=0|pending=1|head=1758600000|log=0"

test_chained_commit_and_push_of_game_code_is_denied if {
	some d in guard.deny with input as event(chained_commit_command, pending_game_code)
	d.rule_id == "DS2-MODS-REQUIRE-RUNTIME-BEFORE-PUSH"
	contains(d.reason, "commits and pushes in one go")
}

# A chained commit of docs or policies stays out of jurisdiction, as a separate commit of them does.
test_chained_commit_and_push_of_docs_is_allowed if {
	count(guard.deny) == 0 with input as event("git commit -am 'docs: x' && git push", pending_docs_only)
}

# The signal format before `pending` existed still decides as it did: an absent field is not "1".
test_signal_without_pending_field_still_proves if {
	count(guard.deny) == 0 with input as event("git push -u origin b", proven)
}
