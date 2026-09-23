# METADATA
# scope: package
# title: Block Python File Writes From Bash (use Edit/Write instead)
# authors: ["er-mods-rs agents", "ds2-mods-rs agents"]
# custom:
#   severity: HIGH
#   id: DS2-MODS-BASH-NO-PYTHON-FILE-WRITE
#   description: >-
#     Hard block on editing files by running a python program from the Bash tool.
#     The shape this exists to stop is a long `python3 - <<'PY' ... open(path,
#     'w').write(s) ... PY` heredoc that rewrites a source file: the edit never
#     appears as a reviewable diff in the transcript, a single bad `replace`
#     silently no-ops or corrupts the file, and one call can burn many minutes
#     composing a program whose only job was an edit the Edit tool makes in one
#     step. User directive 2026-09-16, after ~25 minutes of one file being
#     rewritten this way: "We NEED a hook to stop you from using python to write
#     massive files."
#     Use the Edit tool for a change to an existing file and the Write tool for a
#     new one. Both are diffed, both fail loudly when the anchor does not match,
#     and neither costs a turn to compose.
#     Shell redirection is NOT blocked -- `cmd > file` and a plain heredoc into a
#     file are visible in the command itself. Only python doing the writing is.
#     A committed script under the repo's own `scripts/` tree is exempt by ANY
#     spelling that resolves there -- repo-relative, absolute,
#     `$CLAUDE_PROJECT_DIR`-prefixed, `~`-prefixed -- which is what the repo_paths
#     signal is for. An inline program is not exempt in any spelling.
#   routing:
#     required_events: ["PreToolUse"]
#     required_tools: ["Bash"]
#     required_signals: ["repo_paths"]
package cupcake.policies.claude.bash_no_python_file_write

import rego.v1

command := object.get(input.tool_input, "command", "")

# A python interpreter as a command token: at command start or after a shell
# separator, optionally path-prefixed (`/usr/bin/python3`, `./python`), and
# terminated by a non-identifier char. `uv run ... python3` is caught by the
# same token match because `python3` follows whitespace there too.
python_token_pattern := "(^|[[:space:];|&('\"`])/?([[:alnum:]_.-]+/)*python[0-9.]*($|[^[:alnum:]_])"

invokes_python if {
	regex.match(python_token_pattern, command)
}

# The write itself. Each pattern names a way python mutates a file on disk.
#
# `open(..., 'w')` and friends are matched on the MODE argument rather than on
# `open(` alone, so a read (`open(path)`, `open(path, 'rb')`) is untouched --
# reading a file to answer a question is the thing this guard wants to stay
# cheap. `pathlib.Path.write_text` / `write_bytes` carry the mode in the method
# name, and `shutil.copy`/`move` write without ever naming a mode.
# A python file MODE, and nothing else that happens to be a quoted string.
#
# The mode alphabet is exactly `rwxab+t`, and a real mode is at most four of them.
# Pinning both is what keeps ordinary keyword arguments out: `errors='replace'`
# begins with `r`, and a looser `['"][rwxa][a-z+]*['"]` reads it as the mode `r`
# plus `eplace` and denies a pure read. Measured 2026-09-16, one command after this
# guard landed -- `open(p, encoding='utf8', errors='replace').read()` was refused,
# which is a guard blocking the exact work it exempts.
write_mode_pattern := `open\([^)]*['"][wxa][rwxab+t]{0,3}['"]`

# `r+`, and the two spellings that put the binary flag on either side of it
# (`rb+`, `r+b`). Update mode reads as well as writes, so it belongs here.
read_plus_pattern := `open\([^)]*['"]r[bt]*\+[bt]*['"]`

write_pattern contains pattern if {
	some pattern in [
		write_mode_pattern,
		read_plus_pattern,
		`open\([^)]*mode[[:space:]]*=[[:space:]]*['"][wxa][rwxab+t]{0,3}['"]`,
		`\.write_text\(`,
		`\.write_bytes\(`,
		`\.writelines\(`,
		`shutil\.(copy|copy2|copyfile|move)\(`,
		`os\.(remove|unlink|rename|replace|truncate)\(`,
	]
}

writes_a_file if {
	some pattern in write_pattern
	regex.match(pattern, command)
}

python_file_write_detected if {
	invokes_python
	writes_a_file
	not runs_a_committed_script
}

