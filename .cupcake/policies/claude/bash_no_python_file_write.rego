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
#     A program python reads from STDIN is treated as a script-file invocation
#     with the path hidden (`cat patch.py | python3 -`) UNLESS its text is on the
#     command line, which `echo ...` and `printf ...` put there and a file does
#     not.
#   routing:
#     required_events: ["PreToolUse"]
#     required_tools: ["Bash"]
#     required_signals: ["repo_paths"]
package cupcake.policies.claude.bash_no_python_file_write

import data.cupcake.system.commands
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
# A PROGRAM ARRIVING ON STDIN (2026-09-24, bd ds2-mods-rs-9m8, gap 1)
#
# `cat /tmp/patch.py | python3 -` is the two-call bypass the 2026-09-17 work
# closed for `python3 /tmp/patch.py`, walking in through the other door.
# Measured through scripts/cupcake-check-command.py against the live WASM
# engine: ALLOWED. Neither rule above could see it. It is not a `.py` operand
# (the path belongs to `cat`, not to python), and it carries no write text on
# the command line because the writing lives inside the piped file. The
# `python3 -` disqualifier only stopped such a command from BORROWING the
# committed-script exemption; it never denied one.
#
# THE PROPERTY BEING DISTRUSTED IS UNREADABLE PROGRAM TEXT, which is the same
# property the script-file rule distrusts -- not the pipe, and not the `-`. That
# distinction is the whole design of this arm, because the two spellings sit one
# character apart:
#
#     cat /tmp/patch.py | python3 -      the program is in a FILE     -> DENY
#     echo 'print(1)'   | python3 -      the program is RIGHT THERE   -> judged
#                                        by the inline write rules, as before
#
# Denying every `| python3 -` would have been three characters of policy and a
# behaviour change the issue warned about: a one-off `echo ... | python3 -`
# filter is an inline program like any other, fully visible in the transcript,
# and the write patterns at the top of this file already judge it correctly --
# test_absolute_committed_script_plus_stdin_program_is_denied has pinned exactly
# that since 2026-09-23. A guard that blanket-denies the visible form buys
# nothing and costs the work it already permits.
#
# SO THE TEST IS ON THE SOURCE, not on python: a stdin-program invocation denies
# when what feeds it is NOT text on this command line. `echo` and `printf` put it
# there; a file does not. The allow-list is short on purpose -- the opposite
# shape, a block-list of opaque producers (`cat`, `curl`, `base64 -d`, `xzcat`,
# ...), fails OPEN on the first one nobody thought of, and this guard exists
# because of a bypass nobody thought of.
#
# PER STATEMENT, not per command, and that is load-bearing:
# `cat /tmp/patch.py | python3 -; echo done` would satisfy a whole-command
# "is there an echo" test while the scratch file runs regardless. Statement
# granularity (pipelines kept WHOLE, because a pipeline is exactly how the
# program reaches python) is what makes the question "what feeds THIS python".
#
# READ FROM executed_unquoted_texts, so quoted spans are gone before any of this
# runs. That is not tidiness either: a commit message QUOTING one of these
# commands -- `git commit -m "... python3 - < /tmp/patch.py ..."` -- would
# otherwise be denied by the rule it documents, which is a false positive this
# repo has already paid for twice (see the heredoc-body divergence note in
# .cupcake/system/commands.rego). The cost is that `echo "$(cat f.py)" |
# python3 -` reads as an `echo` and is allowed; see the RESIDUE note below.
# ---------------------------------------------------------------------------

python_file_write_detected if {
	opaque_stdin_program
}

opaque_stdin_program if {
	some text in commands.executed_unquoted_texts(command)
	some statement in commands.shell_statements(text)
	regex.match(python_stdin_program_pattern, statement)
	stdin_source_is_unreadable(statement)
}

# A producer stage in the same pipeline that is not putting text on the command
# line. The python stage itself is skipped -- it is the CONSUMER, and failing to
# skip it would make every `echo ... | python3 -` opaque by its own presence.
stdin_source_is_unreadable(statement) if {
	some segment in commands.shell_segments(statement)
	not regex.match(python_token_pattern, segment)
	not emits_command_line_text(segment)
}

# `python3 - < /tmp/patch.py`, which needs no pipeline at all: one stage, and the
# program still arrives from a file. `<<` is excluded on BOTH sides because a
# heredoc body IS command-line text -- normally commands.quotes_removed has
# already folded it into a quoted span and erased the marker, but when a second
# heredoc in the same command defeats that resolution the raw `<<` survives and
# must not be read as an input redirect.
stdin_source_is_unreadable(statement) if {
	regex.match(`(^|[^<])<($|[^<])`, statement)
}

