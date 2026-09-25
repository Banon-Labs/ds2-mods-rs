# METADATA
# scope: package
# description: Helper functions for secure command analysis
package cupcake.system.commands

import rego.v1

# Check if command contains a specific verb with proper word boundary anchoring
# This prevents bypass via extra whitespace: "git  commit" or "  git commit"
has_verb(command, verb) if {
	pattern := concat("", ["(^|\\s)", verb, "(\\s|$)"])
	regex.match(pattern, command)
}

# Check if command contains ANY of the dangerous verbs from a set, in ONE combined
# regex.match call rather than one per verb (2026-09-16, bd
# cupcake-wasm-regex-cache-reuse-oob-2026-09-16): see the deny-root-delete-after-heredoc-terminator
# fix for the measured WASM regex-cache defect this collapse works around.
has_dangerous_verb(command, verb_set) if {
	pattern := concat("", ["(^|\\s)(", verb_alternation(verb_set), ")(\\s|$)"])
	regex.match(pattern, command)
}

has_any_command_verb(command, verb_set) if {
	pattern := concat("", [
		command_position_prefix_pattern, path_prefix_pattern,
		"(", verb_alternation(verb_set), ")",
		`([ \t\n;&|(){}]|$)`,
	])
	regex.match(pattern, command)
}

verb_alternation(verb_set) := concat("|", [v | some v in verb_set])

# ---------------------------------------------------------------------------
# COMMAND POSITION (2026-08-31, bd protected-paths-parent-heredoc-prose-verb)
#
# has_verb answers "does this word appear anywhere in the text", which is the
# right question for a flag or an operand and the WRONG one for a program name.
# A word only runs when it stands in COMMAND POSITION: at the start of the text,
# or after a shell separator, optionally behind wrapper words and assignments.
#
# The distinction is not academic. The cupcake engine's `whitespace_normalization`
# enrichment replaces EVERY newline in a Bash command with a space before any
# policy runs (measured 2026-08-31 from `cupcake eval --debug-files`), so a
# heredoc body arrives welded onto the command that reads it:
#
#     cat > src/lib.rs <<'EOF' /// Install the detour ... EOF
#
# Under has_verb the prose word "Install" in a Rust doc comment is a destructive
# `install(1)`, and the `///` is a path operand. Under has_command_verb it is what
# it looks like: a word in the middle of an argument. Nothing that actually runs
# is lost, because a program a shell executes is in command position BY
# DEFINITION -- that is what command position means.
#
# WRAPPERS ARE FOLLOWED, so tightening cannot lose a real invocation: `sudo rm`,
# `env FOO=1 rm`, `nohup rm`, `xargs -0 rm`, `TMPDIR=/x rm` and option tokens in
# between all still resolve to the verb behind them.
#
# NOT COVERED, deliberately, because has_verb does not cover it either and this
# is not the change that closes it: a verb inside a quoted wrapper payload
# (`bash -c "rm -rf /"`). has_verb's own `(^|\s)` anchor never matched `"rm`, so
# both before and after this rule that shape is invisible here; callers that need
# it should read commands.executed_texts, which decomposes the payload into a
# text of its own where the verb IS in position 0.
# A separator, then any run of things that stand BEFORE a program without being
# one: assignments, option tokens, and wrapper words.  The second alternative
# lets a wrapper carry up to four arbitrary tokens of its own (`sudo -u root rm`,
# `xargs -I{} rm`), which option-shaped matching alone cannot reach because an
# option's ARGUMENT looks like nothing in particular.  Both alternatives only ever
# make the verb test match MORE commands, never fewer.
# `timeout` is in the second alternative only, and that placement is the point: unlike every other
# wrapper here it takes a MANDATORY operand of its own (`timeout 28 cargo check`, `timeout 5s rm`),
# and a bare duration is neither option-shaped nor an assignment, so the first alternative cannot
# consume it and the verb after it was in nobody's command position. Measured 2026-09-21 against
# the 2026-09-03 incident command, `timeout 28 cargo check ... | grep -E 'error'`, which
# no_grep_for_build_errors stopped denying the moment it was tightened to command position -- and
# the same hole sat in front of `timeout 5 rm -rf /` for every guard that reads this function,
# because nothing here had ever been asked about a wrapper with an argument.
command_position_prefix_pattern := `(^|[;&|(){}\n])[ \t]*(([A-Za-z_][A-Za-z0-9_]*=[^ \t]*|-{1,2}[A-Za-z0-9][^ \t]*|sudo|doas|env|command|nohup|time|exec|xargs|then|else|do)[ \t]+|(sudo|doas|env|command|nohup|time|exec|xargs|timeout)[ \t]+([^ \t;&|(){}]+[ \t]+){1,4})*`

