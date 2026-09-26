# METADATA
# scope: package
# title: No Shouting In Documentation
# description: Documentation puts its emphasis in the sentence, not in the capitalisation.
# custom:
#   severity: MEDIUM
#   id: DS2-MODS-DOCS-NO-SHOUTING
#   routing:
#     required_events: ["PreToolUse"]
#     required_tools: ["Write", "Edit", "MultiEdit", "NotebookEdit"]
package cupcake.policies.claude.docs_no_shouting

import rego.v1

# Why this policy exists.
#
# User directive, 2026-09-23, written in the style it is complaining about: "When I TALK LIKE THIS
# everyone thinks the WORDS ARE IMPORTANT but really it distracts from the SUBSTANCE OF THE MESSAGE
# which is burried by font style and EXCESSIVE PROSE."
#
# A capitalised phrase is emphasis by volume, and a reader skims past it for the same reason a
# reader skims a wall of text: nothing in it tells them which part is the point, so they stop
# looking for one. The habit is everywhere in this repo's doc comments -- headings shouted at the
# top of a paragraph, a clause shouted in the middle of a sentence -- and the sentence that was
# supposed to be rescued goes down with it.
#
# What is refused, and both halves are measured rather than guessed. The measurement is a sweep of
# the 120 git-tracked Markdown files and Rust sources in this checkout, read at the scope below,
# with backticked spans, fenced blocks and quoted spans already removed;
# scripts/cupcake_shouting.py carries the full working.
#
#   1. Four or more capitalised words in a row. At four the sweep finds 54 of them and every one is
#      the habit. At three it finds 86 more, and 80 of those are the game's name written out, a
#      title-screen prompt and the beads markers in CLAUDE.md -- thirteen false positives for each
#      real one. Four is where that stops.
#
#   2. One capitalised word taken from a closed list of function words and absolutes -- the, not,
#      every, only, must, never. This is the half that catches a one-word shout, which the run rule
#      cannot see, and it is safe to run at one word precisely because no entry on the list could
#      ever be a name. It fires 542 times across the same sweep and every sampled hit is the habit.
#
# What survives, and none of it needs an allow-list of its own. A word boundary treats an
# underscore as part of the word, so SAVE_DIR_BUILD, FE_INGAME_MENU_ITEM_VECTOR_CAPACITY and
# FUN_1402e67f0 are one token and never match; a digit does the same for DS2, BND4, MD5, UTF-8 and
# DS2SOFS0000.sl2; a lowercase letter does it for DllMain. Acronyms in an ordinary sentence -- DLL,
# RVA, AES, CBC, TOML, JSON, MSVC, CPU, API, PID, OK, CI, PR -- are names rather than volume, so
# none of them is on the word list and it takes four of them in a row with no punctuation between
# to make a run. Anything inside backticks, inside a fenced block or inside quotation marks is
# removed before any of this runs, which is where a log line, a menu string or a register dump
# belongs anyway.
#
# Scope: prose a human reads. Whole-file for Markdown, and every Rust comment -- `//!`, `///` and
# plain `//` alike. A string literal and a log line the code emits are untouched, because those are
# output rather than explanation.
#
# This is wider than docs_no_size_metrics, deliberately. That rule exempts `//` so an ABI size can
# sit beside the code depending on it, which is a fact a reader needs at that spot. Shouting is
# never such a fact, and the `//` beside the code is where this repo keeps its real explanations --
# so exempting it left the habit its favourite hiding place. Found there on 2026-09-24.

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

# The two patterns below are the only definition of the offence in this repo.
#
# scripts/cupcake_shouting.py holds byte-identical copies for the Stop arm, and
# scripts/test-shouting-signal.py fails if the three ever stop matching. A guard whose two halves
# disagree about what they forbid is worse than one half, because the disagreement is invisible
# until somebody is refused by one and waved through by the other.