emits_command_line_text(segment) if {
	some verb in ["echo", "printf"]
	commands.has_command_verb(segment, verb)
}

# The DENY-side detector for a stdin program, and deliberately NOT the same
# pattern as the exemption disqualifier below. The two point in opposite
# directions: widening this one denies MORE, widening the disqualifier denies
# LESS. So the leading interpreter flags are admitted here -- `cat f.py |
# python3 -u -` is the same bypass wearing a flag -- and the disqualifier is left
# exactly as it was, because widening it would hand `python3 /tmp/patch.py &&
# python3 -u -` a way out of the script-file rule.
python_stdin_program_pattern := concat("", [
	`python[0-9.]*`,
	leading_interpreter_flags,
	`[[:space:]]+-($|[[:space:]])`,
])

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
# ---------------------------------------------------------------------------
# LEADING INTERPRETER FLAGS (2026-09-24, bd ds2-mods-rs-9m8, gap 2)
#
# Until this date the path was read as the FIRST operand after the interpreter,
# so ONE flag made the whole invocation invisible. Measured through
# scripts/cupcake-check-command.py against the live WASM engine:
# `python3 -u /tmp/patch.py` ALLOWED, the bare `python3 /tmp/patch.py` DENIED --
# the same scratch file, the same write, one `-u` apart. `-u` is not an exotic
# spelling either; it is what an agent reaches for the moment it wants a
# script's output unbuffered in a captured pipe.
#
# THE NAIVE FIX IS WORSE THAN THE GAP. "Skip anything starting with `-`" reads
# `python3 -m pytest tests/x.py` as a script-file invocation of `tests/x.py` and
# DENIES an ordinary test run -- a guard blocking the work it has no business
# touching, which is the failure mode this file has already paid for twice (the
# `errors='replace'` read and the absolute committed-script launch). `-m` names a
# MODULE: python resolves it on sys.path and every remaining token is the
# module's own argv, so no `.py` operand after `-m` is a script python executes.
#
# So the flag run is an ALLOW-LIST of flags that do NOT consume the program, and
# `-m` terminates the scan by simply not being on it -- in clustered form too
# (`-um` matches `-u` and then strands `m`, which no alternative accepts, so the
# scan dies exactly where it should). `-c` is off the list for the same reason:
# it IS the program. `-W` and `-X` take a value, so they consume the next token
# as well or that value would be read as the script path.
interpreter_flag_pattern := `(?:-[bBdEiIOPqRsSuvVx]+|--(?:[A-Za-z0-9][A-Za-z0-9=._-]*)?|-[WX][[:space:]]*[^[:space:]]+)`

# Zero or more of them, as a NON-capturing group: script_paths reads submatch 1
# and a capturing group here would silently shift the path out from under it.
leading_interpreter_flags := concat("", ["(?:[[:space:]]+", interpreter_flag_pattern, ")*"])

script_file_pattern := concat("", [
	`python[0-9.]*`,
	leading_interpreter_flags,
	`[[:space:]]+([^[:space:]-][^[:space:]]*\.py)($|[^[:alnum:]_.-])`,
])

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
	not regex.match(python_adjacent_inline_program_pattern, command)
	not regex.match(python_token_pattern_followed_by_stdin, command)
	count(script_paths) > 0
}

# A PYTHON-ADJACENT `-c` (2026-09-24, bd ds2-mods-rs-9m8, gap 3).
#
# The disqualifier above used to read `(^|[[:space:]])-c($|[[:space:]])` -- "does
# a `-c` token appear ANYWHERE in this command" -- and a disqualifier that
# matches more DENIES LESS, because its only job is to take a command out of the
# script-file rule. So every `-c` in the line, including one belonging to some
# OTHER program, was a way out of the rule. Measured through
# scripts/cupcake-check-command.py against the live engine:
# `python3 /tmp/patch.py -c fixtures.json` ALLOWED, while the same command
# without its trailing `-c fixtures.json` DENIED. That is fail-OPEN, and the
# token doing it was the SCRIPT'S OWN flag -- one an agent can append to any
# invocation, truthfully, and be waved through.
#
# The narrow question is the one the disqualifier always meant to ask: is python
# being handed an INLINE program? That is `-c` in python's own flag position --
# directly after the interpreter, behind any of the leading flags above, and
# clustered forms (`-uc`, `-sc`) count because the shell hands them to python as
# one token. `bash -c "..."`, `grep -c`, and a script's own `-c` are none of
# python's business and no longer speak for it.
#
# This is STRICTLY TIGHTER -- it removes escape routes and adds none -- so the
# only thing it can break is a command that was relying on the hole. One shape
# does change and is meant to: `bash -c "python3 /tmp/patch.py"` used to be
# disqualified by the wrapper's own `-c` and is now judged on the python
# invocation inside it, which is the invocation that actually runs.
python_adjacent_inline_program_pattern := concat("", [
	`python[0-9.]*`,
	leading_interpreter_flags,
	`[[:space:]]+-[A-Za-z]*c($|[^A-Za-z0-9])`,
])