# A `.py` FILE invocation whose path is not a committed, reviewed script
# carries no `open(..., 'w')` text of its own on THIS command line -- that
# text lives inside the file being run, not in the command that runs it -- so
# `writes_a_file` alone can never see it. Guard gap measured 2026-09-17: a
# throwaway file written to the session scratchpad with a `cat > ... <<'EOF'`
# heredoc (allowed on purpose -- shell redirection stays visible in the
# command) and then run as a SEPARATE `python3 /tmp/.../patch.py` call sailed
# through, because that second call is a one-line script invocation with no
# inline write text to match. Cupcake evaluates each Bash call independently,
# so the only way to catch call two is to distrust script-file execution
# itself unless the path is one this repo has already reviewed and committed.
python_file_write_detected if {
	invokes_python
	runs_a_python_script_file
	not runs_a_committed_script
}

# ---------------------------------------------------------------------------
# Committed-script exemption.
#
# `python3 scripts/<name>.py ...` runs a reviewed file that lives in the repo.
# Whatever it writes was written once, reviewed once, and is re-runnable -- the
# opposite of an inline program composed for one edit. The exemption is for the
# INVOCATION only: the command must not also carry an inline program, so a
# `python3 scripts/foo.py` followed by a `python3 -c '...open(p,"w")...'` in the
# same command stays denied.
#
# Fail-closed shape: a heredoc (`<<`), a `-c` inline program, or a `-` stdin
# program anywhere in the command disqualifies it, because those are exactly the
# forms that carry an inline program.
#
# SPELLING IS NOT IDENTITY (2026-09-23). Until this date the exemption matched
# one spelling, `(./)?scripts/<name>.py`, and denied every other way of naming
# the same file. That put the guard in direct contradiction with a standing
# instruction the user keeps in their global AGENTS.md, in the section about
# launch commands: "use ABSOLUTE paths in every launch command -- never
# `./script`, because cwd is not yours and resets between calls". Measured
# through scripts/cupcake-hook.sh that day, `python3
# /home/banon/projects/ds2-mods-rs/scripts/ds2-run.py --rows ...` was DENIED as
# a python file write -- a launch of a committed, reviewed script, refused for
# the only spelling the instructions permit, with a block message that named the
# relative spelling as the allowed one. An agent obeying the instructions hit a
# deny; an agent satisfying the guard disobeyed the instructions. A guard that
# can only be satisfied by breaking a rule is not enforcing anything; it is
# teaching agents that guards are obstacles.
#
# So the exemption is now about the FILE, not the spelling: every spelling that
# resolves to a file under this checkout's `scripts/` tree is exempt, and
# nothing else is. What that needs, and what a regex cannot supply, is the
# absolute path of the repo root -- see .cupcake/signals/repo_paths.sh. The
# tempting shortcut, "any path whose tail looks like `/scripts/<name>.py`", is
# exactly the bypass the 2026-09-17 rewrite closed from the other end: it hands
# the exemption to `/tmp/ds2-mods-rs/scripts/patch.py`, which one `mkdir -p`
# manufactures.
# ---------------------------------------------------------------------------

# EVERY python `.py` FILE invocation in the command, as the path token was
# spelled -- as opposed to inline code (`-c`), a heredoc, or stdin (`python3 -`),
# and committed or not. The capture group is the path; find_all is what makes the
# exemption test below able to speak about ALL of them rather than about whichever
# one a regex happened to hit first.
#
# The trailing context is "anything that is not a path character", NOT
# whitespace-or-end. A separator may abut the path with no space in front of
# it, and the narrower spelling missed every one of them. Measured 2026-09-17
# against the live policy, one command after the location fix landed:
# `python3 /tmp/.../patch.py` denied, `python3 /tmp/.../patch.py; echo
# "exit=$?"` ALLOWED -- the semicolon is not whitespace, so the path never
# matched and the script-file rule never fired. The `; echo "exit=$?"` suffix
# is not exotic either: another guard in this directory asks for a build's exit
# code to be read that way, so the bypass shape is one the harness encourages.
script_file_pattern := `python[0-9.]*[[:space:]]+([^[:space:]-][^[:space:]]*\.py)($|[^[:alnum:]_.-])`

# A leading quote is part of the SHELL's syntax, not of the path: `python3
# "$CLAUDE_PROJECT_DIR/scripts/x.py"` and `python3 'scripts/x.py'` name the same
# files as their bare spellings. Only the leading quote needs removing -- the
# capture group above ends at `.py`, so a trailing quote is already outside it.
# Stripping it cannot widen anything: a quoted path that is not committed still
# is not committed.
script_paths contains path if {
	some match in regex.find_all_string_submatch_n(script_file_pattern, command, -1)
	path := trim_left(match[1], "\"'")
}

