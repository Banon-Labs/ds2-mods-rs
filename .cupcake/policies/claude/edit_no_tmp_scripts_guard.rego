# METADATA
# scope: package
# title: No Authoring Scripts Into /tmp
# description: Author scripts/source in the repo (scripts/, scripts/ghidra/), not /tmp; /tmp is for artifacts only.
# custom:
#   severity: MEDIUM
#   id: DS2-MODS-NO-TMP-SCRIPTS-GUARD
#   routing:
#     required_events: ["PreToolUse"]
#     required_tools: ["Write", "Edit", "MultiEdit", "NotebookEdit", "Bash"]
package cupcake.policies.claude.edit_no_tmp_scripts_guard

import rego.v1

# Source/script extensions that should live in the repo (reviewable, version-controlled,
# reusable across sessions) -- never authored into the volatile /tmp tree. DATA artifacts
# (.tsv, .log, .json, .bin, .txt, .csv ...) are deliberately NOT listed: writing those to
# /tmp is fine and intended.
script_exts := {
	".py", ".sh", ".bash", ".rs", ".java", ".rego", ".js", ".ts", ".tsx", ".jsx",
	".go", ".c", ".cc", ".cpp", ".h", ".hpp", ".rb", ".pl", ".lua", ".ps1",
}

tool_input := object.get(input, "tool_input", {})
file_path := object.get(tool_input, "file_path", object.get(tool_input, "path", ""))
command := object.get(tool_input, "command", "")
lower_tool_name := lower(object.get(input, "tool_name", ""))

deny contains decision if {
	input.hook_event_name == "PreToolUse"
	authoring_tool
	in_tmp
	is_script_file

	decision := tmp_script_decision(file_path)
}

deny contains decision if {
	input.hook_event_name == "PreToolUse"
	bash_tool
	some path in tmp_written_script_paths

	decision := tmp_script_decision(path)
}

tmp_script_decision(path) := decision if {

	decision := {
		"rule_id": "DS2-MODS-NO-TMP-SCRIPTS-GUARD",
		"severity": "MEDIUM",
		"reason": concat("", [
			"🧁 Cupcake paused authoring a script into /tmp: ",
			path,
			"\n\nWhy this policy exists: /tmp is volatile (lost on reboot) and unversioned. Scripts authored there cannot be reviewed, reused across sessions, or shared with other agents. GhidraScripts repeatedly ended up stranded in /tmp/ghidra_scripts/.",
			"\n\nHappy path: author the script under the repo -- general helpers in `scripts/`, Ghidra postScripts in `scripts/ghidra/` -- and have IT write its data ARTIFACTS to /tmp or the session scratchpad if needed. (Data files like .tsv/.log/.json/.bin to /tmp are allowed; only source/scripts are blocked.)",
		]),
	}
}

authoring_tool if lower_tool_name == "write"

authoring_tool if lower_tool_name == "edit"

authoring_tool if lower_tool_name == "multiedit"

authoring_tool if lower_tool_name == "notebookedit"

authoring_tool if endswith(lower_tool_name, ".write")

authoring_tool if endswith(lower_tool_name, ".edit")

bash_tool if lower_tool_name == "bash"

bash_tool if lower_tool_name == "ctx_execute"

bash_tool if endswith(lower_tool_name, ".bash")

bash_tool if contains(lower_tool_name, "ctx_execute")

in_tmp if {
	startswith(file_path, "/tmp/")
	not in_current_repo(file_path)
}

in_current_repo(path) if {
	cwd := object.get(input, "cwd", "")
	cwd != ""
	cwd != "/"
	path == cwd
}

in_current_repo(path) if {
	cwd := object.get(input, "cwd", "")
	cwd != ""
	cwd != "/"
	startswith(path, concat("", [cwd, "/"]))
}

is_script_file if is_script_path(file_path)

is_script_path(path) if {
	some ext in script_exts
	endswith(path, ext)
}

