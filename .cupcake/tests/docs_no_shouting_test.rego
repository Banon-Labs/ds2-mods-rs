# OPA unit tests for docs_no_shouting (the PreToolUse refusal of documentation that puts its
# emphasis in the capitalisation).
#
# Run with:
#   opa test .cupcake/policies/claude/docs_no_shouting.rego \
#            .cupcake/tests/docs_no_shouting_test.rego
#
# The allow-cases are the load-bearing half, and there are deliberately more of them. A rule that
# only refuses is a rule nobody can work under, and this one lives in a repo whose prose is made of
# acronyms, register names, hex and mangled symbols. Every allow-case below is a real shape out of
# this codebase that has to keep being writable.
package cupcake.policies.claude.docs_no_shouting_test

import rego.v1

import data.cupcake.policies.claude.docs_no_shouting as guard

edit_event(path, text) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Edit",
	"tool_input": {"file_path": path, "old_string": "x", "new_string": text},
}

write_event(path, text) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Write",
	"tool_input": {"file_path": path, "content": text},
}

multiedit_event(path, texts) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "MultiEdit",
	"tool_input": {
		"file_path": path,
		"edits": [{"old_string": "x", "new_string": t} | some t in texts],
	},
}

rule_ids(denials) := {d.rule_id | some d in denials}

denied(event) if {
	denials := guard.deny with input as event
	"DS2-MODS-DOCS-NO-SHOUTING" in rule_ids(denials)
}

# --- refuse: the four shapes the user named -------------------------------------------------------

# A one-word shout. The run rule cannot see this one at all -- it is a single word -- and the
# function-word list is the only reason it is caught.
test_deny_one_word_absolute if {
	denied(edit_event("crates/ds2-save-file/src/lib.rs", "//! NOTHING in ds2-save-file has been run."))
}

# A shouted clause in the middle of a sentence, which is the commonest form of the habit here.
test_deny_shouted_clause_in_a_sentence if {
	denied(write_event("docs/DS2-RUNTIME.md", "THE GAME IS INVISIBLE TO pgrep, because it runs under Wine."))
}

# A shouted heading. Five words, so the run rule reaches it even before the word list does.
test_deny_shouted_heading if {
	denied(edit_event("crates/ds2-hook/src/lib.rs", "/// # WHAT THIS DOES NOT CLAIM"))
}

# A hyphenated one-word shout. The hyphen splits the run, so this is caught by `ONLY` alone --
# which is the case that proves the word list has to be checked per word rather than per run.
test_deny_hyphenated_one_word_shout if {
	denied(edit_event("crates/ds2-loader/src/lib.rs", "//! STARTUP-ONLY, both of them."))
}

# Four ordinary content words in a row, with no function word anywhere in them. This is the case
# the run rule exists for, and the only shape in the whole repo that the word list cannot see.
test_deny_four_content_words_with_no_function_word if {
	denied(edit_event("crates/ds2-rva/src/lib.rs", "/// ONE ROW PITCH PER ADDED ROW."))
}

# --- allow: identifiers, which must never be mistaken for shouting --------------------------------

# An underscore is a word character, so a screaming-snake-case constant is one token to the matcher
# and cannot start a run. This is why the policy needs no allow-list of identifiers.
test_allow_screaming_snake_case_constants if {
	not denied(edit_event(
		"crates/ds2-save-redirect/src/lib.rs",
		"/// Writes under SAVE_DIR_BUILD, sized by FE_INGAME_MENU_ITEM_VECTOR_CAPACITY.",
	))
}

# A digit ends the word, so a game file name and a Ghidra placeholder are not capitalised words.
test_allow_file_names_and_ghidra_symbols if {
	not denied(edit_event(
		"crates/ds2-sl2-core/src/lib.rs",
		"/// Reads DS2SOFS0000.sl2 through FUN_1402e67f0, entered from DllMain.",
	))
}