# CLOSED 2026-09-24 (bd ds2-mods-rs-9m8). The three gaps that stood here -- all
# about WHICH INVOCATIONS ARE SEEN rather than about which paths are exempt --
# are the stdin arm above, the leading-flag run, and the python-adjacent `-c`.
# Each was measured ALLOWING through the live WASM engine before the change and
# DENYING after, and each was pinned by a test asserting the allowing behaviour,
# so all three tests flipped to deny in the same commit instead of quietly
# starting to pass. The exemption is untouched: every disqualifier still holds,
# so no inline program borrows it, which is the property the issue required to
# survive.
#
# RESIDUE that remains, none of it pinned to a deny today:
#
#   * `echo "$(cat /tmp/patch.py)" | python3 -`. The substitution is inside a
#     quoted span, so executed_unquoted_texts deletes it and the stage reads as a
#     plain `echo`. Closing it means refusing a visible source whenever the
#     statement carries `$(` or a backtick, which also refuses honest one-offs
#     like `echo "$(date)" | python3 -`. Left open deliberately: the alternative
#     trades a contrived bypass for a false positive on ordinary work, and this
#     file has been wrong in that direction twice already.
#   * `cat /tmp/patch.py | python3` -- no `-` operand at all. Python reads stdin
#     as its program when it is handed neither a script nor `-c`, and detecting
#     "python with no program operand" is a different question from the one the
#     patterns here ask. Not reachable by accident; reachable on purpose.
#   * `cat <<'PY' ... PY | python3 -` denies, although the body IS on the command
#     line. The heredoc-reading stage is a `cat`, and `cat` cannot join the
#     text-emitting allow-list without also admitting `cat <file>`, which is gap
#     1 itself. Accepted as an over-denial: the happy path (`python3 - <<'PY'`,
#     the interpreter reading its own heredoc) is unaffected.

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

# The signal's lines 3 and on: this repository's other git worktrees, from git's own record.
repo_roots contains root if {
	some index, line in signal_lines
	index >= 2
	root := absolute_dir(line)
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

# The EXEMPTION disqualifier for a stdin program, unchanged on purpose: see
# python_stdin_program_pattern above for why the deny-side detector may be wider
# than this one and this one may not be widened to match it.
python_token_pattern_followed_by_stdin := "python[0-9.]*[[:space:]]+-($|[[:space:]])"

block_reason := "🧁 Cupcake blocked a python file write from Bash. Editing a file by running a python program hides the change: it never shows up as a reviewable diff, a mismatched `replace` anchor silently no-ops, and composing the program costs a turn that the edit itself does not. Use the Edit tool to change an existing file (it fails loudly when the anchor does not match) and the Write tool to create one. Reading files in python is untouched, and so is shell redirection -- `cmd > file` and a plain heredoc into a file are visible in the command itself. A committed script under this repo's `scripts/` tree is allowed BY ANY SPELLING THAT RESOLVES THERE -- `scripts/<name>.py`, `./scripts/<name>.py`, `$CLAUDE_PROJECT_DIR/scripts/<name>.py`, the absolute path, or `~/<path-to-repo>/scripts/<name>.py` -- so an absolute launch path is not what is being refused here. A script OUTSIDE that tree is not exempt however closely its path resembles one (`/tmp/.../scripts/x.py` included), every `.py` path in the command must be inside it, and an inline program (`-c`, `<<HEREDOC`, `python3 -`) is never exempt. A program python reads from STDIN is refused when its text is not on the command line -- `cat file.py | python3 -` and `python3 - < file.py` are a script-file invocation with the path hidden -- while `echo ... | python3 -` and `printf ... | python3 -` are judged as the inline programs they are, because you can read them here. User directive 2026-09-16: \"We NEED a hook to stop you from using python to write massive files.\""

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
