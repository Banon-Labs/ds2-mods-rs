# METADATA
# scope: package
# title: Do not end a turn with an unpushed game-code build running and its run evidence collected
# authors: ["ds2-mods-rs agents"]
# custom:
#   severity: HIGH
#   id: DS2-MODS-NO-UNSPENT-RUN-EVIDENCE
#   description: >-
#     User directive 2026-09-26: "as soon as you collect your commit sha evidence for runtime
#     testing that's required for a PR, you're obligated to tear down".
#
#     scripts/ds2-teardown-after-evidence-push.py tears the game down after the push that spends the
#     evidence. It keys on the push, so an agent that collects the evidence and does not push leaves
#     the game up indefinitely. The instance, the same day: commit 2f2d0a5 on estus-max-default-on
#     was launched, its `build git=2f2d0a5...` line was read out of ds2-loader.log, and the turn
#     ended with "I haven't pushed yet because the push hook would close this game".
#
#     This halts that turn end. The signal speaks only when the game is alive, the log has the attach
#     line and a clean `build git=<sha>`, that commit is in a local worktree HEAD and on no remote
#     branch, and a worktree holding it changes game code against origin/main. A pushed or merged
#     build (the user's own play sessions), a dirty build, no game, a branch with no game code, and
#     anything unknown stay silent. The decision is in scripts/cupcake_unspent_evidence.py.
#
#     It is also silent while the user hand-tests that worktree's builds (ds2-mods-rs-wdnr,
#     2026-09-27): the user has written to the session after a launch from the worktree holding the
#     running commit, and the turn ends within 30 minutes of that message. Without it the halt fired
#     on every relaunch of an in-game save-picker test, and the push it demands closes the game.
#
#     Known gap, shared by every Stop guard here: an interrupted turn fires no Stop event.
#   routing:
#     required_events: ["Stop"]
#     required_signals: ["unspent_run_evidence"]
package cupcake.policies.claude.no_unspent_run_evidence

import rego.v1

halt contains decision if {
	input.hook_event_name == "Stop"
	sha != ""
	decision := {
		"rule_id": "DS2-MODS-NO-UNSPENT-RUN-EVIDENCE",
		"reason": concat("", [
			"The running game is unpushed commit ", substring(sha, 0, 12), branch_note,
			" and its run evidence is already in ds2-loader.log. Push it now; the teardown hook then closes the game.",
		]),
		"severity": "HIGH",
	}
}

branch_note := concat("", [" on ", branch]) if branch != "" else := ""

# --- signal parsing ------------------------------------------------------------------------------
# UNSPENTEVIDENCE|branch=<branch>|sha=<sha>
branch_marker := "|branch="

sha_marker := "|sha="

raw := trim(matched_facts, " \t\r\n")

sha_at := indexof(raw, sha_marker)

branch_at := indexof(raw, branch_marker)

sha := s if {
	startswith(raw, "UNSPENTEVIDENCE")
	sha_at >= 0
	s := trim(substring(raw, sha_at + count(sha_marker), -1), " \t\r\n")
	regex.match(`^[0-9a-f]{7,40}$`, s)
} else := ""

branch := b if {
	branch_at >= 0
	sha_at > branch_at
	start := branch_at + count(branch_marker)
	b := substring(raw, start, sha_at - start)
} else := ""

matched_facts := p if {
	p := input.signals.unspent_run_evidence
	is_string(p)
} else := p if {
	p := input.signals.unspent_run_evidence.output
} else := ""