# Mixed case is not capitals. The Windows entry point must stay writable in prose.
test_allow_mixed_case_symbol if {
	not denied(write_event("docs/DS2-LOADER.md", "DllMain runs on the loader lock."))
}

# --- allow: acronyms and names in ordinary sentences ----------------------------------------------

# The vocabulary this repo is made of. None of these is on the function-word list, and a run needs
# four in a row with nothing between them.
test_allow_acronyms_in_a_sentence if {
	not denied(write_event(
		"docs/DS2-BUILD.md",
		"The DLL is built for MSVC and its RVA table is checked against the PE headers.",
	))
}

test_allow_more_acronyms_in_a_sentence if {
	not denied(edit_event(
		"crates/ds2-save-file/src/crypt.rs",
		"/// The save is AES in CBC mode, keyed per PID, with an MD5 over the BND4 container.",
	))
}

test_allow_tooling_acronyms if {
	not denied(write_event(
		"docs/DS2-TOOLING.md",
		"Config is TOML, the manifest is JSON, the text is UTF-8, and CI runs on every PR. That is OK on any CPU the API supports.",
	))
}

# A comma between two acronyms ends the run, which is what keeps a list of section tags legal.
test_allow_a_comma_separated_list_of_acronyms if {
	not denied(edit_event("crates/ds2-sl2-core/src/lib.rs", "/// Sections: PE, COFF, IAT, TLS, PEB."))
}

# --- allow: numbers, hex and sentence-initial capitals --------------------------------------------

test_allow_hex_constants_and_numbers if {
	not denied(edit_event(
		"crates/ds2-rva/src/lib.rs",
		"/// The flag sits at 0xDEADBEEF and the stride is 0x58, measured over 12 rows.",
	))
}

# One capitalised word at the start of a sentence is how English works. `Only` here is not `ONLY`.
test_allow_sentence_initial_capitals if {
	not denied(write_event("README.md", "Only the loader touches the game. Nothing else does."))
}

# A lone acronym opening a sentence is the same case from the other side.
test_allow_a_lone_acronym_opening_a_sentence if {
	not denied(edit_event("crates/ds2-loader/src/lib.rs", "/// DLL search order is fixed by the loader."))
}

# --- allow: verbatim spans, which are somebody else's words ---------------------------------------

# Backticks. A shouted phrase inside them is a quoted string, not the author raising their voice.
test_allow_shouting_inside_backticks if {
	not denied(edit_event("crates/ds2-dialog-skip/src/menu.rs", "/// The row reads `PRESS ANY BUTTON` until a save exists."))
}

# A fenced block, which spans lines and so is removed before the text is cut into lines at all.
test_allow_shouting_inside_a_fenced_block if {
	not denied(write_event(
		"docs/DS2-LOG.md",
		"The log looks like this:\n\n```\nTHE GAME IS INVISIBLE TO pgrep\nNOTHING WAS ATTACHED\n```\n\nThat is the whole record.",
	))
}

# Quotation marks. This is what keeps the game's full title legal -- it is written in quotes both
# times it appears here -- and it is also what stops the guard punishing a quoted user directive.
test_allow_shouting_inside_quotation_marks if {
	not denied(edit_event(
		"crates/ds2-rva/src/lib.rs",
		"/// The scene animates the \"DARK SOULS II: SCHOLAR OF THE FIRST SIN\" text in.",
	))
}

# Removing a code span must not weld the words on either side of it into a run that was never
# written. Two capitalised words, a backticked identifier, then two more.
test_allow_a_removed_span_does_not_weld_two_runs if {
	not denied(edit_event("crates/ds2-hook/src/lib.rs", "/// LOAD GAME `FeSubStateTitleLoadDataList` NEW GAME."))
}

# --- allow: text outside the documentation scope --------------------------------------------------

# An ordinary `//` comment is a note beside the code, not prose anybody skims. Same cut
# docs_no_size_metrics makes.
test_allow_shouting_in_an_ordinary_comment if {
	not denied(edit_event("crates/ds2-loader/src/lib.rs", "// THE GAME IS INVISIBLE TO pgrep"))
}

