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
