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
#     required_signals: ["tmp_copy_sources"]
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
#
# A DESTINATION BEHIND A SHELL VARIABLE (2026-11-02). Every pattern used to demand a literal
# `/tmp/` at the head of the path, so `S=/tmp/x; cat > $S/probe.py <<EOF` sailed through while the
# same write spelled out was denied. Measured through scripts/cupcake-check-command.py: the literal
# `cp scripts/frida/summon-sign-trace.js /tmp/.../scratchpad/frozen.js` DENIED, the
# `S=/tmp/.../scratchpad; cp ... $S/frozen.js` spelling ALLOWED. So the path group also accepts a
# leading `$NAME/` or `${NAME}/`, and tmp_path_resolutions below substitutes every value that NAME
# is assigned a /tmp path in the same command. A variable the command never assigns resolves to
# nothing: the guard cannot know where `$CLAUDE_PROJECT_DIR` or an inherited `$TMPDIR` points, and
# refusing every `$X/y.py` would deny ordinary in-repo work.
#
# Quote characters may sit inside the path (`"$S"/x.py`, `> "/tmp/x.py"`); they are shell syntax,
# not part of the file name, and are stripped before the path is judged.
path_head := `["']?(?:/tmp/|\$\{?[A-Za-z_][A-Za-z0-9_]*\}?["']?/)`

path_body := `[^\s\\;&|<>]*`

path_group := concat("", ["(", path_head, path_body, ")"])

quoted_path_group := `((?:/tmp/|\$\{?[A-Za-z_][A-Za-z0-9_]*\}?/)[^'"]+)`

copy_verb_group := `(?:^|[\s;&|(])(cp|mv|install|rsync|scp|ln)\s+((?:[^\s;&|<>]+\s+)+?)`

copy_detail_pattern := concat("", [`(?m)`, copy_verb_group, path_group, `\s*(?:$|[;&|)])`])

write_indicator_patterns := [
	# Shell redirection INTO the path: `> /tmp/x.py`, `>> /tmp/x.py`, `2>/tmp/x.py`, `&>/tmp/x.py`,
	# `>| /tmp/x.py`. This is also what covers a heredoc / `echo` / `printf` redirected into it.
	concat("", [`>>?\|?\s*`, path_group]),
	# `| tee /tmp/x.sh`, `tee -a /tmp/x.sh`.
	concat("", [`(?:^|[\s;&|(])tee(?:\s+-{1,2}[^\s]+)*\s+`, path_group]),
	# Download/output flags: `curl -o /tmp/x.js`, `curl --output /tmp/x.js`, `wget -O /tmp/x.js`.
	concat("", [`(?:^|[\s;&|(])(?:-o|-O|--output|--output-document)[=\s]*`, path_group]),
	# Copy/move/install DESTINATION. The /tmp path must be the LAST argument of its command
	# segment, which is what separates `cp a /tmp/x.py` (writes into /tmp) from
	# `cp /tmp/x.py a` and `cp -f /tmp/planner/*.js target/` (both read OUT of /tmp).
	concat("", [`(?m)(?:^|[\s;&|(])(?:cp|mv|install|rsync|scp|ln)\s+(?:[^\s;&|<>]+\s+)+?`, path_group, `\s*(?:$|[;&|)])`]),
	# In-language writes inside a heredoc / -c snippet: `open('/tmp/x.py', 'w')`. The mode must be
	# the second positional argument and must contain w/a/x, so a READ -- `open('/tmp/a.js')`, or
	# `open('/tmp/a.js', encoding='utf-8')` -- does not match.
	concat("", [`open\(\s*['"]`, quoted_path_group, `['"]\s*,\s*['"][^'"]*[wax][^'"]*['"]`]),
	# `Path('/tmp/x.py').write_text(...)` / `.write_bytes(...)`.
	concat("", [`['"]`, quoted_path_group, `['"]\s*\)\s*\.\s*write`]),
]

# The index of the copy pattern above, which the repo-copy carve-out forgives.
copy_pattern_index := 3

tmp_written_script_paths contains path if {
	# NOT regex.find_n: Cupcake's WASM runtime has no host implementation for it, so the call
	# returned undefined and this entire Bash-authoring branch silently never fired (2026-08-22).
	# regex.find_all_string_submatch_n is compiled into the WASM module itself, and m[1] is the
	# path captured by the write-indicator pattern's single group.
	some index, pattern in write_indicator_patterns
	some m in regex.find_all_string_submatch_n(pattern, command, -1)
	some path in tmp_path_resolutions(m[1])
	is_script_path(path)
	not in_current_repo(path)
	not only_extracted(index, m[1])
	not only_copied_from_repo(index, m[1])
}

strip_quotes(raw) := replace(replace(raw, `"`, ""), "'", "")

# Every /tmp path a captured spelling can name. A literal names itself; `$NAME/rest` names
# `<value>/rest` for each /tmp value NAME is assigned in this command (an assignment made twice
# yields both, and either being a script is enough to deny).
tmp_path_resolutions(raw) := {path |
	spelled := strip_quotes(raw)
	startswith(spelled, "/tmp/")
	path := spelled
} | {path |
	spelled := strip_quotes(raw)
	some m in regex.find_all_string_submatch_n(`^\$\{?([A-Za-z_][A-Za-z0-9_]*)\}?(/.*)$`, spelled, 1)
	some value in tmp_variable_values[m[1]]
	path := concat("", [trim_right(value, "/"), m[2]])
}

