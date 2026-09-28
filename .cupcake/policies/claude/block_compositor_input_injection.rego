# METADATA
# scope: package
# title: Refuse input injection that names no target window
# authors: ["er-mods-rs agents", "ds2-mods-rs agents"]
# custom:
#   severity: HIGH
#   id: DS2-MODS-BLOCK-UNTARGETED-INPUT
#   description: >-
#     Refuse wtype / ydotool, and xdotool input verbs without --window, from
#     agent Bash commands. These deliver to whatever window currently holds
#     focus, so on 2026-09-09 two F3 presses meant for Elden Ring landed in the
#     application the user was actually playing. Injection that names its
#     target window is allowed; read-only xdotool queries are untouched.
#   routing:
#     required_events: ["PreToolUse"]
#     required_tools: ["Bash"]
package cupcake.policies.claude.block_compositor_input_injection

import rego.v1

import data.cupcake.system.commands

# The defect is UNTARGETED input, not input.
#
# Measured 2026-09-09 with `scripts/frida/keystate-probe.js` while the game polled
# `GetAsyncKeyState` ~46 times a second and `GetForegroundWindow` was never null: `wtype -k F3`,
# `xdotool key F3` and `ydotool key 61:1 61:0` produced zero down events in Elden Ring. Two of
# them were delivered to an unrelated application instead, because a focus-addressed press goes
# wherever focus is -- and Elden Ring was mapped, fullscreen and unfocused on another workspace.
#
# "Check focus first, then press" is not the fix and must not be offered as one: `hyprctl
# activewindow` reported `steam_app_1245620` in the same minute `xdotool getwindowfocus` reported
# a different application, and focus can move between the check and the press regardless.
#
# So the rule is that the press must NAME what it is aimed at. Three forms do:
#   * `xdotool key --window <id> ...`, which addresses one X window;
#   * a Frida `Interceptor` on `user32!GetAsyncKeyState` returning 0x8001 for the vkey, which
#     addresses the process and drives the DLL's real poll, edge latch and handler;
#   * the input harness's `inputmgr+0x90+eventId` keystate write, for a native game binding.
#
# `wtype` and `ydotool` have no target parameter at all -- wtype speaks the Wayland
# virtual-keyboard protocol and ydotool writes a uinput device, and both land on the focused
# surface by construction -- so there is no targeted form of them to allow.
#
# See bd never-inject-keys-at-the-compositor-it-hits-the-focused-app-2026-09-09.

command := object.get(input.tool_input, "command", "")

# `wtype` and `ydotool` exist only to synthesise focus-addressed input, so the bare tool name is
# the token. `xdotool` also answers read-only questions, so only its input-generating verbs are
# named -- `xdotool search`, `getactivewindow` and `getwindowclassname` are how a window is
# identified in the first place and stay allowed.
#
# COMMAND POSITION ONLY (2026-09-27). The first version matched the token after ANY whitespace or
# quote, and refused a `python3 - <<'PY'` edit of a script whose body declared a Python field
# `wtype: str | None = None` -- nothing was executed, the word was data. The engine's
# whitespace_normalization welds a heredoc body onto the command reading it, so "after a space"
# is true of every word in the body.
#
# So this reads the command the way the other program-name guards here do, through the shared
# helpers in .cupcake/system/commands.rego:
#   * commands.executed_texts yields every shell text the command runs -- the command itself with
#     quoted operands and non-shell heredoc bodies neutralised, plus the payload of any
#     `bash -c '...'` / `eval '...'` as a text of its own; a heredoc a SHELL reads stays raw;
#   * commands.command_position_prefix_pattern anchors the tool at text start or after `;`, `&&`,
#     `||`, `|`, `&`, `(`, a newline, optionally behind `sudo`, `env`, `exec`, `command`, `nohup`,
#     `timeout`, their options, and `VAR=value` assignments;
#   * commands.path_prefix_pattern covers `/usr/bin/<tool>` and `./<tool>`.
# A program a shell executes is in command position by definition, so nothing that runs is lost.
# That also retires the old bd / git-commit text exemptions: quoted text is neutralised for every
# command now, not only for those two.
#
# The trailing class is the shared one, so a script whose file name merely begins with the tool's
# name (`<tool>-notes.sh`) is not an invocation of it.
tool_end := `([ \t\n;&|(){}]|$)`

untargeted_tool_pattern := concat("", [
	commands.command_position_prefix_pattern,
	commands.path_prefix_pattern,
	`(wtype|ydotool)`,
	tool_end,
])

xdotool_input_pattern := concat("", [
	commands.command_position_prefix_pattern,
	commands.path_prefix_pattern,
	`xdotool[ \t]+(key|keydown|keyup|type|click|mousedown|mouseup|mousemove|mousemove_relative)`,
	tool_end,
])

executed := commands.executed_texts(command)

injection_detected if {
	some text in executed
	regex.match(untargeted_tool_pattern, text)
}

injection_detected if {
	some text in executed
	regex.match(xdotool_input_pattern, text)
	not regex.match(`--window([[:space:]]|=)`, command)
}

block_reason := concat("", [
	"This press names no target window, so it goes wherever focus is. On 2026-09-09 that put two ",
	"F3 presses into an unrelated application while Elden Ring sat unfocused, and a probe measured ",
	"ZERO of them reaching the game. Checking focus first does not fix it: hyprctl and the X ",
	"server disagreed about focus in the same minute, and focus can move between the check and ",
	"the press. Name the target instead: `xdotool key --window <id> ...` for an X window, a Frida ",
	"Interceptor on user32!GetAsyncKeyState returning 0x8001 for the vkey (see ",
	"scripts/frida/f3-toggle-drive.js) for an OS-level hotkey, or the input harness's ",
	"inputmgr+0x90 keystate write for a native game binding. wtype and ydotool have no target ",
	"parameter at all. Read-only xdotool queries are not blocked.",
])

deny contains decision if {
	input.hook_event_name == "PreToolUse"
	input.tool_name == "Bash"
	injection_detected

	decision := {
		"rule_id": "DS2-MODS-BLOCK-UNTARGETED-INPUT",
		"severity": "HIGH",
		"reason": concat("", [block_reason, "\n\nSource: ", command]),
	}
}
