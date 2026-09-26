# METADATA
# scope: package
# title: No Shouting In Script Comments
# description: A comment in a Python or shell script puts its emphasis in the sentence, not in the capitalisation.
# authors: ["er-mods-rs agents", "ds2-mods-rs agents"]
# custom:
#   severity: MEDIUM
#   id: DS2-MODS-SCRIPT-COMMENTS-NO-SHOUTING
#   routing:
#     required_events: ["PreToolUse"]
#     required_tools: ["Write", "Edit", "MultiEdit", "NotebookEdit"]
package cupcake.policies.claude.script_comments_no_shouting

import rego.v1

# Where this came from, and what changed on the way in.
#
# er-mods-rs's edit_no_comment_caps_guard refuses a shouted word written into a comment in `.rs`,
# `.py`, `.sh` and `.bash` files. This repo already had docs_no_shouting for the `.rs` half and for
# Markdown, measured against this tree, and the port left er's guard behind on that ground. What
# that left uncovered is the other three extensions: `scripts/` and `.cupcake/signals/` hold most of
# this repo's reasoning in `#` comments, and nothing refused a shout there.
#
# So this is er's file scope with this repo's definition of the offence. er's word list was tuned
# to a tree that had been swept to zero and was held there by a gate (`check-comment-caps.py`)
# that this repo does not have; docs_no_shouting's two patterns were measured here.
#
# The patterns are copied, not imported. An import was the first version and scripts/check.sh
# refused it: the gate runs each test against its own policy plus commands.rego and nothing else,
# so that one package's rules cannot satisfy another's assertions, and a cross-package reference is
# undefined there. A copy is how this repo already keeps one definition in several places:
# scripts/test-shouting-signal.py compares the patterns in this file, in docs_no_shouting and in
# scripts/cupcake_shouting.py byte for byte, so they cannot drift apart unnoticed.
#
# Measured before porting, 2026-09-26: the emphasis pattern over the `#` comment lines of the
# git-tracked `.py`, `.sh` and `.bash` files, with backticked and quoted spans removed, and a
# sample of the hits read by hand. Every sampled hit was the habit -- clauses shouted mid-comment,
# shouted section headings -- and no new false-positive shape turned up beside the ones
# docs_no_shouting already handles.
#
# Like docs_no_shouting, this reads only the text the call would write, so an edit whose new text
# carries an existing shouted line forward is refused too. That is the same trade the Rust half
# already makes in an unswept tree: the fix is to lowercase the line that is in hand.
#
# Scope: whole-line `#` comments. A trailing comment after code is not read, because telling a `#`
# that starts a comment from one inside a string needs a tokenizer, and a miss here costs less than
# a refused edit the author cannot fix. A shebang is not a comment. Docstrings and string literals
# are output or interface text, not read here either.

scanned_exts := {".py", ".sh", ".bash"}

tool_input := object.get(input, "tool_input", {})

file_path := object.get(tool_input, "file_path", object.get(tool_input, "path", ""))

lower_tool_name := lower(object.get(input, "tool_name", ""))

authoring_tool if lower_tool_name in {"write", "edit", "multiedit", "notebookedit"}

authoring_tool if endswith(lower_tool_name, ".write")

authoring_tool if endswith(lower_tool_name, ".edit")

# Every piece of new text this call would put on disk: Write's `content`, Edit's `new_string`, and
# each `new_string` in a MultiEdit list.
edited_texts contains text if {
	text := object.get(tool_input, "content", "")
	text != ""
}

edited_texts contains text if {
	text := object.get(tool_input, "new_string", "")
	text != ""
}

edited_texts contains text if {
	some edit in object.get(tool_input, "edits", [])
	text := object.get(edit, "new_string", "")
	text != ""
}

# Copies of docs_no_shouting's definitions, held equal to it and to scripts/cupcake_shouting.py by
# scripts/test-shouting-signal.py. The reasoning behind each lives in docs_no_shouting.
run_pattern := `\b[A-Z]{2,}\b(?:[ \t]+\b[A-Z]{2,}\b){3,}`

