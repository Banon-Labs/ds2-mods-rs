# OPA unit tests for script_comments_no_shouting (the PreToolUse refusal of a shouted `#` comment in
# a Python or shell script).
#
# Run with:
#   opa test .cupcake/policies/claude/script_comments_no_shouting.rego \
#            .cupcake/tests/script_comments_no_shouting_test.rego
package cupcake.policies.claude.script_comments_no_shouting_test

import rego.v1

import data.cupcake.policies.claude.script_comments_no_shouting as guard

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

denied(event) if {
	denials := guard.deny with input as event
	"DS2-MODS-SCRIPT-COMMENTS-NO-SHOUTING" in {d.rule_id | some d in denials}
}

# --- refuse ---------------------------------------------------------------------------------------

# A clause shouted in the middle of a Python comment, the commonest shape in scripts/.
test_deny_shouted_clause_in_python_comment if {
	denied(edit_event("scripts/ds2-run.py", "    # A LINE IS EVIDENCE ONLY ONCE IT IS TERMINATED. A read can land mid-write."))
}

# The same in a shell script, written whole.
test_deny_shouted_shell_comment if {
	denied(write_event(".cupcake/signals/example.sh", "#!/usr/bin/env bash\n# The way out is NOT to hedge.\necho ok\n"))
}

# A one-word shout in a `#:` attribute comment.
test_deny_one_word_shout_in_attribute_comment if {
	denied(edit_event("scripts/ds2-flo.py", "#: This file said the opposite, and NOTHING caught it."))
}

# Four content words in a row with no function word, which only the run pattern sees.
test_deny_run_of_four_content_words if {
	denied(edit_event("scripts/check.sh", "# ONE ROW PITCH PER ADDED ROW."))
}

# Every text MultiEdit would write is read, not only the first.
test_deny_second_multiedit_text if {
	denied(multiedit_event("scripts/ds2-sl2.py", ["# plain comment", "# SAME value, read TWICE."]))
}

# An absolute path from the harness reaches the guard as well as a relative one.
test_deny_absolute_path if {
	denied(edit_event("/home/banon/projects/ds2-mods-rs/scripts/ds2-ebl.py", "# THE list is incomplete."))
}

# --- allow ----------------------------------------------------------------------------------------

# A comment made of this repo's names: an environment variable, a screaming-snake constant, a rule
# id, an acronym and a backticked symbol.
test_allow_comment_full_of_names if {
	not denied(edit_event("scripts/ds2-teardown.py", "# SteamAppId and STEAM_COMPAT_APP_ID name the DLL; see DS2-MODS-NO-GREP-FOR-BUILD-ERRORS and `FUN_1402e67f0`."))
}

# A quoted status string or log line is somebody else's text.
test_allow_quoted_status_string if {
	not denied(edit_event("scripts/ds2-run.py", "# the loader prints \"NOT RUN\" when the probe is off"))
}

# Capitals in code, a string literal or a docstring are not a comment.
test_allow_capitals_outside_comments if {
	not denied(edit_event("scripts/ds2-run.py", "print(\"THE GAME IS NOT RUNNING\")\nMSG = 'NOT READY'\n\"\"\"NOTHING here is a comment.\"\"\""))
}

# A trailing comment after code is deliberately not read.
test_allow_trailing_comment_after_code if {
	not denied(edit_event("scripts/ds2-run.py", "x = 1  # NOT read, by design"))
}

# The shebang is not a comment.
test_allow_shebang if {
	not denied(write_event("scripts/example.sh", "#!/usr/bin/env bash\n# plain comment\n"))
}

# Rust and Markdown belong to docs_no_shouting; this guard stays out so one edit is not refused twice.
test_allow_rust_file_left_to_docs_no_shouting if {
	not denied(edit_event("crates/ds2-loader/src/lib.rs", "# NOT a script"))
}

# A Bash tool call is out of scope.
test_allow_bash_tool if {
	not denied({
		"hook_event_name": "PreToolUse",
		"tool_name": "Bash",
		"tool_input": {"command": "echo '# NOT a file'"},
	})
}
