package cupcake.policies.claude.ds2_run_preset_guard_test

import data.cupcake.policies.claude.ds2_run_preset_guard
import rego.v1

old_script := "/home/banon/projects/ds2-mods-rs/.claude/worktrees/feat-music-playlist-probe/scripts/ds2-run.py"

new_script := "/home/banon/projects/ds2-mods-rs/scripts/ds2-run.py"

bash(cmd, signal) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Bash",
	"tool_input": {"command": cmd},
	"signals": {"ds2_run_presets": signal},
}

# --- the launch that wiped the presets ------------------------------------

test_old_script_launch_is_denied if {
	cmd := sprintf("python3 %s", [old_script])
	sig := sprintf("CHECKED\nlaunch\t%s\tpresets=0", [old_script])
	count(ds2_run_preset_guard.deny) == 1 with input as bash(cmd, sig)
}

test_old_script_after_cd_is_denied if {
	cmd := "cd /home/banon/projects/ds2-mods-rs/.claude/worktrees/feat-music-playlist-probe && python3 scripts/ds2-run.py"
	sig := sprintf("CHECKED\nlaunch\t%s\tpresets=0", [old_script])
	count(ds2_run_preset_guard.deny) == 1 with input as bash(cmd, sig)
}

test_deny_message_says_merge_origin_main if {
	cmd := sprintf("python3 %s", [old_script])
	sig := sprintf("CHECKED\nlaunch\t%s\tpresets=0", [old_script])
	some d in ds2_run_preset_guard.deny with input as bash(cmd, sig)
	contains(d.reason, "Merge origin/main into that branch first")
	contains(d.reason, old_script)
}

test_output_object_signal_is_read if {
	cmd := sprintf("python3 %s", [old_script])
	sig := {"output": sprintf("CHECKED\nlaunch\t%s\tpresets=0", [old_script])}
	count(ds2_run_preset_guard.deny) == 1 with input as bash(cmd, sig)
}

test_one_stale_launch_among_two_is_denied if {
	cmd := sprintf("python3 %s; python3 %s", [new_script, old_script])
	sig := sprintf("CHECKED\nlaunch\t%s\tpresets=1\nlaunch\t%s\tpresets=0", [new_script, old_script])
	count(ds2_run_preset_guard.deny) == 1 with input as bash(cmd, sig)
}

# --- the launcher that keeps them -----------------------------------------

test_new_script_launch_is_allowed if {
	cmd := sprintf("python3 %s", [new_script])
	sig := sprintf("CHECKED\nlaunch\t%s\tpresets=1", [new_script])
	count(ds2_run_preset_guard.deny) == 0 with input as bash(cmd, sig)
}

# --- nothing staged: the signal emits no launch line ----------------------

test_old_script_dry_run_is_allowed if {
	cmd := sprintf("python3 %s --dry-run", [old_script])
	count(ds2_run_preset_guard.deny) == 0 with input as bash(cmd, "CHECKED")
}

test_old_script_selftest_is_allowed if {
	cmd := sprintf("python3 %s --selftest", [old_script])
	count(ds2_run_preset_guard.deny) == 0 with input as bash(cmd, "CHECKED")
}

test_unrelated_command_is_allowed if {
	count(ds2_run_preset_guard.deny) == 0 with input as bash("cargo build -p ds2-loader", "")
}

test_grep_of_old_script_with_silent_signal_is_allowed if {
	cmd := sprintf("grep -n SECOND_SIN %s", [old_script])
	count(ds2_run_preset_guard.deny) == 0 with input as bash(cmd, "")
}

# --- fail closed -----------------------------------------------------------

test_unresolvable_launch_is_denied if {
	sig := "CHECKED\nunknown\t$W/scripts/ds2-run.py\tthe path is built from a variable or substitution"
	count(ds2_run_preset_guard.deny) == 1 with input as bash("python3 $W/scripts/ds2-run.py", sig)
}

test_silent_signal_on_a_launch_is_denied if {
	count(ds2_run_preset_guard.deny) == 1 with input as bash("python3 scripts/ds2-run.py", "")
}

test_silent_signal_on_a_launch_after_cd_is_denied if {
	cmd := "cd /x && ./scripts/ds2-run.py --tear-down"
	count(ds2_run_preset_guard.deny) == 1 with input as bash(cmd, "")
}

test_silent_signal_on_a_dry_run_is_allowed if {
	count(ds2_run_preset_guard.deny) == 0 with input as bash("python3 scripts/ds2-run.py --dry-run", "")
}
