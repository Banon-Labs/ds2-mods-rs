# METADATA
# scope: package
# title: Guard Layer Destructive Operations
# authors: ["er-mods-rs agents", "ds2-mods-rs agents"]
# custom:
#   severity: HIGH
#   id: CLAUDE-GUARD-LAYER-DESTRUCTIVE
#   routing:
#     required_events: ["PreToolUse"]
#     required_tools: ["Bash"]

# WHY THIS EXISTS, AND WHY IT IS NOT A LOCKDOWN (2026-08-31).
#
# `.cupcake/rulebook.yml` claimed a TOTAL LOCKDOWN of `.cupcake/` and
# `.git/hooks/` through the `rulebook_security_guardrails` builtin. That claim
# was false for the whole life of the file: cupcake 0.5.2 treats a builtin as
# DISABLED unless its config block carries `enabled: true`, and that block never
# had one, so the engine logged
#
#     Skipping disabled builtin policy: .../rulebook_security_guardrails.rego
#
# and the policy was never compiled, never routed and never evaluated. Measured
# by adding that single line to a copy of `.cupcake/`: the same Write event that
# is ALLOWED against the tree is DENIED against the copy.
#
# ENABLING IT WAS THE WRONG FIX, and the reasoning matters more than the verdict:
#
#   * it halts on Read/Grep/Glob as well as Write, so the policy layer becomes
#     UNAUDITABLE. Every guard repair made in this repo began by reading these
#     files; a rule that forbids reading the rules cannot be reviewed, only
#     obeyed.
#   * its Bash arm is `contains(lower(command), ".cupcake")` -- a raw substring
#     test over the whole command. It denies `opa test .cupcake/`, every
#     `git commit -- .cupcake/...`, and every `python3 -c` inspection one-liner,
#     while `.cup"cake"`, `.cup*` or `$(printf .cupcake)` walk straight past it.
#     It stops maintenance, not tampering.
#   * its Task arm denies any subagent prompt containing `.cupcake`, so the work
#     could not even be delegated.
#   * `.git/hooks/` is not the live hook directory here: `core.hooksPath` is
#     `scripts/hooks`, so the lockdown's second protected path guards a directory
#     git does not consult.
#   * structurally, a policy file stored IN the repo cannot defend against an
#     agent editing the repo. What actually defends these files is the 622-case
#     `opa test .cupcake/` gate, the delivered-shape gate, and the diff a human
#     reads. This rule adds one thing those cannot: it stops a BLUNT INSTRUMENT
#     from removing the guard layer before anyone can review the change.
#
# SO THE SCOPE IS DELIBERATELY NARROW: destructive SHELL operations against the
# guard layer are denied; reading, grepping, editing and testing it are allowed,
# on purpose, and Edit/Write are not routed here at all. A change to a policy
# should arrive as a diff someone can read -- not as a deletion.
#
# WHAT THIS IS NOT. It is not a defence against a determined agent: `rm -rf .`
# from the repo root never names `.cupcake` and is not caught here (the
# protected-paths PARENT rule covers that shape only for ITS configured paths,
# and adding `.cupcake/` to that list would re-import every false positive listed
# above). Saying so plainly is the point -- a guard that overclaims its reach is
# the exact defect this file replaces.
package cupcake.policies.claude.guard_layer_destructive_guard

import data.cupcake.system.commands
import rego.v1

# The two paths the rulebook claimed to lock down. Spelled without a trailing
# slash: the operand matcher below supplies its own boundary, and a bare
# `rm -rf .cupcake` names the directory with no slash at all.
guard_layer_paths := {".cupcake", ".git/hooks"}

# Verbs that REMOVE or RELOCATE. `chmod`/`chown` are deliberately absent: making
# a new `.cupcake/signals/*.sh` executable is ordinary authoring, and the
# disarming form (`chmod -x`) cannot be told from it without parsing mode
# operands -- a speculative rule is worse than an honest gap.
#
# `mv` catches `mv .cupcake .cupcake.off`; `git mv` does NOT match, because `git`
# is not one of the wrapper words `has_command_verb` steps over, which leaves the
# tracked rename available as the supported way to rename a policy.
destructive_verbs := {"rm", "rmdir", "shred", "truncate", "mv"}

