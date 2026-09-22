# OPA unit tests for the shared command helpers, and in particular for the
# executed-text decomposition every git guard now reads (bd er-effects-rs-dt2e).
#
# Run with:
#   opa test .cupcake/system/commands.rego .cupcake/tests/commands_test.rego
package cupcake.system.commands_test

import rego.v1

import data.cupcake.system.commands

# Assembled rather than written whole, so this FILE is not itself denied by the
# guard it tests when an agent edits it through a Bash command.
push_main := concat(" ", ["git", "push", "origin", "main"])

# --- wrapper payloads become executed texts -----------------------------------

test_single_quoted_bash_c_payload if {
	commands.executed_texts(concat("", ["bash -c '", push_main, "'"])) == {
		concat("", ["bash -c '", push_main, "'"]),
		push_main,
	}
}

test_double_quoted_sh_c_payload if {
	push_main in commands.executed_texts(concat("", [`sh -c "`, push_main, `"`]))
}

test_login_shell_flag_payload if {
	push_main in commands.executed_texts(concat("", ["bash -lc '", push_main, "'"]))
}

test_zsh_payload if {
	push_main in commands.executed_texts(concat("", [`zsh -c "`, push_main, `"`]))
}

test_absolute_path_shell_with_options_payload if {
	push_main in commands.executed_texts(concat("", ["/bin/bash --norc -c '", push_main, "'"]))
}

test_env_prefixed_shell_payload if {
	push_main in commands.executed_texts(concat("", ["env FOO=1 bash -c '", push_main, "'"]))
}

test_eval_payload if {
	push_main in commands.executed_texts(concat("", [`eval "`, push_main, `"`]))
}

test_xargs_wrapped_shell_payload if {
	push_main in commands.executed_texts(concat("", ["echo x | xargs -I{} bash -c '", push_main, "'"]))
}

# Nesting: quoting cannot alternate more than twice without escapes, and escaped
# quotes are stripped before the split, so two levels is the reachable maximum
# and the third level of unrolling in shell_payloads_deep is headroom.
test_nested_wrapper_payload_two_levels if {
	push_main in commands.executed_texts(concat("", ["bash -c 'bash -c \"", push_main, "\"'"]))
}

# --- shells that were NOT on the list (2026-08-31) -----------------------------
#
# `(ba|z|k|da|a)?sh` covered eight names and missed three. fish is the one that
# mattered: this repo's own AGENTS.md instructs agents to wrap commands for fish,
# so `fish -c '<guarded command>'` was a bypass the instructions recommend.
# Measured before the fix -- all three decomposed to the EMPTY set, which every
# caller reads as "no payload here", i.e. an allow.

test_fish_payload if {
	push_main in commands.executed_texts(concat("", ["fish -c '", push_main, "'"]))
}

test_csh_payload if {
	push_main in commands.executed_texts(concat("", ["csh -c '", push_main, "'"]))
}

test_tcsh_payload if {
	push_main in commands.executed_texts(concat("", ["tcsh -c '", push_main, "'"]))
}

# ... and the alternation must still consume the token WHOLE from its boundary,
# so a longer word merely ENDING in a shell name is not a wrapper. Without this
# the widening would be a licence to read any `*sh`-suffixed program's quoted
# argument as shell.
test_longer_word_ending_in_a_shell_name_is_not_a_wrapper if {
	not push_main in commands.executed_texts(concat("", ["mycsh -c '", push_main, "'"]))
}

test_longer_word_ending_in_fish_is_not_a_wrapper if {
	not push_main in commands.executed_texts(concat("", ["wifish -c '", push_main, "'"]))
}

# `ssh host '<program>'` is deliberately NOT a payload: it runs on another
# machine, so reading it here would have the protected-path rules reason about
# THIS host's filesystem from a program that cannot reach it.
test_ssh_remote_argument_is_not_a_local_payload if {
	not push_main in commands.executed_texts(concat("", ["ssh host '", push_main, "'"]))
}

# --- a quoted OPERAND is not a payload ----------------------------------------