runs_a_python_script_file if {
	not contains(command, "<<")
	not regex.match(`(^|[[:space:]])-c($|[[:space:]])`, command)
	not regex.match(python_token_pattern_followed_by_stdin, command)
	count(script_paths) > 0
}

# RESIDUE, all three older than the 2026-09-23 spelling work and all three left
# alone by it deliberately -- they are about WHICH INVOCATIONS ARE SEEN, not about
# which paths are exempt, and none of them is reachable through the exemption
# (every disqualifier above still holds, so no inline program can borrow it).
# Each is pinned by a test asserting today's behaviour, so closing one turns a
# test red on purpose rather than passing unnoticed:
#
#   * `cat /tmp/patch.py | python3 -` is not denied by either rule. The program is
#     neither a `.py` operand nor visible write text. bd ds2-mods-rs-9m8.
#   * `python3 -u /tmp/patch.py` is not seen: script_file_pattern takes the FIRST
#     operand after the interpreter, and a leading flag displaces it. Widening it
#     to skip flags would read `python3 -m pytest tests/x.py` as a script-file
#     invocation and deny an ordinary test run, so it needs a module-aware form
#     rather than a looser one. bd ds2-mods-rs-9m8.
#   * the `-c` test is command-WIDE, so appending ` -c x` to any script
#     invocation disqualifies the whole command from the script-file rule -- a
#     fail-OPEN direction. Narrowing it to a python-adjacent `-c` is a deny-side
#     behaviour change on its own merits. bd ds2-mods-rs-9m8.

# EVERY path must be committed, not merely one of them. The old rule asked
# `regex.match(committed_pattern, command)` -- "does this command name a
# committed script anywhere" -- which `python3 scripts/ok.py; python3
# /tmp/patch.py` satisfies while running the scratch file regardless. That hole
# predates this rewrite, and widening the accepted spellings is exactly what
# would have made it easy to walk through, so it closes here with it.
runs_a_committed_script if {
	runs_a_python_script_file
	not uncommitted_script_path
}

uncommitted_script_path if {
	some path in script_paths
	not committed_script_path(path)
}

# The repo-relative tail a committed script must have, anchored end to end
# because this now matches a single extracted token rather than hunting inside a
# whole command line: `scripts/<name>.py` or `scripts/<subdir>/<name>.py`.
#
# The `..` refusal is load-bearing and cannot be folded into the character class:
# `.` is a legal character in a file name, so `scripts/../../tmp/patch.py`
# satisfies the pattern and is an escape out of the tree. It is checked on the
# RESOLVED tail rather than on the whole command, which is the one deliberate
# relaxation here -- `python3 scripts/foo.py --out ../build/x` writes nothing
# this guard can see and is a committed script's own business.
committed_tail_pattern := `^scripts/[[:alnum:]_.-]+(/[[:alnum:]_.-]+)*\.py$`

committed_tail(tail) if {
	regex.match(committed_tail_pattern, tail)
	not contains(tail, "..")
}

# (a) Repo-relative -- the spelling this exemption has always accepted.
committed_script_path(path) if {
	committed_tail(trim_prefix(path, "./"))
}

# (b) `$CLAUDE_PROJECT_DIR` is the harness's OWN name for the repo root, so this
# spelling is exempt symbolically: no root lookup can disagree with it, and an
# agent cannot point it somewhere else from inside a Bash command.
committed_script_path(path) if {
	some prefix in ["$CLAUDE_PROJECT_DIR/", "${CLAUDE_PROJECT_DIR}/"]
	startswith(path, prefix)
	committed_tail(trim_prefix(path, prefix))
}

# (c) An absolute path under a known root. `startswith(path, root + "/")` and
# nothing looser: a path that merely ENDS in `/scripts/<name>.py` is not this
# repo's, which is what keeps `/tmp/.../patch.py`, `/home/banon/scripts/patch.py`
# and a lookalike `/tmp/ds2-mods-rs/scripts/patch.py` denied.
committed_script_path(path) if {
	some root in repo_roots
	prefix := concat("", [root, "/"])
	startswith(path, prefix)
	committed_tail(trim_prefix(path, prefix))
}

