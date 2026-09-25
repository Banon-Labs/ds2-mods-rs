# OPA unit tests for teardown_must_relaunch.
#
# Not loaded by the cupcake engine (which scans .cupcake/policies/<harness>/
# and .cupcake/system/ only). Run with:
#   opa test .cupcake/policies/claude/teardown_must_relaunch.rego \
#            .cupcake/tests/teardown_must_relaunch_test.rego
# End-to-end engine coverage lives in scripts/test-cupcake-policies.py.
package cupcake.policies.claude.teardown_must_relaunch_test

import rego.v1

import data.cupcake.policies.claude.teardown_must_relaunch as guard

RULE := "DS2-MODS-TEARDOWN-MUST-RELAUNCH"

bash_event(cmd) := {
	"hook_event_name": "PreToolUse",
	"tool_name": "Bash",
	"tool_input": {"command": cmd, "description": "test case"},
}

rule_ids(denials) := {d.rule_id | some d in denials}

denied(cmd) if {
	denials := guard.deny with input as bash_event(cmd)
	RULE in rule_ids(denials)
}

# --- a teardown with no relaunch is DENIED -----------------------------------

# The shape that killed an er-mods-rs run on 2026-09-12, with the DS2 script names.
test_deny_teardown_stapled_to_a_build if {
	denied("python3 scripts/ds2-teardown.py > /dev/null 2>&1; cargo xwin build --release -p ds2-loader")
}

test_deny_teardown_with_absolute_path_and_trailing_work if {
	denied("python3 /home/x/repo/scripts/ds2-teardown.py && cargo build")
}

# Reporting what was torn down is not relaunching it.
test_deny_teardown_then_log_read if {
	denied("python3 scripts/ds2-teardown.py; tail -5 run.log")
}

# A dry run stages and launches nothing, so it does not discharge the rule.
test_deny_teardown_with_dry_run_launch if {
	denied("python3 scripts/ds2-teardown.py; python3 scripts/ds2-run.py --dry-run --all-menu-rows")
}

# --- a teardown that is the whole command is ALLOWED --------------------------

# Ending a run on purpose. The sibling guard refuses a source edit while a run is
# live, so this has to be expressible or the two rules deadlock each other.
test_allow_bare_teardown_alone if {
	not denied("python3 scripts/ds2-teardown.py")
}

test_allow_teardown_alone_with_redirects if {
	not denied("python3 scripts/ds2-teardown.py > /dev/null 2>&1")
}

# ds2-teardown.py has no `--reason` flag (er's does), so a bare word after it is trailing work.
test_deny_teardown_with_a_trailing_word if {
	denied("python3 scripts/ds2-teardown.py measured")
}

# ...but riding along with other work is still the accident this rule exists for.
test_deny_teardown_then_second_command if {
	denied("python3 scripts/ds2-teardown.py > /dev/null 2>&1; cargo xwin build --release -p ds2-loader")
}

# --- a teardown that relaunches is ALLOWED -----------------------------------

test_allow_teardown_then_launch if {
	not denied("python3 scripts/ds2-teardown.py > /dev/null 2>&1; python3 scripts/ds2-run.py --all-menu-rows")
}

test_allow_teardown_and_launch_with_absolute_paths if {
	not denied("python3 /home/x/repo/scripts/ds2-teardown.py; python3 /home/x/repo/scripts/ds2-run.py --all-menu-rows")
}

# ds2-run.py tears down on its own, so the launch alone is the ordinary form and names no teardown.
test_allow_launch_alone if {
	not denied("python3 /home/banon/projects/ds2-mods-rs/scripts/ds2-run.py --all-menu-rows")
}

# Written across lines, which the live engine collapses before evaluation.
test_allow_multiline_teardown_and_launch if {
	not denied("python3 scripts/ds2-teardown.py \\\n  && python3 scripts/ds2-run.py --all-menu-rows")
}

# --- read-only status is always allowed --------------------------------------

test_allow_status if {
	not denied("python3 scripts/ds2-teardown.py --status")
}

test_allow_status_piped if {
	not denied("python3 scripts/ds2-teardown.py --status 2>&1 | tail -3")
}

# `--dry-run` lists what would be signalled and signals nothing.
test_allow_teardown_dry_run_with_other_work if {
	not denied("python3 scripts/ds2-teardown.py --dry-run; cargo xwin build --release -p ds2-loader")
}