# An optional path COMPONENT PREFIX immediately before a program name --
# `/bin/`, `/usr/bin/`, `./`, `../scripts/`. REQUIRES a trailing slash (the `?`
# makes the whole group optional, not the slash inside it), which is what stops
# a longer word that merely ENDS in the verb (`confirm` for `rm`, `xmv` for
# `mv`) from matching: the group can only consume characters that are
# immediately followed by a literal `/`, so a verb-shaped suffix with no
# preceding slash falls through to the zero-width alternative and then must
# match the verb starting exactly at the anchor. Shared with shell_name_pattern
# below rather than re-spelled, so the two callers cannot drift apart.
path_prefix_pattern := `([^ \t;&|()'"]*/)?`

# has_command_verb answers "is this word the program a shell would run", which
# command_position_prefix_pattern alone gets right for a BARE verb (`rm -rf x`)
# but not for one invoked by absolute or relative PATH (`/bin/rm -rf x`,
# `./rm -rf x`) -- measured 2026-08-31 (bd er-effects-rs-5z75): a leading path
# component is none of the anchor's separator characters, and the character
# immediately before the verb is `/`, so the anchor never lined up and
# `/bin/rm -rf .cupcake` / `/usr/bin/rm -rf .cupcake` were allowed outright by
# every guard that reads this function. NOT a regression: has_verb's
# `(^|\s)verb(\s|$)` boundary missed it identically, so this predates both the
# command-position work and the segmentation work that came before it -- it was
# simply never covered. Splicing path_prefix_pattern in between the
# command-position prefix and the verb closes it: the prefix must still resolve
# back to a real anchor (string start, a separator, or consumed wrapper words),
# so a slash deep inside an unrelated operand still cannot manufacture a match
# (see commands_test.rego for the negative cases this reasoning predicts).
has_command_verb(command, verb) if {
	pattern := concat("", [command_position_prefix_pattern, path_prefix_pattern, verb, `([ \t\n;&|(){}]|$)`])
	regex.match(pattern, command)
}

# ---------------------------------------------------------------------------
# SHELL SEGMENTS (2026-08-31, bd er-effects-rs-c0t9)
#
# Moved here from guard_layer_destructive_guard.rego, which had the only copy.
# The move is the point: a SECOND segmenter written for the next policy that
# needs one is a divergence bug waiting to happen, and the next policy that
# needed one arrived the same day.
#
# WHAT PROBLEM IT SOLVES. A guard that ANDs a verb test with an operand test over
# the WHOLE command string is asking whether the two tokens are CO-PRESENT, not
# whether they belong to the same command. That is not the question any of these
# rules means to ask, and it fires on commands where the two never meet:
#
#     rm -rf <some unrelated dir> && mkdir -p <hooks dir> && cp shim <hooks dir>/
#
# -- a removal aimed at one path, and a hooks directory named in a DIFFERENT
# statement, read as removing a hook. Measured live twice on 2026-08-31, the
# second time on the bug report about the first, because the report quoted both
# tokens. Evaluating the predicate PER SEGMENT is what closes it.
#
# THE INPUT CONTRACT, and it is load-bearing: callers must pass text that has
# been through scan_text / executed_texts, never the raw command. This splits on
# separator CHARACTERS, so a `;` inside a quoted operand would split a segment a
# shell would keep whole -- and splitting the wrong place separates a verb from
# its operand, which is the fail-OPEN direction. executed_texts blanks the
# anchors inside quoted spans before this ever sees them, so under that contract
# a quoted separator is already a space. Both callers satisfy it.
#
# TWO GRANULARITIES, because `|` is not the same kind of boundary as the others.
# `;`, `&&`, `||`, `&` and a newline separate commands that share nothing. A PIPE
# separates commands that share a data stream -- which is exactly how a removal
# gets its operand from somewhere else in the line:
#
#     echo <hooks dir>/pre-commit | xargs rm -f
#
# The path is in one stage and the verb in the next, and the file is deleted all
# the same. So a co-presence guard whose verb set is small and specific should
# read shell_statements (pipelines kept whole) and accept the narrow over-denial
# that comes with it; a guard whose predicate is broad should read shell_segments
# and accept the narrow gap instead. The destructive guard is the broad case --
# at statement granularity `git diff .cupcake | grep -c restore` would be denied
# by its `git (checkout|restore|clean)` arm, which is a worse trade than the
# xargs gap. That choice is per policy; the SPLITTING is not, and lives here once.
#
# NOT SPLIT ON, deliberately: command substitution. `$(...)` really does open a
# new command context, but leaving it joined to its host errs toward treating
# `echo $(rm -rf <path>)` as one command that both removes and names the path --
# a DENY, the safe direction. Splitting it would be the direction that loses a
# real removal, so the honest gap here costs precision, never coverage.

