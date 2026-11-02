# OPA unit tests for no_unmarked_user_claim_relay.
#
# Not loaded by the cupcake engine (which scans .cupcake/policies/<harness>/
# and .cupcake/system/ only). Run with:
#   opa test .cupcake/system/commands.rego \
#            .cupcake/policies/claude/no_unmarked_user_claim_relay.rego \
#            .cupcake/tests/no_unmarked_user_claim_relay_test.rego
# End-to-end engine coverage lives in scripts/test-cupcake-policies.py.
package cupcake.policies.claude.no_unmarked_user_claim_relay_test

import rego.v1

import data.cupcake.policies.claude.no_unmarked_user_claim_relay as guard

RULE := "DS2-MODS-NO-UNMARKED-USER-CLAIM-RELAY"

agent_event(prompt) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Agent",
	"tool_input": {"description": "test case", "prompt": prompt, "subagent_type": "general-purpose"},
}

task_event(prompt) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Task",
	"tool_input": {"description": "test case", "prompt": prompt},
}

send_event(message) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "SendMessage",
	"tool_input": {"to": "kit-builder", "summary": "test case", "message": message},
}

rule_ids(denials) := {d.rule_id | some d in denials}

denied(event) if {
	denials := guard.deny with input as event
	RULE in rule_ids(denials)
}

allowed(event) if not denied(event)

# The message that was sent on 2026-10-30, verbatim.
estus := "Correction to the filter: keep the Estus Flask in invader kits. The user (2026-10-30) says vanilla disables Estus for invaders and Seamless re-enables it. The binary fits that: ... So the rule is: drop items ... except Estus, which Seamless exempts."

estus_marked := "Correction to the filter: keep the Estus Flask in invader kits. The user (2026-10-30) says vanilla disables Estus for invaders and Seamless re-enables it (unverified -- verify before relying on it). The binary fits that: ... So the rule is: drop items ... except Estus, which Seamless exempts."

# --- the 2026-10-30 failure ---------------------------------------------------

test_deny_estus_send_message_verbatim if {
	denied(send_event(estus))
}

test_deny_estus_as_agent_prompt if {
	denied(agent_event(estus))
}

test_deny_estus_as_task_prompt if {
	denied(task_event(estus))
}

test_allow_estus_with_unverified_marker if {
	allowed(send_event(estus_marked))
}

test_allow_estus_marked_in_agent_prompt if {
	allowed(agent_event(estus_marked))
}

# The marker may open the next sentence instead of sitting inside the claim's own.
test_allow_marker_in_next_sentence if {
	allowed(send_event("The user (2026-10-30) says vanilla disables Estus for invaders and Seamless re-enables it. Unverified: check the Seamless DLL before relying on it."))
}

# A marker two sentences away is not on the claim; this is the shape where one "verified" elsewhere
# in a long prompt would otherwise bless every relayed claim in it.
test_deny_marker_two_sentences_away if {
	denied(send_event("The user says vanilla disables Estus for invaders. The binary fits that. Unverified: the arrow offset in the overlay."))
}

# The denial quotes the offending sentence so the agent knows which one to mark.
test_reason_names_the_sentence if {
	some d in guard.deny with input as send_event(estus)
	contains(d.reason, "The user (2026-10-30) says vanilla disables Estus")
	contains(d.reason, "(unverified -- verify before relying on it)")
}

# --- attribution shapes that are claims ---------------------------------------

test_deny_user_says_no_article if {
	denied(agent_event("User says the arrow points backwards after a load."))
}

test_deny_the_user_said if {
	denied(agent_event("The user said the game crashes when the overlay draws its first frame."))
}

test_deny_user_reports if {
	denied(agent_event("The user reports that the invasion path vanishes after a bonfire rest."))
}

test_deny_user_dated_reports if {
	denied(agent_event("User (2026-10-29) reports the boss arena fog wall is missing."))
}

test_deny_per_the_user if {
	denied(agent_event("Per the user, Seamless re-enables Estus for invaders."))
}

test_deny_according_to_the_user if {
	denied(agent_event("According to the user, the param row for Estus is 60155000."))
}

test_deny_user_also_confirmed if {
	denied(agent_event("The user also confirmed that the dodge roll has 13 i-frames."))
}

test_deny_users_claim_noun if {
	denied(agent_event("The user's claim is that Seamless re-enables Estus."))
}

test_deny_user_saw if {
	denied(agent_event("The user saw the overlay draw on the wrong monitor."))
}

test_deny_user_told_me if {
	denied(agent_event("Earlier the user told me the save picker opens on the second slot."))
}

test_deny_user_says_colon_quote if {
	denied(agent_event("The user says: \"vanilla disables Estus\". Build the filter on that."))
}

test_deny_in_bullet_list if {
	denied(agent_event("Context:\n- the kit drops invader-disabled items\n- the user says Seamless re-enables Estus\n- build the filter"))
}

test_deny_case_insensitive if {
	denied(agent_event("THE USER SAYS the game uses dinput8 for input."))
}

