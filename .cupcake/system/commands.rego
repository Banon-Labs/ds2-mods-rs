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

# Check if command contains ANY of the dangerous verbs from a set
# More efficient than checking each verb individually in policy code
has_dangerous_verb(command, verb_set) if {
	some verb in verb_set
	has_verb(command, verb)
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

# A shell wrapper whose program text is the NEXT argument: `bash -c`, `sh -c`,
# `zsh -c`, `ksh -c`, `dash -c`, `bash -lc`, `/bin/bash --norc -c`, `eval`, and
# the same reached through `command` / `env VAR=x` / `xargs -I{} bash -c`. The
# pattern matches the text that PRECEDES a quote, so it is anchored at its end.
# `python3 -c` is deliberately absent: its argument is Python, not shell.
shell_payload_prefix_pattern := `(?i)(^|[ \t;&|(])(command[ \t]+)?(env([ \t]+[a-z_][a-z0-9_]*=[^ \t]*)+[ \t]+)?(([^ \t;&|()'"]*/)?(ba|z|k|da|a)?sh([ \t]+-{1,2}[a-z][a-z0-9-]*)*[ \t]+-[a-z]*c[a-z]*|eval)[ \t]*$`

# The same wrapper, followed by something that is NOT a quote -- i.e. a payload
# this decomposition cannot read (`bash -c $CMD`, `bash -c git\ push`).
shell_payload_unparsed_pattern := `(?i)(^|[ \t;&|(])(command[ \t]+)?(env([ \t]+[a-z_][a-z0-9_]*=[^ \t]*)+[ \t]+)?(([^ \t;&|()'"]*/)?(ba|z|k|da|a)?sh([ \t]+-{1,2}[a-z][a-z0-9-]*)*[ \t]+-[a-z]*c[a-z]*|eval)[ \t]*[^ \t'"]`

# A wrapper token anywhere, used only to decide whether unbalanced quotes are
# worth failing closed over.
shell_wrapper_anywhere_pattern := `(?i)(^|[ \t;&|(])(([^ \t;&|()'"]*/)?(ba|z|k|da|a)?sh([ \t]+-{1,2}[a-z][a-z0-9-]*)*[ \t]+-[a-z]*c[a-z]*|eval)([ \t]|$)`

# A heredoc whose READER is a shell: its body is a program, not data.
shell_heredoc_reader_pattern := `(?i)(^|[ \t;&|(])(command[ \t]+)?(([^ \t;&|()'"]*/)?(ba|z|k|da|a)?sh|eval|source|\.)([ \t]+-{1,2}[a-z][a-z0-9-]*)*[ \t]*$`

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
	out := concat("", [parts[0], " ", blanked_anchors(segments[0]), " ", tag, segments[1]])
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