# `python3 -c` takes a Python program, not shell, so its argument must not be
# decomposed as shell text.
test_python_dash_c_is_not_a_shell_payload if {
	not push_main in commands.executed_texts(concat("", ["python3 -c '", push_main, "'"]))
}

test_echo_argument_is_not_a_payload if {
	not push_main in commands.executed_texts(concat("", [`echo "`, push_main, `"`]))
}

# A quote inside the OTHER quote style must not be read as a wrapper boundary.
# The payload comes back WHOLE, double-quoted operand and all; the outer text's
# copy of it is neutralised, which blanks the inner quotes.
test_inner_quote_of_other_style_is_not_a_payload if {
	commands.executed_texts(`bash -c 'echo "hi"'`) == {
		`bash -c 'echo  hi '`,
		`echo "hi"`,
	}
}

# The false positive the nesting check exists for: a commit message DESCRIBING
# the bypass names `bash -c '...'`, and splitting on `'` alone finds a span whose
# preceding text ends in a wrapper. It is literal text inside the double-quoted
# message, so it must not become an executed text.
test_wrapper_named_inside_a_double_quoted_message_is_not_a_payload if {
	text := concat("", [`git commit -m "the bypass form was bash -c '`, push_main, `'"`])
	count(commands.executed_texts(text)) == 1
}

# ... and symmetrically, a double-quoted wrapper example inside a single-quoted
# argument is text too.
test_wrapper_named_inside_a_single_quoted_argument_is_not_a_payload if {
	text := concat("", [`git commit -m 'the bypass form was bash -c "`, push_main, `"'`])
	count(commands.executed_texts(text)) == 1
}

# A payload keeps its own quoted operands rather than losing them to the split.
test_payload_keeps_its_inner_quoted_operand if {
	concat("", ["git push origin ", `"main"`]) in commands.executed_texts(`bash -c 'git push origin "main"'`)
}

# An apostrophe inside a double-quoted body must not desynchronise the parity
# check and drop the whole command back to its raw form.
test_apostrophe_in_a_quoted_body_does_not_force_the_raw_fallback if {
	text := concat("", [`bd remember --key k "it's about`, "\n", push_main, `"`])
	commands.executed_texts(text) == {concat("", [`bd remember --key k "it s about `, push_main, `"`])}
}

# --- quoted operand spans lose their command positions ------------------------

test_newline_inside_quotes_is_neutralised if {
	text := concat("", [`bd remember --key k "note`, "\n", push_main, "\n", `end"`])
	commands.executed_texts(text) == {concat("", [`bd remember --key k "note `, push_main, ` end"`])}
}

test_separator_inside_quotes_is_neutralised if {
	text := concat("", [`echo "first; `, push_main, `"`])
	commands.executed_texts(text) == {concat("", [`echo "first  `, push_main, `"`])}
}

# ... while a separator OUTSIDE quotes keeps working.
test_separator_outside_quotes_survives if {
	commands.executed_texts(concat("", ["echo hi; ", push_main])) == {concat("", ["echo hi; ", push_main])}
}

# The operand itself is preserved, not deleted: `git -C "<path>" push` must still
# parse for the worktree exception.
test_quoted_operand_content_is_preserved if {
	commands.executed_texts(`git -C "/path with space" push`) == {`git -C "/path with space" push`}
}

# --- heredoc bodies are data, unless a shell reads them ------------------------

test_heredoc_body_is_neutralised if {
	text := concat("", ["cat > docs/x.md <<'EOF'\nfirst; ", push_main, "\nEOF"])
	not regex.match(`(?m)^git push`, single_text(commands.executed_texts(text)))
}

test_shell_read_heredoc_body_keeps_its_separators if {
	text := concat("", ["bash <<'EOF'\n", push_main, "\nEOF"])
	regex.match(`(?m)^git push`, single_text(commands.executed_texts(text)))
}

single_text(texts) := t if {
	count(texts) == 1
	some t in texts
}

# --- unreadable payloads are reported, never silently dropped -----------------

test_unquoted_payload_is_unparsed if {
	commands.unparsed_shell_payload("bash -c $CMD")
}