# Statement boundaries only. `||` and `&&` are consumed WHOLE before the single
# characters, so that neither leaves a stray `|` behind to be read as a pipe.
shell_statements(cmd) := out if {
	no_or := replace(cmd, "||", "\n")
	no_and := replace(no_or, "&&", "\n")
	replaced := replace(replace(no_and, ";", "\n"), "&", "\n")
	out := {statement |
		some raw in split(replaced, "\n")
		statement := trim_space(raw)
		statement != ""
	}
}

# Statements further cut at each data pipe. Set-equal to the original single-pass
# `|`/`&`/`;` replacement this was factored out of.
shell_segments(cmd) := out if {
	out := {segment |
		some statement in shell_statements(cmd)
		some raw in split(statement, "|")
		segment := trim_space(raw)
		segment != ""
	}
}

# Detect symlink creation commands
# Matches: ln -s, ln -sf, ln -s -f, etc.
creates_symlink(command) if {
	has_verb(command, "ln")
	contains(command, "-s")
}

# Check if symlink command involves a protected path
# IMPORTANT: Checks BOTH source and target (addresses TOB-EQTY-LAB-CUPCAKE-4)
# Blocks: ln -s .cupcake foo AND ln -s foo .cupcake
symlink_involves_path(command, protected_path) if {
	creates_symlink(command)
	contains(command, protected_path)
}

# Detect output redirection operators that could bypass file protection
# Matches: >, >>, |, tee
has_output_redirect(command) if {
	redirect_patterns := [
		`\s>\s`, # stdout redirect
		`\s>>\s`, # stdout append
		`\s\|\s`, # pipe
		`(^|\s)tee(\s|$)`, # tee command
	]
	some pattern in redirect_patterns
	regex.match(pattern, command)
}

# ---------------------------------------------------------------------------
# EXECUTED-TEXT DECOMPOSITION (2026-08-26, bd er-effects-rs-dt2e)
#
# A guard that anchors its command pattern on `(^|[;&|(\n])` is asking about
# LEXICAL POSITION in the raw command string. That is not the question it means
# to ask, and it is wrong in BOTH directions at once:
#
#   bash -c 'git push origin main'    the character before the verb is a quote,
#                                     which is not in that class, so the guard
#                                     sees nothing -- and the command RUNS.
#                                     AGENTS.md actively tells agents to wrap
#                                     commands this way for fish, so the bypass
#                                     form is one this repo recommends.
#
#   bd remember --key k "...          a newline IS in that class, so a memory
#   <the very command being guarded>  body, a commit message or a doc that
#   ..."                              merely QUOTES the command is denied --
#                                     and nothing was ever going to run.
#
# Widening the class alone makes the second failure worse while only partly
# fixing the first. The fix is to stop matching on position in the raw string
# and start matching on EXECUTED TEXT:
#
#   executed_texts(command)
#       the set of shell texts this command actually hands to a shell. Element
#       one is the command itself with quoted OPERAND spans anchor-neutralised
#       (their contents kept, so `git -C "/path with space" push` still parses,
#       but `\n ; & | ( )` inside them turned to spaces so quoted prose has no
#       command position). The remaining elements are the payloads of shell
#       wrappers -- `bash -c '<payload>'` and friends -- each decomposed the
#       same way, so a payload's own separators keep working while a payload's
#       own quoted prose does not.
#
#   executed_unquoted_texts(command)
#       the same set with quoted spans DELETED rather than neutralised, for
#       unanchored substring tests (`contains(cmd, "--flag")`) that must not
#       fire on a flag named inside quoted prose.
#
#   unparsed_shell_payload(command)
#       true when a shell wrapper is present whose payload could NOT be
#       decomposed, so callers can fail closed instead of treating the guard's
#       blindness as an allow.
#
# MEASURED ENGINE BEHAVIOUR (2026-08-26). `cupcake eval` replaces UNQUOTED
# newlines with spaces before policy evaluation, while newlines inside quotes
# survive. So a two-line command's second line arrives with no separator in
# front of it and is invisible to every anchored pattern -- in the live engine
# only; `opa test` sees the raw text. That is an engine-input problem, not one a
# pattern here can reach, and it is filed separately. Everything below behaves
# identically under both because it never depends on an unquoted newline.
#
# LIMITS, all deliberate and all fail-closed (they keep the RAW text, which is
# what the guards matched before this existed, so they can only deny more):
#   * command substitution -- `$(...)` or a backtick anywhere in a text leaves
#     that text un-neutralised, because substitution inside double quotes really
#     does execute;
#   * unbalanced quotes leave the text un-neutralised, because the quote-index
#     parity this relies on is meaningless once a quote is unmatched;
#   * exactly one heredoc is understood, and only with a resolvable terminator;
#   * wrapper nesting is followed three levels deep (see shell_payloads_deep).
# ---------------------------------------------------------------------------

