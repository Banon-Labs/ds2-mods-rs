# METADATA
# scope: package
# title: Block Detached Pushes
# authors: ["er-mods-rs agents", "ds2-mods-rs agents"]
# custom:
#   severity: HIGH
#   id: DS2-MODS-BLOCK-DETACHED-PUSH
#   routing:
#     required_events: ["PreToolUse"]
#     required_tools: ["Bash"]
package cupcake.policies.claude.git_block_detached_push

import rego.v1

import data.cupcake.system.commands

# A push in this repository runs the whole local gate suite and takes ten to fifteen minutes. That
# is past the harness cap on a backgrounded Bash command, so the obvious move is to detach it --
# `setsid nohup git push ... &` -- and the obvious move is the one that costs the user half an hour.
#
# Measured 2026-09-14. A detached push is severed from the harness: nothing remains to report its
# exit, so the only route back to the result is the agent choosing to go and read a log. It did not
# choose to. The push failed at 09:29:45 and was noticed at 09:59:07, twenty-nine minutes of a user
# waiting on a run that was already over. Arming a `Monitor` afterwards does not rescue it either:
# `tail -f` on a log nothing is writing to any more replays the history once and then watches a
# dead file for as long as it is allowed to, which on the terminal looks exactly like a run still
# in progress.
#
# So the rule is about the SHAPE of the invocation, not about intent. `scripts/er-push-watched.sh`
# runs the push in the foreground of whatever process called it; pointed at by a `Monitor`, the
# monitor process is the push, stage verdicts arrive as they happen, and the monitor ending is the
# completion notification. Nothing to poll, nothing to tear down separately.
#
# Why this is a policy and not a note. `AGENTS.md` is explicit that instruction files and `bd`
# memories are advisory and can be missed; binding behaviour needs executable enforcement. A memory
# recording "never detach a push" was written the same day and would have been read by nobody at
# the moment it mattered.
#
# What it deliberately does NOT do: it does not require a `Monitor`. A policy cannot see which tool
# will consume a command's output, so demanding one would be a check this text cannot make. It
# refuses the detachment, which is visible, and names the shape that works.
deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"
	some text in executed_texts
	lowered := lower(text)
	is_git_push(lowered)
	detached(lowered)
	not is_watched_push(lowered)

	decision := {
		"rule_id": "DS2-MODS-BLOCK-DETACHED-PUSH",
		"reason": "Do not detach a push. `setsid`/`nohup`/`&` severs it from the harness, so nothing reports its exit and the result is only found by reading a log by hand -- that cost 29 minutes on 2026-09-14. Run it inside a Monitor instead, where the monitor process is the push and its ending is the notification: Monitor(timeout_ms 3600000) over `bash scripts/er-push-watched.sh <local-ref> <remote-branch> | python3 scripts/monitor-throttle.py 15`.",
		"severity": "HIGH",
	}
}

executed_texts := commands.executed_texts(input.tool_input.command)

# The main-push guard's recogniser, with one addition this policy needs: `setsid` and `nohup` may
# sit between the command anchor and `git`. That guard anchors on `git` directly, so `setsid nohup
# git push` is not a push to it -- which is correct there (the detaching words change nothing about
# which ref is sent) and wrong here, since those words are the entire subject.
git_push_command_pattern := `(^|[;&|(
])\s*((setsid|nohup)[ \t]+)*(command\s+)?git([ \t]+((-c|--git-dir|--work-tree|--namespace|--config-env)(=|[ \t]+)("[^"\n]*"|'[^'\n]*'|[^ \t;&|()\n]+)|--(bare|no-pager|paginate|literal-pathspecs|no-replace-objects|exec-path)(=("[^"\n]*"|'[^'\n]*'|[^ \t;&|()\n]+))?))*[ \t]+push([ \t;&|)\n]|$)`

is_git_push(cmd) if {
	regex.match(git_push_command_pattern, cmd)
}

# The three ways a push gets cut loose here. `&` is matched only at the end of a command or before a
# separator, so `&&` -- which chains rather than detaches -- does not trip it.
detached(cmd) if {
	regex.match(`(^|[;&|(\s])(setsid|nohup)([ \t]|$)`, cmd)
}

detached(cmd) if {
	regex.match(`[^&|]&\s*(;|$|\n)`, cmd)
}

detached(cmd) if {
	regex.match(`(^|[;&|(\s])disown([ \t;&|)\n]|$)`, cmd)
}

# The sanctioned wrapper is exempt even though a caller may background it: it prints a terminal
# `push: ok` / `push: failed` line, so a reader of its output always learns the outcome.
is_watched_push(cmd) if {
	contains(cmd, "er-push-watched.sh")
}