test_eval_with_unquoted_payload_is_unparsed if {
	commands.unparsed_shell_payload("eval $x")
}

test_quoted_payload_is_not_unparsed if {
	not commands.unparsed_shell_payload(concat("", ["bash -c '", push_main, "'"]))
}

test_substituted_but_quoted_payload_is_not_unparsed if {
	not commands.unparsed_shell_payload(`bash -c "$CMD"`)
}

test_plain_command_is_not_unparsed if {
	not commands.unparsed_shell_payload(push_main)
}

# A wrapper mentioned only inside quoted prose is not an unreadable payload.
test_wrapper_named_in_quoted_prose_is_not_unparsed if {
	not commands.unparsed_shell_payload(`git commit -m "avoid bash -c foo in guards"`)
}

test_unbalanced_quotes_near_a_wrapper_are_unparsed if {
	commands.unparsed_shell_payload(concat("", [`echo 'it"s' ; bash -c "`, push_main, `"`]))
}

# --- fail-closed fallbacks keep the RAW text ----------------------------------

# Command substitution executes even inside double quotes, so a text containing
# `$(` is left un-neutralised rather than being trusted as an operand.
test_command_substitution_leaves_text_raw if {
	text := concat("", [`echo "$(`, push_main, `)"`])
	commands.executed_texts(text) == {text}
}

test_backtick_leaves_text_raw if {
	text := concat("", ["echo \"`", push_main, "`\""])
	commands.executed_texts(text) == {text}
}

test_unbalanced_quotes_leave_text_raw if {
	text := concat("", [`echo "unclosed `, push_main])
	commands.executed_texts(text) == {text}
}

# --- quote-scrubbed view, for unanchored substring tests ----------------------

test_unquoted_view_drops_quoted_spans if {
	texts := commands.executed_unquoted_texts(concat("", [`echo "`, push_main, `"`]))
	every t in texts {
		not contains(t, "push")
	}
}

test_unquoted_view_keeps_wrapper_payloads if {
	push_main in commands.executed_unquoted_texts(concat("", ["bash -c '", push_main, "'"]))
}

# --- COMMAND POSITION ---------------------------------------------------------
#
# has_command_verb answers "is this word the program a shell would run", which is
# what a destructive-verb test actually means. Assembled from parts, like
# push_main above, so editing this file through a Bash command does not hand the
# guards a literal root delete to read.

root_delete := concat(" ", ["rm", "-rf", "/"])

test_command_verb_at_start_of_text if {
	commands.has_command_verb(root_delete, "rm")
}

test_command_verb_after_semicolon if {
	commands.has_command_verb(concat("", ["echo hi; ", root_delete]), "rm")
}

test_command_verb_after_and_and if {
	commands.has_command_verb(concat("", ["echo hi && ", root_delete]), "rm")
}

test_command_verb_after_pipe if {
	commands.has_command_verb(concat("", ["echo hi | ", root_delete]), "rm")
}

test_command_verb_after_newline if {
	commands.has_command_verb(concat("", ["echo hi\n", root_delete]), "rm")
}

test_command_verb_inside_command_substitution if {
	commands.has_command_verb(concat("", ["echo $(", root_delete, ")"]), "rm")
}

# Wrappers must be followed, or tightening would lose real invocations.

test_command_verb_behind_sudo if {
	commands.has_command_verb(concat(" ", ["sudo", root_delete]), "rm")
}

test_command_verb_behind_sudo_with_options if {
	commands.has_command_verb(concat(" ", ["sudo", "-u", "root", root_delete]), "rm")
}

test_command_verb_behind_env_assignment if {
	commands.has_command_verb(concat(" ", ["env", "FOO=1", root_delete]), "rm")
}

test_command_verb_behind_bare_assignment if {
	commands.has_command_verb(concat(" ", ["TMPDIR=/x", root_delete]), "rm")
}

test_command_verb_behind_nohup if {
	commands.has_command_verb(concat(" ", ["nohup", root_delete]), "rm")
}

