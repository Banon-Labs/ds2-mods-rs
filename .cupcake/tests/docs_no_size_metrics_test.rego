# OPA unit tests for docs_no_size_metrics.
#
# Run with:
#   opa test .cupcake/policies/claude/docs_no_size_metrics.rego \
#            .cupcake/tests/docs_no_size_metrics_test.rego
#
# The allow-cases are the load-bearing half. A rule that only denies is a rule nobody can work
# under: every one of them is a real sentence from this repo's own documentation that must survive.
package cupcake.policies.claude.docs_no_size_metrics_test

import rego.v1

import data.cupcake.policies.claude.docs_no_size_metrics as guard

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
	"DS2-MODS-DOCS-NO-SIZE-METRICS" in rule_ids(denials)
}

# ---------------------------------------------------------------- line counts

test_deny_line_count_in_rust_doc_comment if {
	denied(edit_event("crates/ds2-save-file/src/lib.rs", "//! `er-quit-menu-core` is 29,000 lines."))
}

test_deny_line_count_in_markdown if {
	denied(write_event("docs/DS2-SAVE-FILE-ROWS.md", "The picker core is 7,839 lines."))
}

test_deny_hyphenated_line_count if {
	denied(edit_event("crates/ds2-menu-row/src/lib.rs", "/// A 234-line Windows shim."))
}

test_deny_approximate_line_count if {
	denied(write_event("README.md", "Roughly ~960 lines of tests live here."))
}

# ------------------------------------------------- a display's height is not a line count

# Measured 2026-09-30: the whole Edit this rule refused on the weapon picker's row budget, where
# "1080-line" is a screen 1080 pixel rows tall and the screen is named on the line before.
test_allow_vertical_resolution_the_rule_refused if {
	not denied(edit_event(
		"crates/ds2-build-recommender-ui/src/panel.rs",
		concat("\n", [
			"/// Rows the weapon picker shows at once until its first draw has measured the screen: the design's",
			"/// three, which fit a 1080-line one.",
			"const PICKER_ROWS: usize = 3;",
		]),
	))
}

# An Edit need not carry the line that named the screen.
test_allow_vertical_resolution_alone_on_its_line if {
	not denied(edit_event("crates/ds2-build-recommender-ui/src/panel.rs", "/// three, which fit a 1080-line one."))
}

test_allow_vertical_resolution_in_markdown if {
	not denied(write_event("docs/DS2-OVERLAY.md", "The picker keeps three rows on a 1440-line screen."))
}

test_allow_any_height_the_next_word_calls_a_display if {
	not denied(edit_event("crates/ds2-overlay/src/lib.rs", "/// Legible on a 1050-line monitor."))
}

test_allow_any_height_measured_as_tall if {
	not denied(edit_event("crates/ds2-overlay/src/lib.rs", "/// The panel is 600 lines tall at most."))
}

# The nearby true positive: the same number, on a line that names code.
test_deny_resolution_number_on_a_line_naming_a_module if {
	denied(edit_event("crates/ds2-overlay/src/lib.rs", "/// A 1080-line module."))
}

test_deny_resolution_number_beside_a_crate_name if {
	denied(edit_event("crates/ds2-overlay/src/lib.rs", "//! `ds2-overlay` is 1080 lines."))
}

test_deny_resolution_number_beside_a_file_name if {
	denied(edit_event("crates/ds2-overlay/src/lib.rs", "/// `panel.rs` is 1080 lines."))
}

test_deny_display_word_on_a_line_naming_a_driver if {
	denied(edit_event("crates/ds2-overlay/src/lib.rs", "/// A 600-line display driver."))
}

test_deny_hedged_resolution_number if {
	denied(edit_event("crates/ds2-overlay/src/lib.rs", "/// About 1080 lines of layout."))
}

test_deny_grouped_resolution_number if {
	denied(edit_event("crates/ds2-overlay/src/lib.rs", "/// The overlay is 1,080 lines."))
}

test_deny_line_count_beside_a_resolution if {
	denied(edit_event("crates/ds2-overlay/src/lib.rs", "/// Fits a 1080-line screen; the layout is 500 lines."))
}

# ---------------------------------------------------------------- file sizes

test_deny_megabyte_size if {
	denied(write_event("README.md", "The memory store is 4.6 MB on disk."))
}

test_deny_kilobyte_size if {
	denied(edit_event("crates/ds2-loader/src/lib.rs", "//! Only the first 8KB is scanned."))
}

test_deny_byte_size_of_an_artifact if {
	denied(write_event("docs/DS2-SAVE-FILE-ROWS.md", "The staged save is 8251680 bytes."))
}

# ---------------------------------------------------------------- test counts

test_deny_test_count if {
	denied(edit_event("crates/ds2-save-file-core/src/lib.rs", "//! Covered by 25 host tests."))
}

test_deny_check_count if {
	denied(write_event("README.md", "`--selftest` runs 158 checks."))
}

test_deny_qualified_test_count if {
	denied(edit_event("crates/ds2-loader/src/menu_row.rs", "//! 19 wine tests pin this."))
}

# ---------------------------------------------------------------- beads ids

test_deny_beads_id_in_doc_comment if {
	denied(edit_event("crates/ds2-save-file/src/lib.rs", "//! The way out is filed as ds2-mods-rs-v3f."))
}

test_deny_beads_id_in_markdown if {
	denied(write_event("docs/DS2-SAVE-FILE-ROWS.md", "See ds2-mods-rs-0r3 for the in-session load."))
}

test_deny_beads_id_inside_multiedit if {
	denied(multiedit_event("README.md", ["clean prose", "tracked in ds2-mods-rs-h4q"]))
}

# ------------------------------------------------- allow: behaviour, not bulk