halt contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"

	# EVERY text this command executes, not just the outer one: `bash -c "rm -rf
	# .cupcake"` puts the verb inside a quoted operand where no `(^|\s)` anchor
	# can reach it. commands.executed_texts decomposes the wrapper payload into a
	# text of its own, and neutralises anchors inside quoted spans so that a
	# commit message or a bd memory QUOTING this command is not read as running
	# it.
	some text in commands.executed_texts(input.tool_input.command)

	# Per SEGMENT, so a destructive verb in one statement cannot borrow a guard
	# path operand from an unrelated one: `rm -f /tmp/x && opa test .cupcake/`
	# must stay allowed.
	some segment in commands.shell_segments(lower(text))

	# THE VERB IS READ FROM THE EXECUTABLE PART OF THE SEGMENT ONLY, with quoted
	# spans DELETED -- while the PATH is read from the whole segment, quotes and
	# all. The asymmetry is deliberate and each half was measured:
	#
	#   * verb from `quotes_removed`, because a quoted operand that is not a shell
	#     payload is DATA. commands.executed_texts neutralises `;&|()` and the
	#     quote characters inside such a span, but NOT `{` or `}` -- and `{` is in
	#     has_command_verb's anchor class. So
	#
	#         python3 -c "s={'rm'}; open('.cupcake/x.rego')"
	#
	#     arrived as `python3 -c "s={ rm }  open( .cupcake/x.rego )"`, the brace
	#     put `rm` in command position, and this rule DENIED an inspection
	#     one-liner about its own policy tree. Caught in vivo on 2026-08-31, by
	#     this rule firing on a command that was auditing it -- the precise
	#     failure this policy exists to avoid, since an agent that cannot inspect
	#     the guard layer cannot repair it. Deleting quoted spans takes the brace
	#     away with the rest of the python.
	#
	#   * path from the RAW segment, because quoting a path is ordinary:
	#     `rm -rf "$repo/.cupcake"` and `rm -rf '.cupcake'` must still be caught,
	#     and quotes_removed would take the operand away with the quotes.
	#
	# A quoted span that IS executed is not lost either way: executed_texts hands
	# back each shell payload as a text of its own, where the verb stands unquoted
	# at position 0.
	destructive_segment(commands.quotes_removed(segment))

	some path in guard_layer_paths
	segment_names_guard_path(segment, path)

	decision := {
		"rule_id": "CLAUDE-GUARD-LAYER-DESTRUCTIVE",
		"reason": concat("", [
			"Destructive shell operation on the guard layer (",
			path,
			") is not permitted. Reading, editing and testing the policy layer are ",
			"allowed -- change it with Edit/Write and record it with git, so the ",
			"change arrives as a reviewable diff rather than a deletion.",
		]),
		"severity": "HIGH",
	}
}

# COMMAND POSITION, never mere presence. The engine's whitespace_normalization
# welds a heredoc body onto the command that reads it, so a prose "mv" or a
# python `rm = 1` would otherwise look like a program. A word a shell actually
# executes is in command position by definition, so nothing that runs is lost.
destructive_segment(segment) if {
	some verb in destructive_verbs
	commands.has_command_verb(segment, verb)
}

# `find ... -delete` / `-exec rm {}` deletes without any verb from the set above.
destructive_segment(segment) if {
	commands.has_command_verb(segment, "find")
	regex.match(`(^|[[:space:]])-(delete|exec|execdir)([[:space:]]|$)`, segment)
}

# `git checkout -- .cupcake`, `git restore .cupcake`, `git clean -fdx .cupcake`:
# each DISCARDS guard work that is not yet committed, which is destruction by a
# different spelling. AGENTS.md already forbids checkout/restore/stash in prose;
# this is that instruction made executable for the guard layer specifically.
# The subcommand is matched anywhere in the segment rather than at a fixed
# offset so `git -C <dir> checkout` and `/usr/bin/git restore` are both covered.
destructive_segment(segment) if {
	commands.has_command_verb(segment, "git")
	regex.match(`(^|[[:space:]])(checkout|restore|clean)([[:space:]]|$)`, segment)
}

# Swapping the directory for a symlink is how a lockdown gets defeated upstream
# (TOB-EQTY-LAB-CUPCAKE-4); it is equally a way to make the real policies stop
# being read.
destructive_segment(segment) if {
	commands.creates_symlink(segment)
}

# The segmenter this rule used to define lives in commands.rego as of 2026-08-31
# (bd er-effects-rs-c0t9), because the vendored git_block_no_verify builtin needed
# the same machinery to close the same defect and two copies of a security
# predicate diverge. Its input contract -- feed it executed_texts output, never
# the raw command -- is documented there and is satisfied above.

# The segment names the guard path AS A PATH: at a token boundary, or after a
# `/` so an absolute or repo-relative spelling counts. A file merely ENDING in
# the same letters (`notes.cupcake`, `scripts/cupcake-hook.sh`) does not match,
# which is what keeps the hook shim itself removable.
segment_names_guard_path(segment, path) if {
	escaped := replace(path, ".", `\.`)
	pattern := concat("", [
		`(^|[[:space:]"'=:(/])`,
		escaped,
		`([[:space:]"')/:;,&*]|$)`,
	])
	regex.match(pattern, segment)
}
