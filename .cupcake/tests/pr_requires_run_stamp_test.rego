# OPA unit tests for pr_requires_run_stamp.
#   opa test .cupcake/system/commands.rego .cupcake/policies/claude/pr_requires_run_stamp.rego \
#     .cupcake/tests/pr_requires_run_stamp_test.rego
#
# This file pins the RULE: which verdict the signal reports is refused, with which sentence, and that
# a gh command the signal failed to read is refused rather than waved through. Which body earns which
# verdict -- missing stamp, wrong sha, stale, result=fail at ready -- is decided in
# scripts/cupcake_run_stamp.py and pinned by scripts/test-run-stamp.py; the live engine path is pinned
# by scripts/test-cupcake-policies.py.
package cupcake.policies.claude.pr_requires_run_stamp_test

import rego.v1

import data.cupcake.policies.claude.pr_requires_run_stamp as guard

sha := "0123456789abcdef0123456789abcdef01234567"

ev(cmd, sig) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Bash",
	"tool_input": {"command": cmd},
	"signals": {"pr_run_stamp": sig},
}

create_cmd := "gh pr create --draft --title t --body-file /tmp/b.md"

ready_cmd := "gh pr ready 69"

denied(e) if {
	some d in guard.deny with input as e
	d.rule_id == "DS2-MODS-PR-RUN-STAMP"
}

reason(e) := r if {
	some d in guard.deny with input as e
	r := d.reason
}

test_create_with_good_stamp_allowed if {
	not denied(ev(create_cmd, concat("", ["RUNSTAMP|verb=create|ok=1|why=ok|sha=", sha])))
}

test_create_missing_stamp_denied if {
	e := ev(create_cmd, "RUNSTAMP|verb=create|ok=0|why=missing")
	denied(e)
	contains(reason(e), "no `Run-Stamp:` line")
}

test_create_wrong_sha_denied_and_names_head if {
	e := ev(create_cmd, concat("", ["RUNSTAMP|verb=create|ok=0|why=wrong-sha|sha=", sha]))
	denied(e)
	contains(reason(e), sha)
}

test_create_stale_denied if {
	e := ev(create_cmd, "RUNSTAMP|verb=create|ok=0|why=stale")
	denied(e)
	contains(reason(e), "older than the commit")
}

test_ready_good_allowed if {
	not denied(ev(ready_cmd, concat("", ["RUNSTAMP|verb=ready|ok=1|why=ok|sha=", sha])))
}

test_ready_stale_sha_denied if {
	e := ev(ready_cmd, concat("", ["RUNSTAMP|verb=ready|ok=0|why=wrong-sha|sha=", sha]))
	denied(e)
	contains(reason(e), "different commit")
}

test_ready_failed_run_denied if {
	e := ev(ready_cmd, "RUNSTAMP|verb=ready|ok=0|why=not-pass")
	denied(e)
	contains(reason(e), "result=fail")
}

test_ready_unreadable_pr_denied if {
	denied(ev(ready_cmd, "RUNSTAMP|verb=ready|ok=0|why=no-pr"))
}

# Fail closed: the command names the verb and the signal said nothing, or said "none".
test_create_with_silent_signal_denied if {
	denied(ev(create_cmd, ""))
}

test_ready_with_none_signal_denied if {
	denied(ev(ready_cmd, "RUNSTAMP|verb=none"))
}

test_garbage_signal_denied if {
	denied(ev(create_cmd, "RUNSTAMP|verb=|ok=1"))
}

test_object_shaped_signal_read if {
	e := {
		"hook_event_name": "PreToolUse",
		"tool_name": "Bash",
		"tool_input": {"command": create_cmd},
		"signals": {"pr_run_stamp": {"output": "RUNSTAMP|verb=create|ok=0|why=missing"}},
	}
	denied(e)
}

# Out of jurisdiction.
test_ready_undo_not_gated if {
	not denied(ev("gh pr ready 69 --undo", "RUNSTAMP|verb=none"))
}

test_other_gh_not_gated if {
	not denied(ev("gh pr view 69 --json body", "RUNSTAMP|verb=none"))
}

test_quoted_mention_not_gated if {
	not denied(ev("git commit -m 'gate gh pr create on a stamp'", "RUNSTAMP|verb=none"))
}

test_plain_command_not_gated if {
	not denied(ev("ls -la", "RUNSTAMP|verb=none"))
}