# Four capitalised words separated by spaces or tabs. Punctuation between two words ends the run,
# which is what keeps a comma-separated list of acronyms -- "PE, COFF, IAT, TLS" -- from reading as
# one shouted phrase.
run_pattern := `\b[A-Z]{2,}\b(?:[ \t]+\b[A-Z]{2,}\b){3,}`

# One capitalised function word or absolute. Nothing on this list can be a name, an acronym, a
# register or a type, which is why a single hit is enough. Words deliberately left off it, each
# read out of this repo's own text: ON/OFF, YES/NO, TRUE/FALSE and NEXT are table-cell values here,
# LEFT/RIGHT are hand positions, BE/LE are byte orders, OUT is a parameter direction, ONE/TWO are
# counts, and OR appears only as "OR'd together".
emphasis_pattern := `\b(?:NOT|NEVER|NOTHING|NONE|NOBODY|NOWHERE|ALWAYS|ALL|EVERY|EVERYTHING|ONLY|BOTH|EITHER|NEITHER|MUST|MANDATORY|CANNOT|CRITICAL|IMPORTANT|IS|ARE|WAS|WERE|BEEN|BEING|DO|DOES|DID|HAS|HAVE|HAD|CAN|COULD|WILL|WOULD|SHOULD|SHALL|THE|AN|THIS|THAT|THESE|THOSE|IT|ITS|YOU|YOUR|WE|OUR|THEY|THEM|THEIR|WHAT|WHY|HOW|WHICH|WHO|WHOSE|WHEN|WHERE|AND|BUT|IN|AT|TO|FROM|WITH|WITHOUT|INTO|ONTO|BY|FOR|OF|AS|THAN|BECAUSE|SO|IF|UNLESS|UNTIL|EXCEPT|ACTUALLY|REALLY|EXACTLY|DELIBERATELY|ENTIRELY|COMPLETELY|PRECISELY|GENUINELY|SIMPLY|MERELY|WHOLLY|PURELY|STRICTLY|ALREADY|STILL|INSTEAD|ALSO|EVEN|JUST|VERY|EVER|RATHER|ONCE|TWICE|AGAIN|FIRST|SECOND|LAST|SAME|OWN|WRONG|HERE|THERE|NOW|THEN|MORE|MOST|LESS|LEAST|BETTER|WORSE)\b`

# Removing a verbatim span, by counting delimiters rather than by rewriting the string.
#
# `regex.replace` is the obvious tool and it cannot be used: it is host-dispatched, Cupcake 0.5.2's
# WASM runtime does not implement it, and a call to it returns UNDEFINED at runtime -- the rule
# would never fire and the engine would report a clean allow. Measured against the live runtime on
# 2026-09-23 with the probe harness in scripts/check-cupcake-wasm-builtins.py, alongside
# `regex.split`, which is dead the same way. `split`, `concat` and `rem` all execute.
#
# So the spans are located by parity, the same trick .cupcake/system/commands.rego uses to read
# quoted text: split on the delimiter, and the pieces at even indices are the ones outside it. The
# `%` below is the `rem` builtin under another spelling -- `opa fmt` rewrites the named call to the
# operator -- and scripts/check-cupcake-wasm-builtins.py had no way to see an infix operator at all
# until this policy needed one, so it now reads them too and probes all five. The pieces are
# rejoined with a NEWLINE rather than a space, because everything downstream works line by line and
# a space would weld the words on either side of a removed span into one run that never existed in
# the source.
#
# An odd number of delimiters means the parity read says nothing, so the text comes back untouched
# and gets judged. That is the fail-closed direction: half a code span must not be a way to hide a
# shouted sentence.
outside(text, delim) := joined if {
	parts := split(text, delim)
	count(parts) % 2 == 1
	joined := concat("\n", [p |
		some i, p in parts
		i % 2 == 0
	])
} else := text

# Fenced blocks come off the whole text before it is cut into lines, since a fence spans lines.
unfenced_lines contains line if {
	some text in edited_texts
	some line in split(outside(text, "```"), "\n")
}

# Markdown is documentation end to end.
doc_lines contains line if {
	is_markdown
	some line in unfenced_lines
}

