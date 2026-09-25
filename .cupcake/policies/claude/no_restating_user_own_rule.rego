# METADATA
# scope: package
# title: Do not hand the user back a rule the user wrote
# authors: ["er-quickload agents", "ds2-mods-rs agents (ported from er-mods-rs)"]
# custom:
#   severity: HIGH
#   id: DS2-MODS-NO-RESTATING-USER-OWN-RULE
#   description: >-
#     PORTED from er-mods-rs on 2026-09-25. One divergence, in scripts/cupcake_user_own_rule.py:
#     pushing is not one of the user-owned actions here, because CLAUDE.md makes the push the
#     agent's job. Undrafting (the global GLOBAL-GITHUB-PR-DRAFT-GUARD), merging, publishing and
#     releasing stay the user's, and restating any of those back to them is what this halts.
#     The instance and history below are er-mods-rs's.
#
#     Halts turn-end when the closing message assigns an action to the user, or denies it to the
#     agent, on the strength of a standing rule the USER authored -- "undrafting is yours", "I don't
#     push", "leaving it draft for you".
#
#     MEASURED 2026-09-23. A turn that had just opened draft pull request #481 closed with "It stays
#     draft -- undrafting is yours." The user owns that rule: it lives in one of their memories and
#     in the global policy that denies `gh pr ready`. Their reply is this policy's specification --
#     "If undrafting is mine, because I told you it was mine, does it need to be stated? Is there a
#     way to guard against you telling me details that I provided to you through rego policies and
#     agent instructions that I already knew?"
#
#     WHY IT COSTS SOMETHING RATHER THAN BEING MERELY REDUNDANT. `wall_of_text` measures what this
#     user actually reads: the first paragraph, and little after it. Every clause spent confirming
#     an instruction they wrote is a clause not spent on the measurement, the failure, or the number
#     they cannot get anywhere else. The redundancy is not the harm; the displacement is.
#
#     WHY THE NEIGHBOURING STOP GUARDS MISS IT. `no_described_next_step` needs a step the agent
#     could have started, and undrafting is one it may never start -- so that guard is correctly
#     silent. `no_narrated_action` needs a present participle. `wall_of_text` charges length, and
#     the sentence is six words. None of them ask whether the reader already owns the fact.
#
#     THE TWO EXEMPTIONS, both of which carry something the user does not have. `command` -- the
#     message hands over the pasteable `git push` / `gh pr ready`, which `AGENTS.md` positively
#     REQUIRES for work the agent may not do; charging for that would delete one instruction to
#     satisfy another. `guardevent` -- the message reports a guard that fired during this turn,
#     which is an event they were not watching for rather than a rule they hold.
#
#     A fact about the tree is never this: "committed as 91b39cdf on branch <x>, unpushed" states
#     what exists, and the guard's fixtures pin that it stays sayable. The line is between the FACT
#     and the RULE.
#
#     The signal emits ONE facts line --
#     OWNRULEFACTS|clause=<clause>|ruleid=<id>|command=0|1|guardevent=0|1 -- so the OBSERVATION
#     lives in the shell and the RULE lives here, where it is unit-testable. Empty signal -> no
#     halt.
#
#     KNOWN GAP: an interrupted turn fires no Stop event, so a restatement the user cuts short is
#     not caught. The sibling pairs (no_authority_agreement + _reminder, idle_hold + _reminder)
#     close that with a UserPromptSubmit interlock reading the same signal; add one the same way if
#     the gap bites.
#   routing:
#     required_events: ["Stop"]
#     required_signals: ["last_assistant_user_own_rule"]
package cupcake.policies.claude.no_restating_user_own_rule

import rego.v1

halt contains decision if {
	input.hook_event_name == "Stop"
	restated_without_exemption
	decision := {
		"rule_id": "DS2-MODS-NO-RESTATING-USER-OWN-RULE",
		"reason": reason,
		"severity": "HIGH",
	}
}

# The conjunction. Either exemption is a turn shape allowed to say this: one handing over the
# command the user needs, one reporting a guard that actually fired.
restated_without_exemption if {
	clause != ""
	command == "0"
	guardevent == "0"
}

reason := msg if {
	msg := concat("", ["You ended the turn handing the user back a rule they wrote: '", clause, "'. They already own that fact -- they are the one who decided it, in their own instructions or a policy in this repo -- so the sentence tells them nothing and spends the one paragraph they read. State what EXISTS, not what the rules say: the pull request number and its URL, the branch, the commit, the measurement. If the point is that work remains for them, the useful form is the pasteable command, spelled absolutely, which AGENTS.md already asks for -- not a recital of who is permitted to run it. And if a guard refused something during this turn, say THAT: a guard firing is news, a guard existing is not."])
}

# --- signal parsing ------------------------------------------------------------------------------
# OWNRULEFACTS|clause=<clause>|ruleid=<id>|command=0|guardevent=0
# The leading tag carries no "=" so it drops out of the fact map on its own. A field the signal omits
# falls back to a default that does NOT exempt: a degraded or crafted signal fails closed and halts,
# matching how the sibling guards treat an untagged non-empty value.
fact[k] := v if {
	some kv in split(raw, "|")
	n := indexof(kv, "=")
	n > 0
	k := trim(substring(kv, 0, n), " \t\r\n")
	v := trim(substring(kv, n + 1, -1), " \t\r\n")
}

clause := object.get(fact, "clause", "")

command := object.get(fact, "command", "0")

guardevent := object.get(fact, "guardevent", "0")

raw := trim(matched_facts, " \t\r\n")

# Signal value tolerates both the bare-string and {output: ...} shapes cupcake may hand back.
matched_facts := p if {
	p := input.signals.last_assistant_user_own_rule
	is_string(p)
} else := p if {
	p := input.signals.last_assistant_user_own_rule.output
} else := ""
