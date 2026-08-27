package cupcake.policies.claude.block_pgrep_full_match_test

import rego.v1

import data.cupcake.policies.claude.block_pgrep_full_match as guard

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

# --- the two commands that actually caused damage in this repo -----------------------------------
# Both are verbatim from 2026-08-26: the first produced a fabricated "process is
# ALIVE" claim by matching the agent's own command line; the second killed the
# agent's own shell, exit 144.

test_denies_the_fabricated_alive_check if denied("pgrep -f 'DarkSoulsII.exe'")

test_denies_the_self_kill if denied("pkill -f 'ds2-run.py --probe neuter'")

# --- flag spellings that must not walk past --------------------------------------------------

test_denies_long_full_flag if denied("pgrep --full DarkSoulsII")

test_denies_pkill_long_full_flag if denied("pkill --full DarkSoulsII")

test_denies_combined_short_cluster_af if denied("pgrep -af DarkSoulsII")

test_denies_combined_short_cluster_fl if denied("pgrep -fl DarkSoulsII")

test_denies_flag_after_an_option_with_an_operand if denied("pgrep -u banon -f DarkSoulsII")

test_denies_absolute_path_invocation if denied("/usr/bin/pgrep -f DarkSoulsII")

test_denies_chained_after_another_command if denied("cargo build && pkill -f ds2-run")

# A quote is not a command boundary to a shell, so a wrapper must not launder it.
test_denies_inside_bash_c if denied("bash -c 'pgrep -f DarkSoulsII.exe'")

# --- fail closed on an unreadable payload ---------------------------------------------------

test_denies_opaque_payload_naming_the_tool if denied("bash -c $CHECK # pgrep")

# --- -x is the correct instrument and must stay frictionless ---------------------------------
# The whole point of this guard is to STEER to -x, not to ban the tool the way the
# sibling repo's WSL-premised guard does. If these deny, the guard is wrong.

test_allows_exact_name_match if allowed("pgrep -x DarkSoulsII.exe")

test_allows_bare_pgrep if allowed("pgrep DarkSoulsII")

test_allows_pgrep_with_count if allowed("pgrep -c -x steam")

test_allows_pkill_by_exact_name if allowed("pkill -x DarkSoulsII.exe")

test_allows_pgrep_with_signal_and_exact_name if allowed("pkill -TERM -x wineserver")

# -F is --pidfile, a completely different option. Case-sensitivity on the flag
# letter is load-bearing: denying -F would be a false positive on a command that
# cannot self-match at all.
test_allows_pidfile_flag if allowed("pkill -F /run/ds2.pid")

# --- neighbouring tools that legitimately take -f --------------------------------------------
# `grep -f patterns` and `tail -f` are unrelated. The tool-token anchor and the
# single-simple-command bound are what keep them out.

test_allows_grep_dash_f if allowed("grep -f patterns.txt haystack.txt")

test_allows_tail_dash_f if allowed("tail -f ds2-loader.log")

test_allows_pgrep_then_an_unrelated_grep_f if allowed("pgrep -x steam && grep -f patterns.txt haystack.txt")

# --- text mentions are not executions ---------------------------------------------------------

test_allows_bd_issue_quoting_the_flag if allowed("bd create --title=\"pgrep guard\" --description=\"pgrep -f matches the agent's own command line, so ban the flag and steer to -x\"")

# --- events this policy does not own -----------------------------------------------------------

test_ignores_non_bash_tools if {
	count(guard.deny) == 0 with input as {
		"hook_event_name": "PreToolUse",
		"tool_name": "Read",
		"tool_input": {"file_path": "pgrep -f something"},
	}
}

test_ignores_other_events if {
	count(guard.deny) == 0 with input as {
		"hook_event_name": "Stop",
		"tool_name": "Bash",
		"tool_input": {"command": "pgrep -f DarkSoulsII.exe"},
	}
}