# In Rust, every comment. This used to read `^\s*//[!/]` -- doc comments only, the cut
# docs_no_size_metrics makes -- and that exemption was found by the user in a merged diff on
# 2026-09-24, on a line this rule had waved through:
#
#   // A COPY YOU PUT AWAY IS NOT A COPY IN YOUR HANDS, and equipping one is what lost the sword.
#
# The reasoning for the old cut was that a `//` beside the code is not prose anybody skims. It does
# not survive contact with this repo, where the load-bearing explanations live in exactly those
# comments -- and the shouting habit lives there with them, because it was the one place the rule
# could not see. A size metric in a `//` is still exempt and should be: that exemption exists so an
# ABI size can sit beside the code that depends on it, which is a fact a reader needs. Volume is
# never a fact a reader needs.
doc_lines contains line if {
	is_rust
	some line in unfenced_lines
	regex.match(`^\s*//`, line)
}

# What is left after the inline spans go: backticked code first, then quoted text. A quotation is
# somebody else's words, and this is also what keeps the game's full title silent -- it is written
# "DARK SOULS II: SCHOLAR OF THE FIRST SIN", in quotation marks, both times it appears here.
#
# Capitalised IDENTIFIERS go too: a rule id or a constant is a name, and its capitals are its
# spelling. Measured 2026-09-21, porting the er guard policies: a scratch note naming
# DS2-MODS-NO-FIX-CLAIM-... and DS2-MODS-NO-GREP-FOR-BUILD-ERRORS unquoted was refused as shouting,
# because `\b` stops at a hyphen, so FOR and WITHOUT inside the ids read as capitalised function
# words. Three or more segments joined by `-` or `_` is what makes it an identifier: a rule id
# has five or more, MIN_INTERVAL_SECONDS has three, and a shouted phrase has spaces, not joins.
# Two segments stay judged, so NOT-ALLOWED is still shouting.
#
# Dropped a whitespace-separated word at a time, NOT with regex.replace: that builtin does not
# execute in Cupcake's WASM runtime (scripts/check-cupcake-wasm-builtins.py, 2026-09-25), so a rule
# built on it would pass `opa test` and silently never fire live.
capitalised_identifier_pattern := `^[^A-Za-z0-9]*[A-Z0-9]+(?:[-_][A-Z0-9]+){2,}[^A-Za-z0-9]*$`

judged contains fragment if {
	some line in doc_lines
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

	# No `sort`: scripts/check-cupcake-wasm-builtins.py has no probe recipe for it, and an
	# unverified builtin returns undefined in Cupcake's WASM runtime, which would make this whole
	# rule silently never fire and report a clean allow. `concat` takes a set directly and emits it
	# in canonical order, which is the determinism a sort would have been for.
	kinds := concat(", ", {v.kind | some v in violation})
	examples := concat("\n  ", {v.line | some v in violation})

	decision := {
		"rule_id": "DS2-MODS-DOCS-NO-SHOUTING",
		"severity": "MEDIUM",
		"reason": concat("", [
			"🧁 Cupcake blocked documentation that shouts (", kinds, ") in ", file_path,
			"\n\n  ", examples,
			"\n\nWhy: capitals used as emphasis are emphasis by volume, and a reader skims them exactly as they skim a wall of text -- nothing in a shouted phrase says which part of it is the point, so they stop looking for one, and the sentence it was meant to rescue goes down with it. The user's words, 2026-09-23: writing like this \"distracts from the substance of the message, which is buried by font style\".",
			"\n\nHappy path: put the emphasis in the sentence's structure instead of in its capitalisation. Lead with the thing that matters, or give it its own short paragraph, and let the position carry the weight. \"THE FIELD IS OPENED HERE, not on the worker\" becomes \"The field is opened here -- not on the worker.\" A heading that needs to stand out is a heading: use one. A name, a menu string, a register or a log line belongs in backticks or quotation marks, and this rule does not read either of those, nor string literals, nor code. Plain `//` comments beside the code are read too -- exempting them was where the habit hid.",
		]),
	}
}
