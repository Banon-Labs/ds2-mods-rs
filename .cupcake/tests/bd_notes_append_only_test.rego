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

test_allows_quoted_mention_with_apostrophe if allowed(`git commit -m "don't use bd update x --notes"`)

test_allows_single_quoted_mention_with_double_quote if allowed(`git commit -m 'never "bd update x --notes"'`)

# A lone `"` inside a single-quoted span used to hide everything after it (commands.quotes_removed
# deletes double-quoted spans first, with no parity check).
test_denies_after_double_quote_inside_single_quotes if denied(`echo 'see "x'; bd update a-1 --notes junk`)

# Neither quote style balances: scanned raw, so it can only deny more.
test_denies_unbalanced_quotes if denied(`echo "a'b; bd update a-1 --notes junk`)

# --- a variable or quoted command word is still bd (2026-11-02: `$B update X --notes` ran unblocked) ---

test_denies_variable_head if denied(`B=$HOME/.local/bin/bd; $B update ds2-mods-rs-169u.3 --notes "clean text"`)

test_denies_braced_variable_head if denied(`${B} update ds2-mods-rs-169u.3 --notes "x"`)

test_denies_defaulted_variable_head if denied(`${BD:-bd} update ds2-mods-rs-169u.3 --notes "x"`)

test_denies_quoted_variable_head if denied(`B=$HOME/.local/bin/bd; "$B" update ds2-mods-rs-169u.3 --notes "x"`)

test_denies_quoted_path_head if denied(`"$HOME/.local/bin/bd" update ds2-mods-rs-169u.3 --notes "x"`)

test_denies_quoted_head_after_chain if denied(`cd /x && "$B" update ds2-mods-rs-169u.3 --notes "x"`)

test_denies_variable_head_in_shell_wrapper if denied(`bash -c '$B update ds2-mods-rs-169u.3 --notes x'`)

test_allows_variable_head_append_notes if allowed(`B=$HOME/.local/bin/bd; $B update ds2-mods-rs-169u.3 --append-notes "finding"`)

test_allows_quoted_variable_head_append_notes if allowed(`"$B" update ds2-mods-rs-169u.3 --append-notes "finding"`)

test_allows_quoted_head_not_in_command_position if allowed(`echo "$B" update x --notes`)

# --- an intended replacement: the marker line names the same issue, in the one accepted shape ---

test_allows_marked_replacement if allowed(`bd update ds2-mods-rs-169u.3 --notes "[notes rebuilt from bd history ds2-mods-rs-169u.3]
STATIC RE of 1000[14] / 1000[17]."`)

test_allows_marked_replacement_from_file if allowed(`$HOME/.local/bin/bd update ds2-mods-rs-169u.3 --notes "[notes rebuilt from bd history ds2-mods-rs-169u.3]
$(cat /path/to/clean.txt)"`)

test_allows_marked_replacement_with_equals if allowed(`bd update ds2-mods-rs-169u.3 --notes="[notes rebuilt from bd history ds2-mods-rs-169u.3] x"`)

test_allows_marked_replacement_variable_head if allowed(`B=$HOME/.local/bin/bd; $B update ds2-mods-rs-169u.3 --notes '[notes rebuilt from bd history ds2-mods-rs-169u.3] x'`)

test_allows_marked_replacement_in_shell_wrapper if allowed(`bash -c 'bd update ds2-mods-rs-169u.3 --notes "[notes rebuilt from bd history ds2-mods-rs-169u.3] x"'`)

test_denies_marker_naming_another_issue if denied(`bd update ds2-mods-rs-169u.3 --notes "[notes rebuilt from bd history ds2-mods-rs-169u.2] x"`)

test_denies_marker_not_at_start_of_value if denied(`bd update ds2-mods-rs-169u.3 --notes "x [notes rebuilt from bd history ds2-mods-rs-169u.3]"`)

test_denies_marker_with_flags_between if denied(`bd update ds2-mods-rs-169u.3 --status open --notes "[notes rebuilt from bd history ds2-mods-rs-169u.3] x"`)

test_denies_marked_then_unmarked_replacement if denied(`bd update a-1 --notes "[notes rebuilt from bd history a-1] x" && bd update b-2 --notes "y"`)

test_denies_marker_quoted_in_another_command if denied(`echo 'bd update a-1 --notes "[notes rebuilt from bd history a-1]'; bd update a-1 --notes junk`)

test_denies_second_notes_flag_on_marked_invocation if denied(`bd update a-1 --notes "[notes rebuilt from bd history a-1] x" --notes "junk"`)

test_denies_paraphrased_marker if denied(`bd update a-1 --notes "Rebuilt from bd history a-1: x"`)
