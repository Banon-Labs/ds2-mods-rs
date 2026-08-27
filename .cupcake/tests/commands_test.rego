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