# WRITE INDICATORS, EACH ANCHORED TO THE PATH ITSELF.
#
# The Write/Edit/MultiEdit/NotebookEdit branch above needs no such test -- those tools ALWAYS
# author the file they name. Bash does not, and the previous form of this branch could not tell
# the difference: it asked only whether an interpreter-ish word appeared ANYWHERE in the command
# and a /tmp script path appeared ANYWHERE else. Two measured false positives (2026-08-25):
#
#   python3 - <<'EOF' ... open(p) ... glob('/tmp/planner/*.js') ... EOF
#       -> READS third-party js out of /tmp. Denied because "python3" and a "/tmp/*.js" both
#          occurred in the same command.
#   mkdir -p target/planner-bundle && cp -f /tmp/planner/*.js target/planner-bundle/ && ls ...
#       -> COPIES OUT of /tmp, no interpreter at all. Denied because the alternation had no word
#          boundaries and "bun" matched inside the substring "planner-bundle", after which `.*`
#          reached the /tmp path. Same trap for cat/"concatenate", node/"node_modules",
#          tee/"committee", echo/"echoes".
#
# So the interpreter alternation is gone entirely -- an interpreter NAME says nothing about
# direction -- and each pattern below binds a real write operator DIRECTLY to the path it writes,
# capturing that path in group 1. Every command word is anchored on both sides ((^|[\s;&|(]) before,
# required whitespace after) so a substring inside an unrelated word can never arm the rule.
write_indicator_patterns := [
	# Shell redirection INTO the path: `> /tmp/x.py`, `>> /tmp/x.py`, `2>/tmp/x.py`, `&>/tmp/x.py`,
	# `>| /tmp/x.py`. This is also what covers a heredoc / `echo` / `printf` redirected into it.
	`>>?\|?\s*(/tmp/[^\s"'\\;&|<>]+)`,

	# `| tee /tmp/x.sh`, `tee -a /tmp/x.sh`.
	`(?:^|[\s;&|(])tee(?:\s+-{1,2}[^\s]+)*\s+(/tmp/[^\s"'\\;&|<>]+)`,

	# Download/output flags: `curl -o /tmp/x.js`, `curl --output /tmp/x.js`, `wget -O /tmp/x.js`.
	`(?:^|[\s;&|(])(?:-o|-O|--output|--output-document)[=\s]*(/tmp/[^\s"'\\;&|<>]+)`,

	# Copy/move/install DESTINATION. The /tmp path must be the LAST argument of its command
	# segment, which is what separates `cp a /tmp/x.py` (writes into /tmp) from
	# `cp /tmp/x.py a` and `cp -f /tmp/planner/*.js target/` (both read OUT of /tmp).
	`(?m)(?:^|[\s;&|(])(?:cp|mv|install|rsync|scp|ln)\s+(?:[^\s;&|<>]+\s+)+?(/tmp/[^\s"'\\;&|<>]+)\s*(?:$|[;&|)])`,

	# In-language writes inside a heredoc / -c snippet: `open('/tmp/x.py', 'w')`. The mode must be
	# the second positional argument and must contain w/a/x, so a READ -- `open('/tmp/a.js')`, or
	# `open('/tmp/a.js', encoding='utf-8')` -- does not match.
	`open\(\s*['"](/tmp/[^'"]+)['"]\s*,\s*['"][^'"]*[wax][^'"]*['"]`,

	# `Path('/tmp/x.py').write_text(...)` / `.write_bytes(...)`.
	`['"](/tmp/[^'"]+)['"]\s*\)\s*\.\s*write`,
]

tmp_written_script_paths contains path if {
	# NOT regex.find_n: Cupcake's WASM runtime has no host implementation for it, so the call
	# returned undefined and this entire Bash-authoring branch silently never fired (2026-08-22).
	# regex.find_all_string_submatch_n is compiled into the WASM module itself, and m[1] is the
	# path captured by the write-indicator pattern's single group.
	some pattern in write_indicator_patterns
	some m in regex.find_all_string_submatch_n(pattern, command, -1)
	path := m[1]
	is_script_path(path)
	not in_current_repo(path)
}