# A bare "verified" or "confirmed" in the claim is the user's word, not a check.
test_deny_user_confirmed_is_not_a_marker if {
	denied(agent_event("The user says they confirmed it in game."))
}

test_deny_user_says_they_verified_it if {
	denied(agent_event("The user says they verified it."))
}

# --- markers that pass ----------------------------------------------------------

test_allow_verified_with_source if {
	allowed(agent_event("The user says the param row is 60155000 (verified: ItemParam dump in regulation.bin, row 60155000)."))
}

test_allow_measured_in if {
	allowed(agent_event("The user reports the arrow points backwards, measured in run br-20261030-1100 ds2-loader.log."))
}

test_allow_confirmed_by_source if {
	allowed(agent_event("Per the user, Estus is disabled for invaders in vanilla; confirmed by the disassembly at DarkSoulsII.exe+0x2a1f30."))
}

test_allow_hypothesis if {
	allowed(agent_event("The user says Seamless re-enables Estus -- treat it as a hypothesis and check the Seamless DLL first."))
}

test_allow_hypothesis_to_check_form if {
	allowed(send_event("The user says Seamless re-enables Estus. Hypothesis to check: grep the Seamless DLL for the Estus item id."))
}

test_allow_to_verify if {
	allowed(agent_event("The user said the save picker opens on slot two, to verify against the menu state before relying on it."))
}

test_allow_not_yet_verified if {
	allowed(agent_event("The user said the save picker opens on slot two; not yet verified."))
}

test_allow_unconfirmed if {
	allowed(agent_event("The user believes the fog wall is missing (unconfirmed)."))
}

test_allow_users_hypothesis_noun if {
	allowed(agent_event("The user's hypothesis is that Seamless re-enables Estus."))
}

# --- decisions and preferences pass -------------------------------------------

test_allow_user_decided if {
	allowed(send_event("Correction to the filter: the user decided to keep the Estus Flask in invader kits."))
}

test_allow_user_wants if {
	allowed(agent_event("The user wants the overlay off by default."))
}

test_allow_users_rule if {
	allowed(agent_event("The user's rule: never act on unfounded guidance."))
}

test_allow_user_asked if {
	allowed(agent_event("The user asked for a PR, not a direct push."))
}

test_allow_user_prefers if {
	allowed(agent_event("The user prefers Banon-Labs for new repositories."))
}

test_allow_user_directive if {
	allowed(agent_event("User directive 2026-10-30: keep Estus in invader kits."))
}

test_allow_user_says_to if {
	allowed(send_event("The user says to keep the Estus Flask in invader kits."))
}

test_allow_user_said_not_to if {
	allowed(agent_event("The user said not to touch the running game."))
}

test_allow_user_says_we_should if {
	allowed(agent_event("The user says we should open a PR instead of pushing to main."))
}

test_allow_user_said_they_want if {
	allowed(agent_event("The user said they want the kit to carry every ammo slot."))
}

test_allow_user_says_go_ahead if {
	allowed(send_event("The user says go ahead with the merge."))
}

# --- quoted text is about the sentence, not a relay of it ---------------------

# A prompt quoting the failure (this guard's own task did) is not relaying the claim.
test_allow_quoted_failure_message if {
	allowed(agent_event(concat("", ["The failure to prevent: the orchestrator wrote \"", estus, "\" and the subagent acted on it."])))
}

test_allow_curly_quoted_failure_message if {
	allowed(agent_event(concat("", ["The orchestrator wrote “", estus, "”."])))
}

test_allow_backticked_fixture if {
	allowed(agent_event("Add a fixture for `The user says vanilla disables Estus.` to the test file."))
}

# --- no attribution, no judgement ----------------------------------------------

test_allow_plain_task if {
	allowed(agent_event("Find every caller of GameManagerImp::update and report their RVAs."))
}

test_allow_user_as_noun_not_attribution if {
	allowed(agent_event("Read user32.dll imports and the userdata path, then report the user-facing strings."))
}

test_allow_empty_prompt if {
	allowed(agent_event(""))
}

# --- other tools and events are never judged ------------------------------------

test_allow_bash_with_claim_text if {
	allowed({
		"hook_event_name": "PreToolUse",
		"tool_name": "Bash",
		"tool_input": {"command": concat("", ["echo '", estus, "'"])},
	})
}

test_allow_write_with_claim_text if {
	allowed({
		"hook_event_name": "PreToolUse",
		"tool_name": "Write",
		"tool_input": {"file_path": "/tmp/x.md", "content": estus},
	})
}

# Agent's `prompt` key on SendMessage is not its message, and vice versa.
test_allow_send_message_reads_message_not_prompt if {
	allowed({
		"hook_event_name": "PreToolUse",
		"tool_name": "SendMessage",
		"tool_input": {"to": "x", "message": "carry on", "prompt": estus},
	})
}

test_allow_post_tool_use if {
	allowed({
		"hook_event_name": "PostToolUse",
		"tool_name": "SendMessage",
		"tool_input": {"to": "x", "message": estus},
	})
}