test_command_verb_behind_xargs_with_flag if {
	commands.has_command_verb(concat(" ", ["xargs", "-0", root_delete]), "rm")
}

# `timeout` is the wrapper with a MANDATORY operand, and a bare duration is neither option-shaped
# nor an assignment -- so before 2026-09-21 nothing here consumed it and the verb behind it was in
# nobody's command position. Every guard that reads has_command_verb was blind to these.
test_command_verb_behind_timeout if {
	commands.has_command_verb(concat(" ", ["timeout", "5", root_delete]), "rm")
}

test_command_verb_behind_timeout_with_unit_suffix if {
	commands.has_command_verb(concat(" ", ["timeout", "5s", root_delete]), "rm")
}

test_command_verb_behind_timeout_with_signal_flag if {
	commands.has_command_verb(concat(" ", ["timeout", "-s", "KILL", "30", root_delete]), "rm")
}

# PINNED OVER-MATCH, not an accident. The wrapper alternative consumes up to four arbitrary
# tokens, so a verb-shaped OPERAND four words behind a wrapper reads as command position:
# `timeout 5 ls /tmp/rm` answers true for "rm", exactly as `sudo -u root ls /tmp/rm` already did.
# That is the direction commands.rego chose on purpose -- the alternatives "only ever make the
# verb test match MORE commands, never fewer" -- because a deny guard that over-matches asks a
# question, while one that under-matches allows the thing it exists to stop. Written down here so
# the next reader knows it was decided rather than missed.
test_timeout_inherits_the_wrapper_over_match if {
	commands.has_command_verb("timeout 5 ls /tmp/rm", "rm")
	commands.has_command_verb("sudo -u root ls /tmp/rm", "rm")
}

test_command_verb_after_then if {
	commands.has_command_verb(concat("", ["if true; then ", root_delete, "; fi"]), "rm")
}

# The whole point: prose is not a program. Written in LOWER CASE deliberately --
# callers fold case before testing verbs, and a capitalised "Install" would make
# these pass on the case mismatch instead of on command position, which is not
# the property under test.

test_prose_word_in_rust_doc_comment_is_not_a_command_verb if {
	not commands.has_command_verb("cat > src/lib.rs <<'EOF' /// install the detour EOF", "install")
}

test_prose_word_in_line_comment_is_not_a_command_verb if {
	not commands.has_command_verb("cat > src/lib.rs <<'EOF' // install the thing EOF", "install")
}

test_prose_word_mid_sentence_is_not_a_command_verb if {
	not commands.has_command_verb("echo the installer will install things", "install")
}

test_word_as_an_operand_is_not_a_command_verb if {
	not commands.has_command_verb("git commit -m install notes", "install")
}

# has_verb, by contrast, matches all of those -- which is exactly the difference.
test_has_verb_still_matches_prose_so_the_two_are_not_interchangeable if {
	commands.has_verb("echo the installer will install things", "install")
}

# --- ABSOLUTE/RELATIVE PATH INVOCATION (2026-08-31, bd er-effects-rs-5z75) ----
#
# command_position_prefix_pattern's anchor class is `(^|[;&|(){}\n])` -- a
# leading path component (`/bin/`, `/usr/bin/`, `./`) is none of those
# characters, and the character immediately before the verb is `/`, so before
# this fix the anchor never lined up and `/bin/rm -rf .cupcake` was allowed
# outright by CLAUDE-GUARD-LAYER-DESTRUCTIVE and BUILTIN-GIT-BLOCK-NO-VERIFY
# alike (measured against the live policies, not just this helper).

abs_root_delete := concat(" ", ["/bin/rm", "-rf", "/"])

test_command_verb_behind_absolute_path_bin if {
	commands.has_command_verb(abs_root_delete, "rm")
}

test_command_verb_behind_absolute_path_usr_bin if {
	commands.has_command_verb(concat(" ", ["/usr/bin/rm", "-rf", "/"]), "rm")
}

test_command_verb_behind_relative_path_dot_slash if {
	commands.has_command_verb(concat(" ", ["./rm", "-rf", "/"]), "rm")
}

