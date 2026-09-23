# METADATA
# scope: package
# title: No Size Metrics Or Issue IDs In Documentation
# description: Documentation states what code DOES, never how big it is (lines, bytes, test counts) and never links a beads issue.
# custom:
#   severity: MEDIUM
#   id: DS2-MODS-DOCS-NO-SIZE-METRICS
#   routing:
#     required_events: ["PreToolUse"]
#     required_tools: ["Write", "Edit", "MultiEdit", "NotebookEdit"]
package cupcake.policies.claude.docs_no_size_metrics

import rego.v1

# WHY THIS POLICY EXISTS
#
# A size is not a fact about behaviour, it is a fact about a snapshot, and documentation outlives
# the snapshot. Three concrete failures, all measured in this repo:
#
#   1. `ds2-save-file/src/lib.rs` refused to port the save picker because "`er-quit-menu-core` is
#      29,000 lines". That number was real and the conclusion drawn from it was false -- the picker
#      is a different crate. A reader cannot check a line count without leaving the document, so a
#      wrong one survives review and then gets cited as a reason not to build something.
#   2. Test counts ("25 host tests", "158 checks") are stale the moment anyone adds a test, and
#      they invite a reader to treat a number as coverage.
#   3. File sizes rot the same way and say nothing a reader can act on.
#
# And beads IDs: `.beads/` is a Dolt database that is squashed, renumbered and closed. A doc
# comment pointing at `ds2-mods-rs-v3f` is a dangling reference the compiler cannot check, aimed at
# a tracker the reader of a published crate does not have. Describe the missing behaviour in the
# doc; track the work in beads; do not staple one to the other.
#
# SCOPE: documentation text only -- whole-file for Markdown, and Rust `//!`/`///` doc comments.
# Ordinary `//` comments, code, tests and log strings are untouched, because a size in a log line
# is a measurement being reported rather than a claim being documented.

tool_input := object.get(input, "tool_input", {})

file_path := object.get(tool_input, "file_path", object.get(tool_input, "path", ""))

lower_tool_name := lower(object.get(input, "tool_name", ""))

authoring_tool if lower_tool_name in {"write", "edit", "multiedit", "notebookedit"}

authoring_tool if endswith(lower_tool_name, ".write")

authoring_tool if endswith(lower_tool_name, ".edit")

# Every piece of new text this call would put on disk. Edit carries `new_string`, Write carries
# `content`, MultiEdit carries a list -- all three are checked, because a rule that reads only one
# of them is a rule with a documented way around it.
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

is_markdown if endswith(lower(file_path), ".md")

is_rust if endswith(lower(file_path), ".rs")

# Markdown is documentation end to end.
doc_lines contains line if {
	is_markdown
	some text in edited_texts
	some line in split(text, "\n")
}

# In Rust only the doc comments are. `//!` and `///`, not `//`: an implementation note explaining
# why a buffer is 152 bytes is exactly the kind of size that SHOULD be written down next to the
# code that depends on it.
doc_lines contains line if {
	is_rust
	some text in edited_texts
	some line in split(text, "\n")
	regex.match(`^\s*//[!/]`, line)
}

# THE FOUR BANNED SHAPES.
#
# Each requires a DIGIT bound to the unit, so "the two rows", "every line of the table" and "the
# tests below" all stay legal -- the ban is on quantifying an artifact, not on naming one.
banned_patterns := {
	"a line count": `(?i)(?:~|about |approximately )?\d[\d,_]*\s*(?:-\s*)?lines?\b`,
	"a file size": `(?i)\d[\d,._]*\s*(?:KB|MB|GB|TB|KiB|MiB|GiB)\b`,
	"a test count": `(?i)\d[\d,_]*\s+(?:\w+\s+)?(?:tests?|checks?|assertions?|cases?)\b`,
	"a beads issue id": `(?i)\bds2-mods-rs-[a-z0-9]{3,}\b`,
}

# `N bytes` is the one shape that is sometimes a real ABI fact (a struct's size, an AES block, a
# Steam ID's length) and sometimes an artifact's bulk. It is banned only when the same line also
# names an artifact, which is what separates "the record is 152 bytes" from "the save is 8 bytes".
artifact_words := `(?i)\b(?:file|files|crate|module|save|log|archive|dll|binary|document|repo|repository)\b`

violation contains {"kind": kind, "line": trim_space(line)} if {
	some line in doc_lines
	some kind, pattern in banned_patterns
	regex.match(pattern, line)
}

violation contains {"kind": "a file size in bytes", "line": trim_space(line)} if {
	some line in doc_lines
	regex.match(`(?i)\d[\d,_]*\s*bytes\b`, line)
	regex.match(artifact_words, line)
}

deny contains decision if {
	input.hook_event_name == "PreToolUse"
	authoring_tool
	count(violation) > 0

	# NO `sort`: `scripts/check-cupcake-wasm-builtins.py` has no probe recipe for it, and an
	# unverified builtin returns UNDEFINED in Cupcake's WASM runtime -- which would make this whole
	# rule silently never fire and report a clean ALLOW. `concat` takes a set directly and emits it
	# in canonical order, which is the determinism the sort was for.
	kinds := concat(", ", {v.kind | some v in violation})
	examples := concat("\n  ", {v.line | some v in violation})

	decision := {
		"rule_id": "DS2-MODS-DOCS-NO-SIZE-METRICS",
		"severity": "MEDIUM",
		"reason": concat("", [
			"🧁 Cupcake blocked documentation carrying ", kinds, " in ", file_path,
			"\n\n  ", examples,
			"\n\nWhy: a size is a fact about a snapshot, and documentation outlives the snapshot. This repo's own doc comment refused to port the save picker because a crate was \"29,000 lines\" -- the number was real, the crate was the wrong one, and no reader could check either without leaving the document. Test counts go stale on the next test. Beads IDs point at a Dolt database that gets squashed and renumbered, and that a reader of the published crate does not have.",
			"\n\nHappy path: say what the code DOES and what is MISSING, in behaviour. Instead of \"er-quit-menu-core is 29,000 lines\" write \"the picker's menu chrome is Elden Ring's and does not port\"; instead of \"25 host tests\" write what they cover; instead of \"see ds2-mods-rs-v3f\" describe the missing behaviour and leave the tracking in beads. Sizes that a reader must act on -- a struct's ABI size, a block length -- belong in a plain `//` comment beside the code that depends on them, which this rule does not touch.",
		]),
	}
}