# A log line the code emits is a measurement being reported, not documentation.
test_allow_shouting_in_a_string_literal if {
	not denied(edit_event(
		"crates/ds2-game-base/src/log.rs",
		"    log::warn!(\"NOTHING WAS ATTACHED AND THE PROBE NEVER RAN\");",
	))
}

# A file that is neither Markdown nor Rust carries no documentation scope at all.
test_allow_other_file_types if {
	not denied(write_event("scripts/ds2-stage.sh", "echo 'THE GAME IS INVISIBLE TO pgrep'"))
}

# --- the three tool shapes ------------------------------------------------------------------------
# A rule that reads only one of them is a rule with a documented way around it.

test_deny_via_write if {
	denied(write_event("docs/DS2-NOTES.md", "WHAT THIS DOES NOT CLAIM"))
}

test_deny_via_edit if {
	denied(edit_event("docs/DS2-NOTES.md", "WHAT THIS DOES NOT CLAIM"))
}

test_deny_via_multiedit_second_edit if {
	denied(multiedit_event("docs/DS2-NOTES.md", ["a clean first edit", "WHAT THIS DOES NOT CLAIM"]))
}

test_allow_multiedit_when_every_edit_is_clean if {
	not denied(multiedit_event("docs/DS2-NOTES.md", ["The DLL is fine.", "So is the RVA table."]))
}

# --- the shape of the refusal ---------------------------------------------------------------------

# The offending line is quoted back, so the author knows which sentence to rewrite rather than
# being told their file shouts somewhere.
test_reason_quotes_the_offending_line if {
	denials := guard.deny with input as edit_event("crates/ds2-hook/src/lib.rs", "/// # WHAT THIS DOES NOT CLAIM")
	some d in denials
	contains(d.reason, "WHAT THIS DOES NOT CLAIM")
	contains(d.reason, "crates/ds2-hook/src/lib.rs")
}

# The correction has to say what to do instead, in the terms the user asked for: move the emphasis
# into the structure of the sentence. Without that it reads as "write quieter", which is not the
# instruction and is not actionable.
test_reason_names_the_alternative if {
	denials := guard.deny with input as edit_event("crates/ds2-hook/src/lib.rs", "/// # WHAT THIS DOES NOT CLAIM")
	some d in denials
	contains(d.reason, "structure")
	contains(d.reason, "own short paragraph")
}

# And it must say that a name in capitals was never the offence, because that is the half a reader
# is most likely to over-generalise from -- and over-generalising it would make this repo, which is
# made of acronyms and register names, unwritable.
test_reason_exempts_names_explicitly if {
	denials := guard.deny with input as edit_event("crates/ds2-hook/src/lib.rs", "/// # WHAT THIS DOES NOT CLAIM")
	some d in denials
	contains(d.reason, "backticks")
}

test_deny_is_medium_severity if {
	denials := guard.deny with input as edit_event("crates/ds2-hook/src/lib.rs", "/// # WHAT THIS DOES NOT CLAIM")
	some d in denials
	d.severity == "MEDIUM"
}

# --- events and tools this rule has no business judging -------------------------------------------

test_no_deny_on_a_non_authoring_tool if {
	denials := guard.deny with input as {
		"hook_event_name": "PreToolUse",
		"tool_name": "Bash",
		"tool_input": {"command": "echo WHAT THIS DOES NOT CLAIM"},
	}
	count(denials) == 0
}

test_no_deny_on_a_non_pretooluse_event if {
	denials := guard.deny with input as {
		"hook_event_name": "PostToolUse",
		"tool_name": "Edit",
		"tool_input": {"file_path": "docs/x.md", "new_string": "WHAT THIS DOES NOT CLAIM"},
	}
	count(denials) == 0
}

test_no_deny_on_an_empty_edit if {
	denials := guard.deny with input as edit_event("docs/x.md", "")
	count(denials) == 0
}