# NAME -> {/tmp values} for every `NAME=/tmp/...` (optionally `export`/`local`/`declare`d, quoted
# or not) anywhere in the command.
tmp_variable_assignment_pattern := `(?:^|[\s;&|(])(?:(?:export|local|declare|readonly)\s+)?([A-Za-z_][A-Za-z0-9_]*)=["']?(/tmp(?:/[^\s"';&|<>)]*)?)`

tmp_variable_values[name] := values if {
	some m in regex.find_all_string_submatch_n(tmp_variable_assignment_pattern, command, -1)
	name := m[1]
	values := {v |
		some n in regex.find_all_string_submatch_n(tmp_variable_assignment_pattern, command, -1)
		n[1] == name
		v := n[2]
	}
}

# A COPY OF A TRACKED REPO SCRIPT IS NOT AUTHORING (2026-11-02). `cp
# scripts/frida/summon-sign-trace.js <scratchpad>/summon-sign-trace-frozen.js` publishes an edit to
# a hot-reloading `scripts/ds2-frida-watch.py --agent <frozen file>`. The bytes live in a tracked
# repo file -- reviewable, versioned, exactly where this policy's own refusal says scripts belong --
# and the /tmp copy is the artifact. It was denied.
#
# Forgiven only for `cp` and `install`, only when EVERY source operand of that copy is reported by
# the tmp_copy_sources signal as a tracked file under the repo's scripts/ tree (realpath checked,
# so a link out of the tree does not count), and only when every copy pattern match for the path
# qualifies. `-t`/`--target-directory` turns the operand order around, so it never qualifies, and
# a command that also runs `ln` could repoint a source after the signal looked, so it does not
# either. A path that is ALSO redirected, tee'd or python-written stays denied through that other
# pattern's index.
only_copied_from_repo(index, raw) if {
	index == copy_pattern_index
	not regex.match(`(?:^|[\s;&|(])ln\s`, command)
	all_copies := [m | some m in regex.find_all_string_submatch_n(write_indicator_patterns[copy_pattern_index], command, -1); m[1] == raw]
	repo_copies := [m |
		some m in regex.find_all_string_submatch_n(copy_detail_pattern, command, -1)
		m[3] == raw
		m[1] in {"cp", "install"}
		copy_sources_tracked(m[2])
	]
	count(repo_copies) > 0
	count(repo_copies) == count(all_copies)
}

# Flags whose value is the next token. `-t`/`--target-directory` are refused outright below.
copy_value_flags := {"-m", "--mode", "-o", "--owner", "-g", "--group", "-S", "--suffix"}

copy_sources_tracked(args) if {
	tokens := [t | some t in split(replace(replace(trim_space(args), "\t", " "), "\n", " "), " "); t != ""]
	not copy_names_target_directory(tokens)
	sources := [strip_quotes(t) |
		some i, t in tokens
		not startswith(t, "-")
		not follows_value_flag(tokens, i)
	]
	count(sources) > 0
	every source in sources {
		source in tracked_copy_sources
	}
}

copy_names_target_directory(tokens) if {
	some t in tokens
	regex.match(`^(?:-[A-Za-z]*t[A-Za-z]*|--target-directory(?:=.*)?)$`, t)
}

follows_value_flag(tokens, i) if {
	i > 0
	tokens[i - 1] in copy_value_flags
}

tracked_copy_sources contains token if {
	some line in split(tmp_copy_sources_text, "\n")
	parts := split(line, "\t")
	count(parts) == 2
	parts[0] == "tracked"
	token := parts[1]
}

tmp_copy_sources_text := value if {
	value := input.signals.tmp_copy_sources
	is_string(value)
} else := value if {
	value := input.signals.tmp_copy_sources.output
	is_string(value)
} else := ""

# A READ-ONLY EXTRACT is data, whatever its extension. Measured 2026-09-24, rebasing PR39/42/40:
# `sed -n ... > $SCRATCH/ours.rs` saved one side of a merge conflict to compare, and was denied as
# "authoring a script" because the content is Rust and so the file was named .rs. Nothing was
# authored -- every byte already existed in a tracked file -- and the refusal's own text allows
# data into /tmp. The pivot (rename to .txt) was cheap; the rule mistaking a copy for a script was
# the defect.
#
# The carve-out is narrow on purpose. The writer must be `sed -n`, `git show`, `head` or `tail`,
# standing at the START of its command (a `|` in front of it means something else produced the
# bytes -- `echo 'code' | sed -n p > /tmp/x.py` is still authoring), with no UNQUOTED `<` between
# it and the redirect, so a heredoc cannot ride along. A quoted one is fine and is the common case:
# the sed address that extracts a conflict side is `'/<<<<<<< /,/=======/p'`. And it only forgives the redirect pattern
# (index 0): a path that is ALSO written some other way, or redirected into more times than the
# extracts account for, is still denied.
extract_redirect_pattern := concat("", [
	`(?:^|[;&(\n])\s*(?:sed\s+-n|git\s+(?:-C\s+[^\s;&|<>]+\s+)?show|head|tail)\s(?:[^;&|<>\n'"]|'[^'\n]*'|"[^"\n]*")*>\s*`,
	path_group,
])

only_extracted(0, path) if {
	redirects := [m | some m in regex.find_all_string_submatch_n(write_indicator_patterns[0], command, -1); m[1] == path]
	extracts := [m | some m in regex.find_all_string_submatch_n(extract_redirect_pattern, command, -1); m[1] == path]
	count(extracts) > 0
	count(extracts) == count(redirects)
}