emphasis_pattern := `\b(?:NOT|NEVER|NOTHING|NONE|NOBODY|NOWHERE|ALWAYS|ALL|EVERY|EVERYTHING|ONLY|BOTH|EITHER|NEITHER|MUST|MANDATORY|CANNOT|CRITICAL|IMPORTANT|IS|ARE|WAS|WERE|BEEN|BEING|DO|DOES|DID|HAS|HAVE|HAD|CAN|COULD|WILL|WOULD|SHOULD|SHALL|THE|AN|THIS|THAT|THESE|THOSE|IT|ITS|YOU|YOUR|WE|OUR|THEY|THEM|THEIR|WHAT|WHY|HOW|WHICH|WHO|WHOSE|WHEN|WHERE|AND|BUT|IN|AT|TO|FROM|WITH|WITHOUT|INTO|ONTO|BY|FOR|OF|AS|THAN|BECAUSE|SO|IF|UNLESS|UNTIL|EXCEPT|ACTUALLY|REALLY|EXACTLY|DELIBERATELY|ENTIRELY|COMPLETELY|PRECISELY|GENUINELY|SIMPLY|MERELY|WHOLLY|PURELY|STRICTLY|ALREADY|STILL|INSTEAD|ALSO|EVEN|JUST|VERY|EVER|RATHER|ONCE|TWICE|AGAIN|FIRST|SECOND|LAST|SAME|OWN|WRONG|HERE|THERE|NOW|THEN|MORE|MOST|LESS|LEAST|BETTER|WORSE)\b`

capitalised_identifier_pattern := `^[^A-Za-z0-9]*[A-Z0-9]+(?:[-_][A-Z0-9]+){2,}[^A-Za-z0-9]*$`

# Span removal by delimiter parity, because `regex.replace` does not execute in Cupcake's WASM
# runtime. An odd delimiter count returns the text untouched, which is the fail-closed direction.
outside(text, delim) := joined if {
	parts := split(text, delim)
	count(parts) % 2 == 1
	joined := concat("\n", [p |
		some i, p in parts
		i % 2 == 0
	])
} else := text

scanned_file if {
	some ext in scanned_exts
	endswith(lower(file_path), ext)
}

comment_lines contains line if {
	scanned_file
	some text in edited_texts
	some line in split(text, "\n")
	regex.match(`^\s*#`, line)
	not regex.match(`^\s*#!`, line)
}

# The same span removal docs_no_shouting applies to its lines: backticked code, then quoted text,
# then capitalised identifiers such as rule ids and constants.
judged contains fragment if {
	some line in comment_lines
	some uncoded in split(outside(line, "`"), "\n")
	some quoted in split(outside(uncoded, "\""), "\n")
	fragment := concat(" ", [word |
		some word in split(quoted, " ")
		not regex.match(capitalised_identifier_pattern, word)
	])
}

violation contains {"kind": "four or more capitalised words in a row", "line": trim_space(fragment)} if {
	some fragment in judged
	regex.match(run_pattern, fragment)
}

violation contains {"kind": "a capitalised function word", "line": trim_space(fragment)} if {
	some fragment in judged
	regex.match(emphasis_pattern, fragment)
}

deny contains decision if {
	input.hook_event_name == "PreToolUse"
	authoring_tool
	count(violation) > 0

	kinds := concat(", ", {v.kind | some v in violation})
	examples := concat("\n  ", {v.line | some v in violation})

	decision := {
		"rule_id": "DS2-MODS-SCRIPT-COMMENTS-NO-SHOUTING",
		"severity": "MEDIUM",
		"reason": concat("", [
			"🧁 Cupcake blocked a script comment that shouts (", kinds, ") in ", file_path,
			"\n\n  ", examples,
			"\n\nWhy: capitals used as emphasis are emphasis by volume. A reader skims a shouted phrase the way they skim a wall of text, so the sentence it was meant to rescue goes down with it. Same rule as docs_no_shouting, applied to the `#` comments in Python and shell scripts, where most of this repo's reasoning about its tooling lives.",
			"\n\nHappy path: lowercase the words and let the sentence's structure carry the weight -- lead with the point, or give it its own short paragraph. A name, a log line, an environment variable or a status string belongs in backticks or quotation marks, and neither is read. Screaming-snake constants and rule ids are names and are not read either.",
		]),
	}
}