test_command_verb_behind_nested_relative_path if {
	commands.has_command_verb(concat(" ", ["../scripts/rm", "-rf", "/"]), "rm")
}

# Wrappers and an absolute-path verb compose: tightening one gap cannot re-open
# the other.
test_command_verb_behind_sudo_and_absolute_path if {
	commands.has_command_verb(concat(" ", ["sudo", abs_root_delete]), "rm")
}

# After a real separator too, not just at the start of the text.
test_command_verb_behind_absolute_path_after_semicolon if {
	commands.has_command_verb(concat("", ["echo hi; ", abs_root_delete]), "rm")
}

# THE NEGATIVE HALF, and the reason the group requires a trailing slash rather
# than being a bare optional prefix: a word merely ENDING in the verb must not
# match just because there happens to be a slash earlier in the same operand,
# and a verb-shaped suffix with no preceding path component at all must not
# match either.

test_word_ending_in_verb_with_path_prefix_is_not_a_command_verb if {
	not commands.has_command_verb("scripts/confirm foo", "rm")
}

test_word_ending_in_verb_with_no_path_prefix_is_not_a_command_verb if {
	not commands.has_command_verb("xmv foo bar", "mv")
}

# A slash-terminated substring deep inside an unrelated operand must not
# manufacture a match either: there is no real anchor (string start or a
# separator character) immediately before it, only a plain space, which
# command_position_prefix_pattern does not treat as an anchor on its own.
test_path_operand_mid_command_is_not_a_command_verb if {
	not commands.has_command_verb("cp foo /path/to/rm-similar-file", "rm")
	not commands.has_command_verb("chmod +x /path/to/rm", "rm")
}

# --- shell segmentation (2026-08-31, bd er-effects-rs-c0t9) -------------------
#
# shell_segments moved here from guard_layer_destructive_guard.rego so the
# vendored git_block_no_verify builtin could close the same co-presence defect
# without a second copy. shell_statements is the coarser cut it is now built on:
# statements only, pipelines kept whole, because a pipe carries data between the
# commands it joins and a co-presence guard has to see that.

test_statements_split_on_semicolon if {
	commands.shell_statements("a x; b y") == {"a x", "b y"}
}

test_statements_split_on_and_and if {
	commands.shell_statements("a x && b y") == {"a x", "b y"}
}

test_statements_split_on_or_or if {
	commands.shell_statements("a x || b y") == {"a x", "b y"}
}

test_statements_split_on_background_and_newline if {
	commands.shell_statements("a x &\nb y") == {"a x", "b y"}
}

# THE DISTINCTION THIS TYPE EXISTS FOR. `||` is consumed whole before the single
# characters, so it cannot leave a stray `|` behind that would read as a pipe --
# and a real pipe keeps its two stages together.
test_statements_keep_a_pipeline_whole if {
	commands.shell_statements("echo p | xargs rm -f") == {"echo p | xargs rm -f"}
}

test_segments_cut_a_pipeline if {
	commands.shell_segments("echo p | xargs rm -f") == {"echo p", "xargs rm -f"}
}

# shell_segments was a single-pass `|`/`&`/`;` replacement before it was factored
# through shell_statements. Set-equality on a command carrying every separator at
# once is what pins the refactor as behaviour-preserving.
test_segments_match_the_single_pass_form if {
	cmd := "a 1 | b 2 && c 3 || d 4; e 5 & f 6"
	commands.shell_segments(cmd) == {"a 1", "b 2", "c 3", "d 4", "e 5", "f 6"}
}

test_segments_drop_empty_pieces if {
	commands.shell_segments(";; a x ;;") == {"a x"}
}

# The input contract: callers pass executed_texts output, where a separator
# inside a quoted operand has already been blanked. Under that contract a quoted
# `;` does not split -- which matters, because splitting in the wrong place
# separates a verb from its operand and that is the fail-OPEN direction.
test_quoted_separator_does_not_split_under_the_contract if {
	some text in commands.executed_texts(`rm -rf "a;b"`)
	commands.shell_statements(text) == {`rm -rf "a b"`}
}
