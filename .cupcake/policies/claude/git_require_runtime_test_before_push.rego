# METADATA
# scope: package
# title: No Push of Game Code That Has Never Been Run
# authors: ["ds2-mods-rs agents"]
# custom:
#   severity: HIGH
#   id: DS2-MODS-REQUIRE-RUNTIME-BEFORE-PUSH
#   routing:
#     required_events: ["PreToolUse"]
#     required_tools: ["Bash"]
#     required_signals: ["runtime_evidence_for_head"]
package cupcake.policies.claude.git_require_runtime_test_before_push

import rego.v1

import data.cupcake.system.commands

# THE RULE THE USER BELIEVED ALREADY EXISTED (2026-09-23, their words): "I thought your agent
# instructions were clear: no push until runtime tests." It did not exist in either AGENTS.md, and
# the repo's session-completion section says the opposite in capitals -- "Work is NOT complete until
# `git push` succeeds", "NEVER stop before pushing". Four commits of DLL code reached the remote on
# the strength of a green local gate, which compiles for MSVC and runs pure-logic tests under wine
# and cannot see a single thing that makes this repo hard.
#
# The two rules are not in conflict once the order is written down: FINISH the session by pushing,
# but run the thing first. This guard supplies the "first".
#
# WHAT IT GATES. Only a push whose commits touch `crates/` or `scripts/ds2-run.py` -- code that ends
# up inside the game's address space, or that decides what does. A push of policies, docs, or the
# beads export is not gated, because a guard that demanded a game launch before a Rego fix could be
# pushed would be routed around within the hour, and a routed-around guard is worse than none.
#
# WHAT SATISFIES IT. `ds2-loader.log` beside the game executable containing `ds2-loader: attach`
# -- written from `DllMain` by the DLL itself, after the game mapped it -- with a birth time (the run's
# start; mtime only where the filesystem has no birth time) at or after
# both HEAD's commit time and the staged DLL's, AND a staged DLL whose sha256 equals the one this
# checkout built. An agent cannot write that line by asserting it ran the game. A run older than the
# code proves nothing about the code, and a run of somebody else's binary proves nothing either.
#
# WHAT IT DOES NOT CLAIM. That the run exercised the feature under review. A filesystem signal cannot
# know which row was pressed. It proves the game launched with this code and the DLL loaded, which is
# the failure mode that actually shipped here: crates that were green and had never been in a
# process.
#
# FAIL CLOSED. No signal, an unreadable shell wrapper, or a signal that cannot parse means denied,
# because every one of those is indistinguishable from "the game was never run".
#
# WHY THERE IS NO DOCUMENTATION EXEMPTION, asked for 2026-09-24 after this guard cost three game
# restarts in one session and measured before it was declined. Of the last 120 commits, 101 touch
# `crates/` or the launcher and 3 of those change nothing but comment and blank lines -- and the
# jurisdiction test runs over `origin/main...HEAD`, not per commit, so a branch qualifies only if
# its ENTIRE game-code delta is comments. The branch that prompted the request carries 2255
# non-comment changed lines. There are also no `.md` files under `crates/` for a file-extension
# exemption to reach; `docs/` is already outside the gate.
#
# None of the three restarts would have been saved by it either. Each carried a real change to the
# DLL -- twice a log format string, which alters the bytes that ship and the output a run produces,
# and is exactly what `dll_match` exists to notice. The guard was right all three times.
#
# What DID cost those restarts is the order the work was done in, so the denial now says so. The
# freshness floor is `max(HEAD commit time, staged DLL mtime)`, which means a run taken before the
# commit can never clear it: commit, then build, then launch, and one launch is enough. Loosening
# jurisdiction would have bought a 3% case at the price of the property that makes this guard worth
# having, when the actual fix is free and is a sentence in the reason string.
deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"
	any_executed_push
	gated

	decision := {
		"rule_id": "DS2-MODS-REQUIRE-RUNTIME-BEFORE-PUSH",
		# `concat`, not `sprintf`: sprintf has no WASM probe recipe, and an unprobed builtin returns
		# UNDEFINED in cupcake's runtime, which makes the whole rule silently never fire.
		# scripts/check-cupcake-wasm-builtins.py caught this one before it shipped inert.
		"reason": concat(" ", [
			"This push carries game code that has not been run.",
			why,
			"Launch it with `scripts/ds2-run.py`, confirm `ds2-loader: attach` in the game's `ds2-loader.log`, then push.",
			"Commit before you launch, not after: the freshness floor is the newer of HEAD's commit time and the staged DLL's mtime, so a run taken before the commit never clears it and costs a second launch.",
		]),
		"severity": "HIGH",
	}
}

# A wrapper whose payload this guard cannot read, naming git and push. Same jurisdiction-scoped
# fail-closed as the other two git guards, `gated` included: an unreadable payload is refused on the
# same evidence a readable push is refused on, and permitted on the same evidence. Without `gated`
# here this arm denied `bash -c $CMD` on a policies-only branch and after a clean runtime run alike
# -- a guard answering a question nobody asked, which is how a guard earns the reputation that gets
# it switched off. DS2-MODS-REQUIRE-FRESH-ORIGIN-MAIN scopes its own unreadable-payload arm with
# `not fresh` for exactly this reason.
deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"
	commands.unparsed_shell_payload(input.tool_input.command)
	lowered := lower(input.tool_input.command)
	contains(lowered, "git")
	contains(lowered, "push")
	gated

	decision := {
		"rule_id": "DS2-MODS-REQUIRE-RUNTIME-BEFORE-PUSH",
		"reason": "This command wraps a shell payload the guard cannot read while naming git and push, so it cannot be shown not to push unrun game code. Run the git command directly.",
		"severity": "HIGH",
	}
}