# (d) `~/...`, which is a spelling the shell resolves against $HOME before python
# ever sees it. Expanding it needs $HOME, which only the signal can supply; with
# no signal this arm is simply undefined and the `~` spelling denies, which is
# the safe direction. Without the expansion the policy cannot tell
# `~/projects/ds2-mods-rs/scripts/x.py` from `~/ds2-mods-rs/scripts/x.py`, and
# guessing from the tail is the bypass named at the top of this section.
committed_script_path(path) if {
	startswith(path, "~/")
	expanded := concat("", [signal_home_dir, "/", trim_prefix(path, "~/")])
	some root in repo_roots
	prefix := concat("", [root, "/"])
	startswith(expanded, prefix)
	committed_tail(trim_prefix(expanded, prefix))
}

# ---------------------------------------------------------------------------
# Where the repo root comes from.
#
# Two sources, both supplied by the harness or by the policy layer's own
# location, NEITHER of them readable or settable from the Bash command being
# judged -- which is the property that makes them safe to trust. A `cd /tmp` in
# the command changes neither.
#
#   repo_paths signal  the checkout that owns THESE policies, resolved from
#                      .cupcake/signals/'s own path. Authoritative: the scripts/
#                      tree being exempted and the policy set doing the exempting
#                      provably come from the same checkout.
#   input.cwd          the session's working directory, as the harness reports
#                      it. Already trusted this way by
#                      edit_no_tmp_scripts_guard's in_current_repo, and it keeps
#                      the absolute spelling working if the signal ever fails --
#                      the alternative being a silent return to the deny that
#                      started this.
#
# Both are absolute-path-checked before use, so a signal that emits an error
# message, an empty string or `/` contributes no root. An empty root set means
# only the repo-relative and `$CLAUDE_PROJECT_DIR` spellings are exempt, which is
# strictly narrower than today -- the failure direction a guard is allowed to
# have.
# ---------------------------------------------------------------------------

repo_roots contains root if {
	root := signal_repo_root
}

repo_roots contains root if {
	root := absolute_dir(object.get(input, "cwd", ""))
}

# A root usable as a path prefix: absolute, non-empty, not `/` (which would
# exempt every `/anything/scripts/*.py` on the filesystem), and with any trailing
# slash removed so `concat(root, "/")` cannot produce `//`.
absolute_dir(raw) := dir if {
	trimmed := trim_space(raw)
	startswith(trimmed, "/")
	dir := trim_right(trimmed, "/")
	dir != ""
}

# The signal's contract: line 1 is the repo root, line 2 is $HOME. Cupcake hands
# a signal's stdout through as either a bare string or `{output: "..."}`
# depending on version, so both shapes are read -- the same `else` chain the
# other signal-reading policies here use.
signal_text := value if {
	value := input.signals.repo_paths
	is_string(value)
} else := value if {
	value := input.signals.repo_paths.output
	is_string(value)
} else := ""

signal_lines := split(signal_text, "\n")

signal_repo_root := absolute_dir(signal_lines[0])

signal_home_dir := absolute_dir(signal_lines[1])

python_token_pattern_followed_by_stdin := "python[0-9.]*[[:space:]]+-($|[[:space:]])"

block_reason := "🧁 Cupcake blocked a python file write from Bash. Editing a file by running a python program hides the change: it never shows up as a reviewable diff, a mismatched `replace` anchor silently no-ops, and composing the program costs a turn that the edit itself does not. Use the Edit tool to change an existing file (it fails loudly when the anchor does not match) and the Write tool to create one. Reading files in python is untouched, and so is shell redirection -- `cmd > file` and a plain heredoc into a file are visible in the command itself. A committed script under this repo's `scripts/` tree is allowed BY ANY SPELLING THAT RESOLVES THERE -- `scripts/<name>.py`, `./scripts/<name>.py`, `$CLAUDE_PROJECT_DIR/scripts/<name>.py`, the absolute path, or `~/<path-to-repo>/scripts/<name>.py` -- so an absolute launch path is not what is being refused here. A script OUTSIDE that tree is not exempt however closely its path resembles one (`/tmp/.../scripts/x.py` included), every `.py` path in the command must be inside it, and an inline program (`-c`, `<<HEREDOC`, `python3 -`) is never exempt. User directive 2026-09-16: \"We NEED a hook to stop you from using python to write massive files.\""

deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"
	python_file_write_detected

	decision := {
		"rule_id": "DS2-MODS-BASH-NO-PYTHON-FILE-WRITE",
		"severity": "HIGH",
		"reason": concat("", [block_reason, "\n\nSource: ", command]),
	}
}
