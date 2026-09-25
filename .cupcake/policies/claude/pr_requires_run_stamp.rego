# METADATA
# scope: package
# title: A PR Carries a Run-Stamp for the Commit It Proposes
# authors: ["ds2-mods-rs agents"]
# custom:
#   severity: HIGH
#   id: DS2-MODS-PR-RUN-STAMP
#   routing:
#     required_events: ["PreToolUse"]
#     required_tools: ["Bash"]
#     required_signals: ["pr_run_stamp"]
package cupcake.policies.claude.pr_requires_run_stamp

import rego.v1

import data.cupcake.system.commands

# User directive 2026-09-25, their words: "a PR shouldn't be able to be drafted unless it has a
# timestamped/sha stamped or other metadata stamped line(s) related to the commit that is paired with
# both the drafting of the PR, and then the marking as ready".
#
# The line is
#
#     Run-Stamp: sha=<40 hex> at=<YYYY-MM-DDTHH:MM:SSZ> gate=<name> result=<pass|fail> [key=value ...]
#
# and `python3 scripts/pr-run-stamp.py` prints it. What it must say, per verb:
#
#   gh pr create  a stamp whose sha is the commit being proposed (HEAD, or `--head <branch>`), with a
#                 time at or after that commit's committer time. Either result: a draft may say its
#                 last run failed, which is information; it may not say nothing.
#   gh pr ready   the LIVE body, read with `gh pr view`, has a stamp whose sha is the PR's current
#                 headRefOid, fresh against that commit, and the latest such stamp is result=pass. A
#                 push after drafting moves headRefOid, so the draft's stamp no longer counts.
#
# The global draft guard refuses `gh pr ready` outright today; this arm is what stands if that
# changes. `gh pr ready --undo` is untouched.
#
# ALL DECIDING IS IN scripts/cupcake_run_stamp.py. Parsing a body with composed regexes is exactly
# what cupcake's WASM runtime silently drops (scripts/check-cupcake-wasm-builtins.py), so this file
# only maps the signal's verdict to a sentence, with literal patterns and `concat`.
#
# FAIL CLOSED: a command that names `gh pr create` or `gh pr ready` and a signal that is missing,
# unreadable or reports `verb=none` is refused. The signal wrapper cannot exit non-zero, so a crash in
# the Python module arrives here as silence, and silence must not read as a stamp.

deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"
	verb_in_signal
	field("ok") != "1"

	decision := {
		"rule_id": "DS2-MODS-PR-RUN-STAMP",
		"reason": concat(" ", [why_text, how_to_fix]),
		"severity": "HIGH",
	}
}

deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"
	names_gated_gh_verb
	not verb_in_signal

	decision := {
		"rule_id": "DS2-MODS-PR-RUN-STAMP",
		"reason": concat(" ", [
			"This command runs `gh pr create` or `gh pr ready`, and the Run-Stamp check could not read it, so it cannot be shown to carry a stamp for the commit.",
			"Run the gh command directly, with the body in a file passed by `--body-file`.",
			how_to_fix,
		]),
		"severity": "HIGH",
	}
}

verb_in_signal if field("verb") == "create"

verb_in_signal if field("verb") == "ready"

names_gated_gh_verb if {
	some text in commands.executed_texts(input.tool_input.command)
	regex.match(`(^|[;&|(\n])[ \t]*([a-z_][a-z0-9_]*=[^ \t]*[ \t]+)*((command|env|exec|time)[ \t]+)*([^ \t;&|()]*/)?gh[ \t]+pr[ \t]+create([ \t;&|)]|$)`, lower(text))
}

names_gated_gh_verb if {
	some text in commands.executed_texts(input.tool_input.command)
	regex.match(`(^|[;&|(\n])[ \t]*([a-z_][a-z0-9_]*=[^ \t]*[ \t]+)*((command|env|exec|time)[ \t]+)*([^ \t;&|()]*/)?gh[ \t]+pr[ \t]+ready([ \t;&|)]|$)`, lower(text))
	not contains(lower(text), "--undo")
}

how_to_fix := "Print the line with `python3 scripts/pr-run-stamp.py --from-last-check` (after `scripts/check.sh` ran on this commit) or `python3 scripts/pr-run-stamp.py --gate <name> --result <pass|fail>` for another run, e.g. `--gate runtime`, and put it in the PR body footer. Format: `Run-Stamp: sha=<40hex> at=<YYYY-MM-DDTHH:MM:SSZ> gate=<name> result=<pass|fail>`. To refresh a live PR: `gh pr edit <n> --body-file <file>`."

why_text := "The PR body has no `Run-Stamp:` line." if {
	field("why") == "missing"
} else := concat("", ["The PR body's Run-Stamp names a different commit; the PR's head is ", field("sha"), ". A stamp describes the run of one commit, and this is not it."]) if {
	field("why") == "wrong-sha"
} else := "The PR body's Run-Stamp for this commit is older than the commit itself, so the run it describes cannot have run this commit. Run the gate again after committing." if {
	field("why") == "stale"
} else := "The PR body's Run-Stamp is dated in the future. Stamps are printed by scripts/pr-run-stamp.py, not typed." if {
	field("why") == "future"
} else := "The latest Run-Stamp for this PR's head commit says result=fail. A PR is marked ready on a passing run of the commit it carries." if {
	field("why") == "not-pass"
} else := "The commit this PR would propose could not be resolved (no HEAD in the invoking checkout, or `--head` names no branch)." if {
	field("why") == "no-head"
} else := "The PR could not be read with `gh pr view`, so its live body and head commit are unknown." if {
	field("why") == "no-pr"
} else := "The PR's head commit time could not be read from `gh pr view`, so a stamp's freshness cannot be judged." if {
	field("why") == "no-commit-time"
} else := "The Run-Stamp check returned no verdict."

# --- Reading the signal -------------------------------------------------------

signal_text := out if {
	out := input.signals.pr_run_stamp
	is_string(out)
} else := out if {
	out := input.signals.pr_run_stamp.output
	is_string(out)
} else := ""

# `RUNSTAMP|verb=create|ok=0|why=missing` -> the value for one key, or "" when absent.
field(key) := value if {
	startswith(trim_space(signal_text), "RUNSTAMP|")
	some part in split(trim_space(signal_text), "|")
	startswith(part, concat("", [key, "="]))
	value := trim_prefix(part, concat("", [key, "="]))
} else := ""