# Escaped quotes are removed first so that quote-index parity below cannot be
# desynchronised by `\"` / `\'`. Same shape as the launch guard's scrub.
escaped_quotes_stripped(text) := replace(replace(text, `\"`, ""), `\'`, "")

quote_parity_ok(text) if {
	count(split(text, `"`)) % 2 == 1
	count(split(text, "'")) % 2 == 1
}

# Characters a command pattern uses to recognise a COMMAND POSITION. Blanking
# them inside a quoted operand is what stops prose from looking like a command,
# while every other character is preserved so quoted operands still parse.
#
# Quote characters are blanked too, and that is not cosmetic: a quote INSIDE a
# quoted span is literal text, and leaving it in desynchronises the next phase's
# quote-index parity. Without this, `bd remember --key k "it's about ...<newline>
# git push origin main"` fails the single-quote parity check on one apostrophe,
# falls back to the raw text, and is denied for a memory body again.
blanked_anchors(text) := out if {
	no_newline := replace(replace(text, "\n", " "), "\r", " ")
	no_sep := replace(replace(replace(no_newline, ";", " "), "&", " "), "|", " ")
	no_paren := replace(replace(no_sep, "(", " "), ")", " ")
	out := replace(replace(no_paren, "'", " "), `"`, " ")
}

odd_span_blanked(index, text) := text if {
	index % 2 == 0
}

odd_span_blanked(index, text) := blanked_anchors(text) if {
	index % 2 == 1
}

# The shell NAMES this decomposition recognises, as one alternation so the four
# patterns below cannot drift apart. `(ba|z|k|da|a|fi|c|tc)?sh` covers sh, bash,
# zsh, ksh, dash, ash, fish, csh and tcsh.
#
# fish/csh/tcsh were ADDED 2026-08-31 and were a real hole, not tidiness: this
# repo's AGENTS.md tells agents to wrap commands for fish, and `fish -c 'git push
# origin main'` decomposed to NOTHING before -- measured, along with `csh -c` and
# `tcsh -c`, while every other name on the list decomposed correctly. A name that
# is not on this list is not a hypothetical gap; it is a working bypass of every
# guard that reads executed_texts.
#
# The leading path_prefix_pattern is what admits `/bin/bash`, and it REQUIRES a
# trailing slash, so a longer word that merely ends in one of these names (`mycsh`,
# `wifish`) cannot match: the alternation has to consume the token whole from its
# boundary. Shared with has_command_verb above (2026-08-31, bd er-effects-rs-5z75)
# rather than re-spelled, so the two cannot drift apart.
shell_name_pattern := concat("", [path_prefix_pattern, `(ba|z|k|da|a|fi|c|tc)?sh`])

# A shell wrapper whose program text is the NEXT argument: `bash -c`, `sh -c`,
# `zsh -c`, `ksh -c`, `dash -c`, `fish -c`, `bash -lc`, `/bin/bash --norc -c`,
# `eval`, and the same reached through `command` / `env VAR=x` /
# `xargs -I{} bash -c`. The pattern matches the text that PRECEDES a quote, so it
# is anchored at its end.
# `python3 -c` is deliberately absent: its argument is Python, not shell.
#
# `ssh host '...'` is deliberately absent too, and that is a decision rather than
# an oversight: its payload runs on ANOTHER machine, so reading it as an executed
# text would have the protected-paths rules reason about this host's `/etc` from a
# program that cannot touch it. Nothing here can see a remote filesystem; claiming
# otherwise would be a guard that is wrong on purpose.
# THE `eval` BRANCH NEEDS ITS OWN ANCHOR (2026-09-21).
#
# Every pattern below used to spell the wrapper as `(<shell> ... -c|eval)` behind ONE shared
# prefix whose anchor class is `(^|[ \t;&|(])` -- a SPACE is enough. That anchor is right for the
# shell form, which needs it to reach `sudo bash -c` and `xargs -I{} bash -c`; it is wrong for
# `eval`, which is a bare word, because it makes any command containing `eval` as a separate token
# look like a shell wrapper whose payload cannot be read. Two of this repo's most ordinary
# commands are exactly that shape:
#
#     cupcake eval --harness claude --strict < event.json
#     opa eval 'data.cupcake.system.commands.unparsed_shell_payload'
#
# Both were read as "wraps a shell payload the guard cannot read", so every guard that fails
# closed on unparsed_shell_payload denied them the moment the command also happened to name git
# and push, or pgrep -- which is to say, whenever an agent was investigating those guards.
# Measured twice in one session while debugging this file.
#
# The fix is command position for the bare words only. `eval`, `source` and `.` must stand where a
# shell would run them: at the start, after a real separator, or behind a wrapper word. The shell
# `-c` form keeps the looser anchor it needs, so nothing that decomposed before stops decomposing.
eval_position_prefix := `(^|[;&|(){}\n])[ \t]*((sudo|doas|env|command|nohup|time|exec|timeout|xargs)[ \t]+([^ \t;&|(){}]+[ \t]+){0,4})?`