test_allow_behavioural_prose if {
	not denied(edit_event(
		"crates/ds2-save-file/src/lib.rs",
		"//! The picker's menu chrome is Elden Ring's and does not port.",
	))
}

test_allow_counting_game_objects if {
	not denied(edit_event(
		"crates/ds2-menu-row/src/lib.rs",
		"//! The item vector holds 5 slots and the System tab ships 3 rows.",
	))
}

test_allow_counting_save_entries if {
	not denied(write_event("docs/DS2-SAVE-FILE-ROWS.md", "The container holds 23 entries."))
}

test_allow_frame_and_time_budgets if {
	not denied(edit_event(
		"crates/ds2-save-file/src/export.rs",
		"//! 300 menu frames is about five seconds at 60 fps.",
	))
}

test_allow_naming_lines_without_counting_them if {
	not denied(edit_event("crates/ds2-loader/src/lib.rs", "//! Read the line the DLL wrote."))
}

# ------------------------------------- allow: sizes a reader has to act on

test_allow_abi_size_in_a_plain_comment if {
	not denied(edit_event(
		"crates/ds2-save-file/src/dialog.rs",
		"// OPENFILENAMEW is 152 bytes on x64.",
	))
}

test_allow_abi_size_in_a_doc_comment_with_no_artifact_word if {
	not denied(edit_event(
		"crates/ds2-sl2-core/src/lib.rs",
		"/// A payload is a whole number of 16 bytes.",
	))
}

test_allow_hex_offsets if {
	not denied(edit_event(
		"crates/ds2-rva/src/lib.rs",
		"/// `[saveLoadSystem+0x08]` and `+0x0c`, both zero when idle.",
	))
}

# Measured 2026-09-27: a disassembly listing in a doc comment was read as "5 tests" -- the last
# digit of the hex address, then whitespace, then the x86 `test` mnemonic.
test_allow_disassembly_test_instruction_after_a_hex_address if {
	not denied(edit_event(
		"crates/ds2-rva/src/lib.rs",
		"/// 0x1401bf9a5  test byte [gm+0x24b2],0x4",
	))
}

test_allow_disassembly_test_instruction_on_registers if {
	not denied(edit_event("crates/ds2-rva/src/lib.rs", "/// +12  test eax,eax"))
}

test_allow_hex_digit_tail_before_the_word_tests if {
	not denied(edit_event("crates/ds2-rva/src/lib.rs", "/// see 0xa5 tests in the table at +0x10"))
}

test_deny_test_count_beside_a_hex_offset if {
	denied(edit_event("crates/ds2-rva/src/lib.rs", "/// `+0x24b2` is covered by 25 host tests."))
}

# --------------------------------------------- allow: everything outside docs

test_allow_line_count_in_rust_code if {
	not denied(edit_event("crates/ds2-loader/src/lib.rs", "let budget = 29_000; // lines"))
}

test_allow_line_count_in_a_python_script if {
	not denied(write_event("scripts/ds2-run.py", "# prints 158 checks"))
}

test_allow_beads_id_in_a_shell_script if {
	not denied(write_event("scripts/beads-prime.sh", "bd show ds2-mods-rs-v3f"))
}

test_allow_unrelated_tool if {
	not denied({
		"hook_event_name": "PreToolUse",
		"tool_name": "Bash",
		"tool_input": {"command": "wc -l crates/ds2-save-file/src/lib.rs"},
	})
}

# ---------------------------------------------------------------- shell writes into documentation

bash_event(command) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Bash",
	"tool_input": {"command": command},
}

issue_id := concat("", ["ds2-mods", "-rs-v3f"])

# Measured 2026-09-24: two ids went into docs/COMMENTS.md this way while the Edit path refused them.
test_deny_heredoc_append_into_docs if {
	denied(bash_event(concat("", ["cat >> docs/COMMENTS.md <<'EOF'\nTracked as ", issue_id, ".\nEOF"])))
}

# The delivered shape: the engine collapses the heredoc's newlines, and the shim separates the
# statement after the terminator.
test_deny_heredoc_append_into_docs_delivered_shape if {
	denied(bash_event(concat("", ["cat >> docs/COMMENTS.md <<'EOF' Tracked as ", issue_id, ". EOF; echo done"])))
}

test_deny_echo_into_a_markdown_file_anywhere if {
	denied(bash_event(concat("", ["echo 'see ", issue_id, "' > README.md"])))
}

test_deny_printf_piped_through_tee_into_docs if {
	denied(bash_event(concat("", ["printf '", "25 host ", "tests' | tee -a docs/PORTING.md"])))
}

# A write that is not documentation, and a read of documentation, stay out of it.
test_allow_id_redirected_into_a_non_doc_file if {
	not denied(bash_event(concat("", ["echo ", issue_id, " > /tmp/claude-1000/ids.txt"])))
}

test_allow_grep_of_docs_for_ids if {
	not denied(bash_event("grep -rn ds2-mods-rs- docs/"))
}

test_allow_clean_heredoc_into_docs if {
	not denied(bash_event("cat > docs/NEW.md <<'EOF'\nThe loader writes one log per run.\nEOF"))
}

# Measured on this rule's first draft: a doc path named inside a heredoc that writes a .rego file is
# not a target, and a separate statement's `timeout 25 opa test` is not a test count in a document.
test_allow_doc_path_inside_a_heredoc_written_elsewhere if {
	not denied(bash_event(concat("", ["cat >> tests/x.rego <<'REGO'\ndenied(bash_event(\"echo ", issue_id, " > README.md\"))\nREGO\ntimeout 25 opa test ."])))
}

test_allow_clean_doc_write_beside_a_counted_command if {
	not denied(bash_event("echo 'The loader writes one log per run.' > docs/NEW.md; timeout 25 opa test ."))
}