executed_texts := commands.executed_texts(input.tool_input.command)

any_executed_push if {
	some text in executed_texts
	regex.match(git_push_command_pattern, lower(text))
}

# The same invocation pattern the main-push guard uses, including `git -C <path> push` and the
# global options that may sit before the verb. `git` may be spelled by path (`/usr/bin/git`): that
# spelling let a never-run crates/ push through on 2026-09-25, when it was used to get past the
# rewrite of bare `git` that worktree isolation refuses.
git_push_command_pattern := `(^|[;&|(
])\s*(command\s+)?(?:[^\s;&|()"']*/)?git([ \t]+((-c|--git-dir|--work-tree|--namespace|--config-env)(=|[ \t]+)("[^"\n]*"|'[^'\n]*'|[^ \t;&|()\n]+)|--(bare|no-pager|paginate|literal-pathspecs|no-replace-objects|exec-path)(=("[^"\n]*"|'[^'\n]*'|[^ \t;&|()\n]+))?))*[ \t]+push([ \t;&|)\n]|$)`

# --- The decision -------------------------------------------------------------

# Game code, and no proof it has ever been in a process. A push carrying only policies, docs or the
# beads export never reaches here, because `game_code` is 0 and this body fails at its first line.
gated if {
	field("game_code") == "1"
	not runtime_proven
}

# No readable signal at all. Indistinguishable from never having run the game.
gated if {
	not signal_readable
}

# THREE INDEPENDENT FACTS, and the third was added because the first two are satisfiable by a run
# that never touched this code. `attached` says a DLL loaded. `fresh` says the log postdates both
# HEAD's commit and the staged binary. `dll_match` says the binary that was staged is byte-identical
# to the one this checkout built -- which is the only one of the three that a parallel worktree's
# launch, or a plain Steam launch of a stale DLL, cannot satisfy on your behalf. The measurement that
# forced it is in the signal's header: this guard's verdict on an unchanged branch flipped from deny
# to allow within the hour because another session launched the game.
runtime_proven if {
	field("attached") == "1"
	field("fresh") == "1"
	field("dll_match") == "1"
	field("pending") != "1"
	field("pushdir") != "0"
}

# The command moves somewhere before pushing and the signal could not resolve where (ds2-mods-rs-lgor):
# `cd "$VAR"`, `cd -`, `popd`, pushes from two directories, or a target that is not a work tree. The
# checkout being pushed is unknown, so neither its game code nor its run can be measured. Gated on
# its own, not through `game_code`, so a signal that reported this and a wrong `game_code=0` with it
# still refuses. An absent field -- the format before it existed -- is not "0".
gated if {
	signal_readable
	field("pushdir") == "0"
}

# A commit chained in front of the push, in one command. Checked first because no run can satisfy
# it: the hook fires before the commit exists, so there is nothing yet a run could have tested.
# Measured 2026-09-25 -- `git commit -qam ... && git push -u origin launcher-drop-flag` went out
# carrying scripts/ds2-run.py, because at hook time HEAD was still origin/main and the diff the
# signal took was empty. The signal now reads the command (scripts/cupcake_push_scope.py).
why := "This command changes directory before pushing (a `cd`, `pushd` or `git -C`), and the guard could not resolve that directory to a checkout, so it cannot tell which checkout's code or run to judge. Use a literal absolute path, or `cd` there in its own command first." if {
	signal_readable
	field("pushdir") == "0"
} else := "This command commits and pushes in one go, so the commit being pushed does not exist yet and no run can have tested it. Commit in its own command, launch that commit, then push." if {
	signal_readable
	field("pending") == "1"
} else :="The game's log has no `ds2-loader: attach` line, so the DLL has never been loaded." if {
	signal_readable
	field("attached") != "1"
} else := "The DLL staged in the game directory is not the one this checkout built, so whatever ran was not this code." if {
	signal_readable
	field("dll_match") != "1"
} else := "The game's log predates HEAD or the staged DLL, so the last run does not cover this code." if {
	signal_readable
	field("fresh") != "1"
} else := "No runtime evidence could be read."

# --- Reading the signal -------------------------------------------------------

signal_text := out if {
	out := input.signals.runtime_evidence_for_head
	is_string(out)
} else := out if {
	out := input.signals.runtime_evidence_for_head.output
} else := ""

# PRESENT IS NOT READABLE, and the difference was a live hole. `signal_present` alone -- a
# `RUNTIME|` prefix test -- let `RUNTIME|game_code=|attached=|fresh=` through as an ALLOW: the
# game-code arm did not fire because `game_code` read "", and the missing-signal arm did not fire
# because the prefix was there. Fail-closed means unreadable and absent reach the same verdict, so
# the decision requires the one field that decides jurisdiction to be one of the two values the
# signal can emit. The other three fields need no such test: `runtime_proven` demands "1" from each
# of `attached`, `fresh` and `dll_match`, so any other content -- empty, garbage, or a field the
# signal no longer emits -- already fails them.
signal_present if {
	startswith(trim_space(signal_text), "RUNTIME|")
}

signal_readable if {
	signal_present
	game_code_known
}

game_code_known if {
	field("game_code") == "0"
}

game_code_known if {
	field("game_code") == "1"
}

# `RUNTIME|game_code=1|attached=0|...` -> the value for one key, or "" when absent.
field(key) := value if {
	some part in split(trim_space(signal_text), "|")
	startswith(part, concat("", [key, "="]))
	value := trim_prefix(part, concat("", [key, "="]))
} else := ""