shell_wrapper_head := concat("", [`(^|[ \t;&|(])(command[ \t]+)?(env([ \t]+[a-z_][a-z0-9_]*=[^ \t]*)+[ \t]+)?`, shell_name_pattern, `([ \t]+-{1,2}[a-z][a-z0-9-]*)*[ \t]+-[a-z]*c[a-z]*`])

eval_head := concat("", [eval_position_prefix, `eval`])

shell_payload_prefix_pattern := concat("", [`(?i)(?:`, shell_wrapper_head, `|`, eval_head, `)[ \t]*$`])

# The same wrapper, followed by something that is NOT a quote -- i.e. a payload
# this decomposition cannot read (`bash -c $CMD`, `bash -c git\ push`).
shell_payload_unparsed_pattern := concat("", [`(?i)(?:`, shell_wrapper_head, `|`, eval_head, `)[ \t]*[^ \t'"]`])

# A wrapper token anywhere, used only to decide whether unbalanced quotes are
# worth failing closed over.
shell_wrapper_anywhere_pattern := concat("", [`(?i)(?:`, shell_wrapper_head, `|`, eval_head, `)([ \t]|$)`])

# A heredoc whose READER is a shell: its body is a program, not data.
shell_heredoc_reader_pattern := concat("", [
	`(?i)(?:(^|[ \t;&|(])(command[ \t]+)?`,
	shell_name_pattern,
	`|`,
	eval_position_prefix,
	`(eval|source|\.))([ \t]+-{1,2}[a-z][a-z0-9-]*)*[ \t]*$`,
])

# Quoted payload of a shell wrapper, one nesting level. Odd split indices are
# quoted spans; a span counts as a payload only when the text immediately before
# its opening quote is a wrapper awaiting its program AND that opening quote is
# not itself inside a span of the OTHER quote style.
#
# That second condition is what keeps a commit message describing this very fix
# from being read as an executed payload:
#
#   git commit -m "the bypass form was bash -c 'git push origin main'"
#
# Splitting on `'` alone finds a span whose preceding text ends in `bash -c `, so
# without the nesting check the message body becomes an executed text and the
# commit is denied -- the same false positive in a new costume. The check counts
# quotes of the other style before the opening quote: an ODD count means the
# quote is inside such a span and is literal text, not a shell quote.
#
# Doing it this way rather than by scrubbing the other style first also keeps the
# payload whole: `bash -c 'git push origin "main"'` yields the full payload
# rather than one with its double-quoted operand cut out of the middle.
shell_payloads(text) := payloads if {
	stripped := escaped_quotes_stripped(text)
	single := split(stripped, "'")
	double := split(stripped, `"`)
	payloads := {p |
		some i
		single[i]
		i % 2 == 1
		regex.match(shell_payload_prefix_pattern, single[i - 1])
		outside_other_quote_style(array.slice(single, 0, i), "'", `"`)
		p := single[i]
	} | {p |
		some j
		double[j]
		j % 2 == 1
		regex.match(shell_payload_prefix_pattern, double[j - 1])
		outside_other_quote_style(array.slice(double, 0, j), `"`, "'")
		p := double[j]
	}
}

# The text before the opening quote, reassembled from the split, must contain an
# EVEN number of the other quote character (split count odd) for that opening
# quote to be a real shell quote rather than literal text inside another span.
outside_other_quote_style(prefix_parts, own_quote, other_quote) if {
	prefix := concat(own_quote, prefix_parts)
	count(split(prefix, other_quote)) % 2 == 1
}

# THREE levels of nesting. Two is the deepest that literal quoting reaches
# without escapes (`bash -c 'bash -c "..."'`), so the third exists purely as
# headroom; a fourth would need escaped quotes, which are stripped before the
# split and so cannot form a payload at all. A wrapper deeper than this is
# reported by unparsed_shell_payload rather than silently ignored.
shell_payloads_deep(text) := deep if {
	level1 := shell_payloads(text)
	level2 := {p |
		some q in level1
		some p in shell_payloads(q)
	}
	level3 := {p |
		some q in level2
		some p in shell_payloads(q)
	}
	deep := (level1 | level2) | level3
}

