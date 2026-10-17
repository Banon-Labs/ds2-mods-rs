package cupcake.policies.claude.ds2_launch_guard_test

import rego.v1

import data.cupcake.policies.claude.ds2_launch_guard

bash(command) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Bash",
	"tool_input": {"command": command},
}

denied(command) if {
	count(ds2_launch_guard.deny) > 0 with input as bash(command)
}

allowed(command) if {
	count(ds2_launch_guard.deny) == 0 with input as bash(command)
}

# --- the raw Steam launch this guard exists to stop --------------------------------------------

test_denies_bare_applaunch if denied("steam -applaunch 335300")

test_denies_applaunch_with_absolute_path if denied("/usr/bin/steam -applaunch 335300")

test_denies_applaunch_with_double_dash if denied("steam --applaunch 335300")

test_denies_applaunch_with_intervening_steam_options if denied("steam -silent -applaunch 335300")

test_denies_applaunch_chained_after_another_command if denied("cargo build && steam -applaunch 335300")

test_denies_run_url if denied("xdg-open steam://run/335300")

test_denies_rungameid_url if denied("steam steam://rungameid/335300")

# A wrapper must not launder the launch. This is the failure mode that made the
# executed-text decomposition necessary in the first place: the character before
# the verb is a quote, so a raw-string pattern sees nothing while the shell runs it.
test_denies_applaunch_inside_bash_c if denied("bash -c 'steam -applaunch 335300'")

test_denies_applaunch_inside_sh_c_double_quoted if denied("sh -c \"steam -applaunch 335300\"")

# --- fail closed on an unreadable payload -------------------------------------------------------

test_denies_opaque_payload_naming_steam_and_appid if denied("bash -c $STEAM_CMD # steam 335300")

# --- the sanctioned launcher must stay usable ---------------------------------------------------
# If any of these deny, the guard is fighting the workflow it exists to protect.

test_allows_the_testimony_gated_launcher if allowed("python3 scripts/ds2-run.py")

test_allows_launcher_with_probe_arm if allowed("python3 scripts/ds2-run.py --probe neuter --observe 30")

test_allows_launcher_dry_run if allowed("python3 scripts/ds2-run.py --dry-run")

# --- a DIFFERENT appid is somebody else's business ----------------------------------------------
# 1245620 is Elden Ring. This repo has no opinion about it, and a guard that
# grabs every applaunch would be reproducing the ER guard's overreach in mirror.

test_allows_applaunch_of_another_appid if allowed("steam -applaunch 1245620")

test_allows_appid_prefix_not_being_ours if allowed("steam -applaunch 3353001")

# --- static RE on the binary is the single most encouraged activity here ------------------------
# All of these name DarkSoulsII.exe. None of them launches it. Denying them would
# make the guard the enemy of the work.

test_allows_objdump_on_the_exe if allowed("objdump -d '/home/banon/.local/share/Steam/steamapps/common/Dark Souls II Scholar of the First Sin/Game/DarkSoulsII.exe'")

test_allows_strings_on_the_exe if allowed("strings -n 8 DarkSoulsII.exe | grep -i dlrf")

test_allows_sha256_of_the_exe if allowed("sha256sum DarkSoulsII.exe")

test_allows_checking_whether_the_game_is_running if allowed("pgrep -x DarkSoulsII.exe")

# --- but wine/proton actually running it is a launch --------------------------------------------

test_denies_wine_launching_the_exe if denied("wine DarkSoulsII.exe")

test_denies_proton_launching_the_exe if denied("proton run /path/to/DarkSoulsII.exe")

# --- text mentions are not launches -------------------------------------------------------------
# commands.executed_texts neutralises quoted operand spans, so filing an issue or
# writing a commit message ABOUT this rule is not an execution of it. A guard that
# cannot tell those apart makes it impossible to document itself.

test_allows_bd_issue_quoting_the_command if allowed("bd create --title=\"Launch guard\" --description=\"Deny a raw steam -applaunch 335300 that bypasses scripts/ds2-run.py\"")

test_allows_grepping_the_repo_for_the_appid if allowed("grep -rn 335300 scripts/")

# --- WRITING a launch option is not running one (2026-10-17) -------------------------------------
# Measured false positive, verbatim. The `bash -c '...'` is inside sed's double-quoted
# replacement: it is the VALUE of a Steam launch option being written into localconfig.vdf, and
# nothing in this command runs a shell on it. It was denied as an opaque payload because the
# escaped-quote scrub read `\\\\\"'` as `\\\` -- eating the real closing `'` -- so quote parity
# failed and the unbalanced-quote branch of unparsed_shell_payload fired.
localconfig_launch_option_edit := `F=~/.local/share/Steam/userdata/26191038/config/localconfig.vdf; N=$(grep -n -F 'WINEDLLOVERRIDES=\"dinput8=n,b\" %command%' $F | cut -d: -f1); echo "line $N"; [ "$(echo "$N" | wc -l)" = 1 ] && sed -n "$((N-5)),$((N))p" $F | grep -q '"335300"' && sed -i "${N}s|%command%\"\$|bash -c 'exec \\\\\"\${@/%DarkSoulsII.exe/ds2-launcher.exe}\\\\\"' _ %command%\"|" $F; sed -n "$((N-5)),$((N+1))p" $F`

test_allows_sed_writing_a_bash_c_launch_option_into_localconfig if allowed(localconfig_launch_option_edit)

# The same edit chained with a REAL launch is still a launch. The fix reads escapes the way a
# shell does; it does not exempt `sed -i` or anything else by name.
test_denies_localconfig_edit_followed_by_a_bare_applaunch if denied(concat(" && ", [localconfig_launch_option_edit, "steam -applaunch 335300"]))

test_denies_localconfig_edit_followed_by_a_bash_c_applaunch if denied(concat("; ", [localconfig_launch_option_edit, "bash -c 'steam -applaunch 335300'"]))

# Escaped backslashes before a real quote must not hide an executed payload either.
test_denies_bash_c_applaunch_after_escaped_backslashes if denied(`echo "a\\\\"; bash -c "steam -applaunch 335300"`)

test_denies_opaque_payload_after_escaped_backslashes if denied(`echo "\\\\"; bash -c $STEAM_CMD # steam 335300`)

# --- events this policy does not own -------------------------------------------------------------

test_ignores_non_bash_tools if {
	count(ds2_launch_guard.deny) == 0 with input as {
		"hook_event_name": "PreToolUse",
		"tool_name": "Read",
		"tool_input": {"file_path": "steam -applaunch 335300"},
	}
}

test_ignores_other_events if {
	count(ds2_launch_guard.deny) == 0 with input as {
		"hook_event_name": "PostToolUse",
		"tool_name": "Bash",
		"tool_input": {"command": "steam -applaunch 335300"},
	}
}
