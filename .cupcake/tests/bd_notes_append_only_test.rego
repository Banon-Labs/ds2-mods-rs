package cupcake.policies.claude.bd_notes_append_only_test

import rego.v1

import data.cupcake.policies.claude.bd_notes_append_only as guard

bash(command) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Bash",
	"tool_input": {"command": command},
}

denied(command) if {
	count(guard.deny) > 0 with input as bash(command)
}

allowed(command) if {
	count(guard.deny) == 0 with input as bash(command)
}

# --- the shape that overwrote notes on 2026-09-26 ---

test_denies_update_notes_by_path if denied(`$HOME/.local/bin/bd update ds2-mods-rs-lmd --notes "Branch soul-memory-guard ..."`)

test_denies_bare_bd_update_notes if denied(`bd update ds2-mods-rs-4yd --notes "PR #150"`)

test_denies_notes_with_equals if denied(`bd update ds2-mods-rs-4yd --notes="x"`)

test_denies_notes_after_other_flags if denied(`bd update ds2-mods-rs-4yd --status in_progress --notes "x"`)

test_denies_chained_after_another_command if denied(`git push && bd update x-1 --notes "y"`)

# --- the right spellings stay frictionless ---

test_allows_append_notes if allowed(`$HOME/.local/bin/bd update ds2-mods-rs-lmd --append-notes "finding"`)

test_allows_create_with_notes if allowed(`bd create "title" -t task --notes "first note"`)

test_allows_update_other_fields if allowed(`bd update ds2-mods-rs-4yd --status closed`)

test_allows_quoted_mention if allowed(`git commit -m "never use bd update x --notes"`)