# Heredoc bodies are DATA for the command reading them, so their contents get
# the same treatment as a quoted operand -- unless a shell is what reads them,
# in which case the body stays raw (fail closed) and its separators keep working.
heredoc_body_blanked(text) := out if {
	parts := split(text, "<<")
	count(parts) == 2
	not regex.match(shell_heredoc_reader_pattern, parts[0])
	tag := heredoc_tag(parts[1])
	segments := split(parts[1], concat("", ["\n", tag]))
	count(segments) == 2

	# THE BODY IS WRAPPED IN A SYNTHETIC DOUBLE QUOTE, and that is this repo's divergence from
	# er-mods-rs, kept deliberately when commands.rego was refreshed from it on 2026-09-21.
	#
	# er-mods-rs emits `blanked_anchors(segments[0])` bare, so the body's TEXT survives into
	# executed_unquoted_texts and every substring/flag test reads it. It then buys back the cases
	# that breaks with per-policy exemptions -- block_manual_pgrep carries one for `bd` text and
	# another for git commit MESSAGE text, each with its own single-command, no-chaining
	# conditions. That is a workable design; it is not the one the policies in THIS repo were
	# written against.
	#
	# Here the body is a quoted span, so quotes_removed drops it whole and a data heredoc is
	# uniformly invisible to flag tests -- which is what ds2's own guards assume. Measured when the
	# bare form was imported: `git commit -q -F - <<'EOF' ... pgrep -f DarkSoulsII.exe ... EOF`, a
	# commit message DOCUMENTING the pgrep rule, was denied by the rule it documents, and the same
	# went for a message quoting `git push origin main`. Those two allow-cases are pinned in
	# scripts/test-cupcake-policies.py and in the guards' own test suites.
	#
	# The exemption is for a body a NON-SHELL command reads -- the `not shell_heredoc_reader` test
	# above is what keeps `bash <<EOF` out, because there the body really is a program.
	quoted_body := concat("", [`"`, blanked_anchors(segments[0]), `"`])
	out := concat("", [parts[0], " ", quoted_body, " ", tag, segments[1]])
}

heredoc_tag(after_marker) := tag if {
	matched := regex.find_all_string_submatch_n(`^-?[ \t]*["']?([A-Za-z_][A-Za-z0-9_]*)["']?`, after_marker, 1)
	tag := matched[0][1]
}

heredoc_resolved(text) := out if {
	out := heredoc_body_blanked(text)
} else := text

# Blank the OUTER quote style's spans first, then the INNER style's spans in what
# survives. Parity is checked per phase, and the second phase's parity is read
# AFTER the first has blanked the outer spans' contents -- which is why a lone
# apostrophe inside a double-quoted body (`"it's about ..."`) no longer makes the
# whole text unparseable and drop back to its raw form.
blank_spans(text, outer, inner) := out if {
	first := split(text, outer)
	count(first) % 2 == 1
	phase_one := concat(outer, [span |
		some i
		part := first[i]
		span := odd_span_blanked(i, part)
	])
	second := split(phase_one, inner)
	count(second) % 2 == 1
	out := concat(inner, [span |
		some j
		part := second[j]
		span := odd_span_blanked(j, part)
	])
}

# One shell text with its quoted operand spans anchor-neutralised. Undefined
# (so callers fall back to the raw text) when command substitution or unbalanced
# quotes make the parity read meaningless. Double-quoted-outer is tried first
# because it is the commoner shell idiom; single-quoted-outer is the fallback,
# which is what reads `echo 'don"t' ; ...` correctly. Neither can open a hole:
# each still requires its own balanced parity, and a text that satisfies neither
# keeps its raw form.
operands_blanked(text) := out if {
	resolved := scannable(text)
	out := blank_spans(resolved, `"`, "'")
} else := out if {
	resolved := scannable(text)
	out := blank_spans(resolved, "'", `"`)
}

scannable(text) := stripped if {
	resolved := heredoc_resolved(text)
	not contains(resolved, "$(")
	not contains(resolved, "`")
	stripped := escaped_quotes_stripped(resolved)
}

scan_text(text) := out if {
	out := operands_blanked(text)
} else := text