# --- the guard stays out of everything else ----------------------------------

# A similarly-named script is not the teardown.
test_allow_lookalike_script if {
	not denied("python3 scripts/ds2-teardown-report.py")
}

test_allow_unrelated_command if {
	not denied("cargo xwin build --release -p ds2-loader")
}

test_allow_read_only_python_extraction_without_teardown if {
	not denied(`python3 - <<'PY'
from pathlib import Path
ranges = [
 ('driver.rs', Path('crates/er-npc-possess/src/possess/driver.rs'), [(240,310),(1125,1245),(1645,1815)]),
 ('game.rs', Path('crates/er-npc-possess/src/possess/game.rs'), [(760,845),(1088,1125),(1228,1255),(1436,1504)]),
 ('netdamage.rs', Path('crates/er-npc-possess/src/possess/netdamage.rs'), [(1,118),(330,445),(470,560)]),
 ('layout.rs', Path('crates/er-npc-possess/src/possess/layout.rs'), [(68,140),(470,505),(1160,1195),(1218,1260)]),
]
for title,path,spans in ranges:
 print('\\n##', title)
 lines=path.read_text().splitlines()
 for a,b in spans:
  print(f'-- {a}-{b}')
  for i in range(a,min(b,len(lines))+1):
   s=lines[i-1]
   if any(tok in s for tok in ['INVINC','teamType','request_move','SetHP','Packet15','Receive','set_invincible','set_alpha','set_no_attack','set_body_scale','set_camera_override','players_in_world','last_received_damage_packet','packet15_receive','TEAM_TYPE','TINT_ALPHA','SCALE_SIZE','camOverride','remote player','mode','Incoming','PvP DAMAGE','request_move','body is stretched','hurtbox','collision','PlayerIns','ChrIns','co-locate','camera', 'Charmed', 'SetStamina']):
    print(f'{i}: {s}')
PY`)
}

test_allow_other_events if {
	denials := guard.deny with input as {
		"hook_event_name": "PostToolUse",
		"tool_name": "Bash",
		"tool_input": {"command": "python3 scripts/ds2-teardown.py"},
	}
	not RULE in rule_ids(denials)
}

# --- a payload that NAMES the script is not an invocation of it ---------------
#
# Both shapes below were refused in production on 2026-09-21 (bd er-effects-rs-ak3q,
# bd er-effects-rs-if5l). The invoked binary is `git` and `bd`; the script appears in a
# commit message and an issue body, where it is prose. The cost was not the block: the
# commit message had to be reworded to say "a repo-relative teardown command" rather than
# name the file, and the issue reporting it had to be filed through a body file written by
# a separate command, so the record this guard is not there to police came out degraded.

test_allow_commit_message_naming_the_script if {
	not denied(`git commit -m "fix(guard): teardown_must_relaunch fires on scripts/ds2-teardown.py in prose"`)
}

test_allow_commit_heredoc_naming_the_script if {
	not denied(`git commit -F - <<'EOF'
fix(guard): decide on the command, not the text

scripts/ds2-teardown.py named in a commit message is documentation, and
python3 scripts/ds2-teardown.py; cargo build in one is still documentation.
EOF`)
}

test_allow_issue_body_naming_the_script if {
	not denied(`bd create --description "scripts/ds2-teardown.py must be paired with a launch; see python3 scripts/ds2-teardown.py; bash scripts/er-build-dlls.sh"`)
}

test_allow_memory_body_naming_the_script if {
	not denied(`bd remember --key teardown-guard-note "the shape that was refused was python3 scripts/ds2-teardown.py > /dev/null 2>&1; cargo xwin build --release -p ds2-loader"`)
}

# ...and the guard still reaches a real invocation inside a shell wrapper payload, which
# is the half a naive "is the first word git" test would lose.
test_deny_teardown_inside_a_shell_wrapper_payload if {
	denied(`bash -c 'python3 scripts/ds2-teardown.py; cargo build'`)
}

# A `--status` earlier in the command does not excuse a real kill later in it. The
# two-rule shape this replaced asked only whether SOME teardown was followed by
# `--status`, so this chained past it.
test_deny_status_then_a_real_teardown if {
	denied("python3 scripts/ds2-teardown.py --status; python3 scripts/ds2-teardown.py; cargo build")
}
