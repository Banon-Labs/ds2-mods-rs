# OPA unit tests for git_block_detached_push.
# Run with:
#   opa test .cupcake/system/commands.rego \
#     .cupcake/policies/claude/git_block_detached_push.rego \
#     .cupcake/tests/git_block_detached_push_test.rego
package cupcake.policies.claude.git_block_detached_push_test

import rego.v1

import data.cupcake.policies.claude.git_block_detached_push as guard

bash_event(cmd) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Bash",
	"tool_input": {"command": cmd, "timeout": 30000},
	"signals": {},
}

rule_ids(denials) := {d.rule_id | some d in denials}

denied(cmd) if {
	denials := guard.deny with input as bash_event(cmd)
	"DS2-MODS-BLOCK-DETACHED-PUSH" in rule_ids(denials)
}

# The exact command that cost 29 minutes.
test_deny_setsid_nohup_push if {
	denied("setsid nohup git push --force-with-lease origin pr435-rebase:refs/heads/feature > log 2>&1 < /dev/null &")
}

test_deny_nohup_push if {
	denied("nohup git push origin feature/x &")
}

test_deny_trailing_ampersand_push if {
	denied("git push origin feature/x &")
}

test_deny_disowned_push if {
	denied("git push origin feature/x & disown")
}

test_deny_detached_push_inside_a_shell_wrapper if {
	denied("bash -c 'setsid git push origin feature/x &'")
}

test_deny_detached_git_c_push if {
	denied("setsid git -C .worktrees/w push origin feature/x &")
}

# A foreground push is the ordinary case and must stay allowed.
test_allow_foreground_push if {
	not denied("git push --force-with-lease origin feature/x:refs/heads/feature/x")
}

# `&&` chains, it does not detach. Getting this wrong would refuse most real command lines.
test_allow_push_chained_with_double_ampersand if {
	not denied("git fetch origin && git push origin feature/x")
}

test_allow_push_piped_to_a_filter if {
	not denied("git push origin feature/x 2>&1 | tail -20")
}

# The sanctioned wrapper prints a terminal verdict line of its own, so backgrounding it still tells
# the reader how the push ended.
test_allow_watched_push_even_when_detached if {
	not denied("setsid bash scripts/er-push-watched.sh pr435-rebase feature/x &")
}

test_allow_watched_push_under_a_monitor if {
	not denied("bash scripts/er-push-watched.sh pr435-rebase feature/x | python3 scripts/monitor-throttle.py 15")
}

# A detached command that is not a push is none of this policy's business.
test_allow_detached_non_push if {
	not denied("setsid nohup cargo build --release > build.log 2>&1 &")
}