# Quoted spans DELETED rather than neutralised, for unanchored substring tests.
quotes_removed(text) := out if {
	stripped := escaped_quotes_stripped(heredoc_resolved(text))
	double := split(stripped, `"`)
	outside_double := concat(" ", [part |
		some i
		part := double[i]
		i % 2 == 0
	])
	single := split(outside_double, "'")
	out := concat(" ", [part |
		some j
		part := single[j]
		j % 2 == 0
	])
}

# PUBLIC. Every shell text this command executes, quoted operands neutralised.
executed_texts(command) := texts if {
	payloads := shell_payloads_deep(command)
	texts := {scan_text(command)} | {t |
		some p in payloads
		t := scan_text(p)
	}
}

# PUBLIC. The same set with quoted spans removed, for substring/flag tests.
executed_unquoted_texts(command) := texts if {
	payloads := shell_payloads_deep(command)
	texts := {quotes_removed(command)} | {t |
		some p in payloads
		t := quotes_removed(p)
	}
}

# PUBLIC. A shell wrapper is present whose payload could not be decomposed.
# Callers must treat this as "I cannot see what runs", never as an allow.
unparsed_shell_payload(command) if {
	regex.match(shell_payload_unparsed_pattern, quotes_removed(command))
}

unparsed_shell_payload(command) if {
	not quote_parity_ok(escaped_quotes_stripped(command))
	regex.match(shell_wrapper_anywhere_pattern, command)
}

# Everything from here down was ported from er-mods-rs's copy of this file on 2026-09-25, unchanged,
# for the two guards that read it: no_rust_edit_without_frida_proof and teardown_must_relaunch. It is
# additive -- nothing above this line calls it -- and the bd ids below are that repo's.

# ---------------------------------------------------------------------------
# COMMAND SLOT, AS TOKENS (2026-09-22, bd er-effects-rs-ak3q / er-effects-rs-if5l)
#
# `has_command_verb` above answers the same question with a regex, and it is the
# right tool when the program name is a fixed word. It is the wrong one when the
# name is a PATH the caller has to recognise by suffix -- `scripts/er-teardown.py`,
# `./scripts/check.sh`, `/usr/bin/python3` -- because the pattern then has to carry
# the path shape as well as the anchor, and this package's own header records what
# a regex costs here: on 2026-09-14 one crashed every policy at once.
#
# So the same decision is available as token arithmetic. `no_whole_check_sh` wrote
# it first and the next two policies that needed it were about to transcribe it,
# which is the divergence bug the segmenter note above already warns about. It
# lives here now and that file reads it like everyone else.
#
# THE INPUT CONTRACT is the one shell_segments states: pass text that has been
# through scan_text / executed_texts, never the raw command. Those blank the
# anchors inside quoted spans, which is what makes a separator in a commit message
# or a `bd remember` body stop looking like a separator.

# A word that cannot be the separator it looks like, because the anchors inside
# quoted spans are already neutralised by the time this splits. Kept as a distinct
# token rather than collapsed to a space so command position survives the split.
command_slot_separator := "__cupcake_command_slot_sep__"

# Spaces either side, so a separator written tight against its neighbours
# (`a;b`, `x&&y`) still splits into three words rather than one.
command_slot_marker := " __cupcake_command_slot_sep__ "

# PUBLIC. One executed text as words, with every command separator standing as a
# word of its own.
command_slot_words(text) := [word |
	marked := replace(
		replace(
			replace(replace(text, ";", command_slot_marker), "&", command_slot_marker),
			"|", command_slot_marker,
		),
		"(", command_slot_marker,
	)
	some word in split(marked, " ")
	word != ""
]

# Words that may stand between a command slot and the thing it runs without the
# thing stopping being a command: the shells, the exec wrappers, their flags and
# numeric arguments, and leading `VAR=value` assignments.
#
# The set is closed on purpose and short. Anything outside it -- `git`, `cat`,
# `python3`, `bd` -- means the words after it are that command's OPERANDS, which
# is the whole of how a commit message naming a guarded script stays allowed.
command_slot_wrapper(word) if endswith(word, "bash")

command_slot_wrapper(word) if endswith(word, "sh")

command_slot_wrapper(word) if endswith(word, "zsh")

command_slot_wrapper(word) if endswith(word, "env")

command_slot_wrapper("command")

command_slot_wrapper("timeout")

command_slot_wrapper("nice")

command_slot_wrapper("nohup")

command_slot_wrapper("setsid")

command_slot_wrapper("stdbuf")

command_slot_wrapper("exec")

command_slot_wrapper("sudo")

command_slot_wrapper("uv")

# Shell keywords that stand in front of a command without being one. The same three are
# already in `command_position_prefix_pattern` above, and the loop body is where they
# matter: `for c in ...; do sed -i ... crates/$c/src/config.rs; done` splits into a
# statement whose first word is `do`, and reading `do` as the program hides the `sed`
# behind it. That exact loop rewrote four crate sources past the Frida gate on 2026-09-21
# (bd er-effects-rs-wuij).
command_slot_wrapper(word) if word in {"do", "then", "else"}

# Interpreters stand aside the same way shells do. The program a shell runs for
# `python3 scripts/er-teardown.py` really is `python3`, but the thing every guard
# in this rulebook needs to recognise is the SCRIPT it was handed -- no policy
# has ever cared that the interpreter was involved. `startswith` carries the
# versioned spellings (`python3.12`) and the `/`-form carries the absolute ones
# (`/usr/bin/python3`).
command_slot_wrapper(word) if startswith(word, "python")

command_slot_wrapper(word) if contains(word, "/python")

command_slot_wrapper(word) if word in {"perl", "ruby", "node", "uvx", "pwsh"}

command_slot_wrapper(word) if startswith(word, "-")

# `timeout 30`, `nice -n 5`. Spelled as a digit test rather than `regex.match`
# on `^[0-9]+$`: the header of this package records a regex in a rule crashing
# every policy at once, and a rule that runs once per word of every command is
# the last place to spend one.
command_slot_wrapper(word) if {
	word != ""
	count([c |
		some c in split(word, "")
		not digit_char(c)
	]) == 0
}

digit_char(c) if c in {"0", "1", "2", "3", "4", "5", "6", "7", "8", "9"}

# `VAR=value`, which is a prefix to a command rather than a command. Excludes a
# path-looking operand so `--out=/some/path` is not read as an assignment.
command_slot_wrapper(word) if {
	contains(word, "=")
	not contains(word, "/")
}

# The index just past the nearest separator before `index`, or 0 when there is
# none: where the command containing `index` begins.
command_slot_start(words, index) := start if {
	befores := [position |
		some position, word in words
		position < index
		word == command_slot_separator
	]
	start := max(array.concat(befores, [-1])) + 1
}

# Wrapper-ness that needs the word BEFORE it to decide. `uv run --with frida
# python3 x.py` is the shape AGENTS.md asks for by name, and no per-word rule can
# read it: `run` on its own is `cargo run` and `npm run`, whose next word is a
# target rather than a program, and `frida` is the value of a flag rather than a
# word with a shape. Both are wrappers here only because of what precedes them.
command_slot_wrapper_at(words, i) if {
	command_slot_wrapper(words[i])
}

command_slot_wrapper_at(words, i) if {
	words[i] == "run"
	words[i - 1] == "uv"
}

command_slot_wrapper_at(words, i) if {
	words[i - 1] == "--with"
}

# PUBLIC. True when every word from the start of this command up to `index` is a
# wrapper, which is what makes the word at `index` the thing being run.
word_in_command_slot(words, index) if {
	start := command_slot_start(words, index)
	count([position |
		some position, _ in words
		position >= start
		position < index
		not command_slot_wrapper_at(words, position)
	]) == 0
}

# ---------------------------------------------------------------------------
# THIS EVENT'S DECOMPOSITION, COMPUTED ONCE (2026-09-22, bd er-effects-rs-xktj)
#
# Nine policies called `executed_texts(input.tool_input.command)` with the same
# argument, and the compiled wasm module re-ran the decomposition for each one:
# several copies of the command per call plus the split arrays behind them. On a
# 10 KB command that exhausted the module's linear memory and `opa_malloc`
# aborted the evaluation -- which in the production hook is not a refusal, it is
# `{}` at exit 0, every policy in the rulebook silent at once.
#
# Measured 2026-09-22 by bisecting the length of a `cat > doc.md <<'EOF' ... EOF`
# artifact write until it aborts, against a copy of this policy set with only this
# change applied:
#
#   per-policy call, 10MB (the engine default)    10,355 bytes
#   read as a rule,  10MB                         15,931 bytes
#   per-policy call, 100MB (the engine ceiling)  118,705 bytes
#   read as a rule,  100MB                       180,095 bytes
#
# A rule is a virtual document, evaluated once per query and cached, so reading
# these instead of calling the function collapses the work to a single pass.
# Left undefined when the command key is absent, exactly as the direct call was,
# so no caller's definedness changes.
#
# This is the smaller of the two levers and it is the one that survives an engine
# upgrade. The larger is `--wasm-max-memory 100MB` in `scripts/cupcake-hook.sh`,
# which multiplies whatever this achieves by about eleven.
input_executed_texts := executed_texts(input.tool_input.command)

input_executed_unquoted_texts := executed_unquoted_texts(input.tool_input.command)

input_unparsed_shell_payload if {
	unparsed_shell_payload(input.tool_input.command)
}
