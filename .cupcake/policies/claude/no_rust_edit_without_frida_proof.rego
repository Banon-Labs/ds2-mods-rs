# METADATA
# scope: package
# title: No Rust edit under crates/ without a Frida measurement behind it
# authors: ["er-quickload agents", "ds2-mods-rs agents (ported from er-mods-rs)"]
# custom:
#   severity: HIGH
#   id: DS2-MODS-NO-RUST-EDIT-WITHOUT-FRIDA-PROOF
#   description: >-
#     Refuse a Write/Edit to `crates/**/*.rs` unless a Frida session has attached to
#     the game and reported observations since the last commit.
#
#     PORTED from er-mods-rs on 2026-09-25 (`ER-EFFECTS-NO-RUST-EDIT-WITHOUT-FRIDA-PROOF`),
#     rule unchanged. The evidence reader is scripts/ds2-frida-evidence.py and the
#     watcher that records into it is scripts/ds2-frida-watch.py, both run as this
#     repo's AGENTS.md "Frida first, and only then a DLL" describes. That section was
#     itself ported from er-mods-rs on 2026-09-22 as prose; this is its executable half.
#     The history below is er-mods-rs's.
#
#     User directive 2026-09-16. AGENTS.md has said it for months -- "The order is
#     Frida, then Frida, then Frida, and only then a DLL: prototype with it, run the
#     experiment with it, and fix the thing with it if a hook can. Build a DLL when
#     the mechanism is already known and the code is the product, never to find
#     something out" -- and on this day the agent did the opposite twice in one
#     session: it tore the user's game down, ran a build, relaunched, and did it again,
#     to land a change whose mechanism it had never measured. The justification it gave
#     itself was an argument about a prior A/B it had already shown was confounded.
#     The user's answer was that a note is not a rule, so here is the rule.
#
#     The tell this encodes: when the reason for an edit is reasoning rather than a
#     measurement, the edit is a guess, and a guess here costs a teardown, a build and
#     a relaunch of somebody's game -- against an agent file that reloads in place for
#     nothing.
#
#     What opens the gate is `scripts/ds2-frida-evidence.py --check` printing PROVEN,
#     which needs a session that attached to a pid and received at least one message,
#     recorded after HEAD's commit time. The log lives under XDG_STATE_HOME, not in
#     the repo, so the Write tool being gated cannot forge its own permission. A commit
#     spends the evidence: one measurement licenses one change.
#
#     Deliberately NOT carved out: tests, doc comments, host-only crates. Every carve-out
#     is a door, and the failure this exists to stop was the agent walking through the
#     door it argued for itself. `scripts/`, `.cupcake/`, docs and every non-Rust file
#     stay editable, which is what makes it possible to build and fix this gate.
#
#     A SECOND INSTRUMENT was added 2026-09-19, and it is not one of those doors. It
#     exempts no file: it accepts a different measurement, taken at the branch by our own
#     DLL and quoted verbatim out of a live run's log, for the class Frida physically
#     cannot see -- our own unexported internals in a `cdylib` that exports only `DllMain`.
#     It is strictly narrower than the Frida path, because it opens the one crate whose
#     log it came from rather than the whole tree. The three conditions that keep it a
#     measurement (verbatim line, log newer than the last committed Rust change, real
#     crate) are enforced when the record is written, by the evidence reader.
#     A THIRD INSTRUMENT is not what arrived 2026-09-22 -- the Bash tool did. The rule
#     routed on the write TOOLS and denied on `tool_input.file_path`, and a Bash call
#     carries no such key, so the rule was undefined for it and the gate never saw the
#     edit. Measured (bd er-effects-rs-wuij): a `for c in ...; do sed -i ... crates/$c/src/config.rs;
#     done` rewrote four crate sources with no measurement behind it, and the very next
#     Edit-tool call adding a `pub fn` to the four `lib.rs` files beside them was refused.
#     Same change, same files, two different answers, decided by which tool typed it. The
#     description above says every carve-out is a door and the failure it exists to stop
#     was the agent walking through the door it argued for itself; this one nobody argued
#     for.
#   routing:
#     required_events: ["PreToolUse"]
#     required_tools: ["Write", "Edit", "MultiEdit", "NotebookEdit", "Bash"]
#     required_signals: ["frida_evidence"]
package cupcake.policies.claude.no_rust_edit_without_frida_proof

import rego.v1

import data.cupcake.system.commands

# Defaulted at both levels for the same reason the signal below is: `input.tool_input` read
# directly is undefined when the key is absent, and an undefined term anywhere in the deny body
# takes the whole rule with it. Nothing in this file may depend on a key being present.
file_path := object.get(object.get(input, "tool_input", {}), "file_path", "")

# Rust source that compiles into something the game loads. The gate is the file
# extension plus the directory, so a `.toml`, a `.md` or a script is never caught.
rust_under_crates if {
	endswith(file_path, ".rs")
	contains(file_path, "/crates/")
}

rust_under_crates if {
	endswith(file_path, ".rs")
	startswith(file_path, "crates/")
}

# The verdict line from `scripts/ds2-frida-evidence.py --check`.
#
# Read through a defaulted `signals` object rather than `input.signals` directly. The
# engine attaches signals for a policy that declares `required_signals`, but the direct
# form is undefined when the key is absent, and an undefined term in the deny body below
# makes the whole rule undefined -- the gate would open on exactly the input that proves
# least. Measured by `test_deny_when_the_signals_object_is_missing_entirely`, which failed
# against the direct form.
signals := object.get(input, "signals", {})

raw_evidence := object.get(signals, "frida_evidence", "")

# A signal whose command exits non-zero does not reach a policy as its output at all: cupcake
# replaces the string with a failure record, an object carrying `error`, `exit_code`, `output`
# and `success`. Every string operation on that object is then undefined, and an undefined term
# in the deny body takes the whole rule with it -- so the gate opens on exactly the input that
# proves least.
#
# That is not hypothetical. Measured 2026-09-16 against cupcake 0.5.2: this signal script was the
# only one in `.cupcake/signals/` without its executable bit, an auto-discovered signal is exec'd
# directly rather than through `bash`, and the result was `Signal 'frida_evidence' failed with
# exit code 126` on every evaluation. `cupcake eval` allowed a `crates/` edit with no measurement
# behind it while `opa test` was green, because the tests all handed the rule a string.
#
# Coercing a non-string to the empty string makes that case deny instead, and the reason below
# says which of the two nothings it was.
evidence := raw_evidence if {
	is_string(raw_evidence)
}

evidence := "" if {
	not is_string(raw_evidence)
}

# Anything that is not a `PROVEN` line leaves this undefined and the deny below fires --
# including an empty signal, which is what a broken or timed-out reader produces. Failing
# closed is the point: a gate that opens when its evidence reader breaks is not a gate.
#
# A Frida verdict opens every crate, because the instrument reaches the game and the game is what
# all of this eventually talks to.
proven if {
	proven_for(file_path)
}

proven_for(_) if {
	startswith(evidence, "PROVEN")
	not telemetry_verdict
}

# A telemetry verdict opens exactly one crate: the shell whose own log the quoted line came out of.
#
# The blind spot this closes, measured 2026-09-19. Frida reaches the GAME; it does not reach our
# own DLLs. A release `cdylib` here exports `DllMain` and nothing else, so an unexported Rust
# static, a `pub(crate)` seam, or "which of our functions calls which of our setters" has no
# address to attach to and no name to resolve. `er-save-game-row` served its destination browser
# undressed for exactly that reason -- `gfx_swap::set_profile_05_010_edit_armed` has one caller and
# that shell does not go through it -- and the gate answered a provable defect by demanding an
# instrument which cannot see it. An agent facing that either stalls or reaches for one of the
# forgery routes `scripts/ds2-frida-evidence.py` names in its own docstring, and neither is the
# behaviour this was written to get.
#
# What makes it evidence rather than an argument is enforced at record time, not here: the quoted
# line must be present verbatim in the named log, the log must be newer than the last committed
# Rust change, and the crate must exist. What is enforced HERE is the scope -- a measurement of one
# shell's branch says nothing about any other crate, so it may not open one. That makes this path
# narrower than the Frida path above, which opens the whole tree.
proven_for(path) if {
	telemetry_verdict
	contains(path, concat("", ["crates/", licensed_crate, "/"]))
}

# Keyed on the two fixed words alone, so a verdict that has lost its `crate=` field is still
# recognised as telemetry and is refused by the rule above rather than falling through to the
# unscoped one.
telemetry_verdict if {
	startswith(evidence, "PROVEN telemetry")
}

# Anchored at the front of the verdict: everything to the right of the crate name is free text
# from a log, and free text must not be able to impersonate the field that decides scope.
licensed_crate := name if {
	matches := regex.find_all_string_submatch_n(`^PROVEN telemetry crate=([A-Za-z0-9_-]+)(?: |$)`, evidence, 1)
	count(matches) == 1
	name := matches[0][1]
}

block_reason := "🧁 Cupcake blocked a Rust edit with no Frida measurement behind it. AGENTS.md: \"The order is Frida, then Frida, then Frida, and only then a DLL: prototype with it, run the experiment with it, and fix the thing with it if a hook can. Build a DLL when the mechanism is already known and the code is the product, never to find something out.\"\n\nGo and look first:\n  python3 scripts/ds2-frida-up.py\n  uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/<agent>.js\n\nThe watch records what it saw on exit, and `python3 scripts/ds2-frida-evidence.py --check` is what opens this gate. It needs a session that attached to a pid and received at least one message -- a watch that observed nothing did not look at anything. A commit spends the evidence, so the next change needs its own measurement.\n\nIf the mechanism is inside one of OUR DLLs rather than in the game -- an unexported static, a `pub(crate)` seam, which of our functions calls which of our setters -- Frida has nothing to attach to, and the instrument that reaches it is the shell's own in-process telemetry from a live run:\n  python3 scripts/ds2-frida-evidence.py --record-telemetry --crate <crate> --log <the run's log> --line '<a line from it, verbatim>'\nThe line must be in that log word for word and the log must be newer than the last committed Rust change. It opens that ONE crate.\n\nIf neither instrument can reach it, say so in one sentence and say what can. Do not edit around this by narrowing the change until it looks harmless.\n\nEditable without proof: `scripts/`, `.cupcake/`, docs, and every non-Rust file."

# What the refusal quotes back. The three cases are worth telling apart: a verdict line is the
# reader answering, an absent signal is nobody having asked, and a failure record is the reader
# being broken -- which is a bug to fix rather than a measurement to go and take.
said := evidence if {
	is_string(raw_evidence)
	raw_evidence != ""
}

said := "<signal absent>" if {
	is_string(raw_evidence)
	raw_evidence == ""
}

said := "<the frida_evidence signal failed; cupcake replaced its output with a failure record, so it exited non-zero. Check that .cupcake/signals/frida_evidence.sh is executable and runs>" if {
	not is_string(raw_evidence)
}

# The tools that only look. Everything else carrying a `crates/**/*.rs` path is treated as a
# write, so a write tool nobody thought to list is still refused.
#
# Routing metadata was the whole guard until 2026-09-16, when a subagent reading
# `crates/er-telemetry-core/src/counters.rs` with the Read tool was refused by this policy and
# could not find anything in `.cupcake/` that explained it. A gate whose purpose is to make an
# agent go and look must never be the thing that stops it looking, and a reader has nothing to
# offer as evidence because it changes nothing.
#
# The previous shape was argued for rather than measured: a test here recorded that a hand-built
# Read event does reach the deny, and reasoned that it did not matter because routing keeps Read
# out. It did matter. Routing is an optimisation; the deny body is the contract. Listing readers
# rather than writers keeps the fail-closed direction the old argument was right about.
read_only_tools := {"Read", "Grep", "Glob", "NotebookRead", "WebFetch", "WebSearch", "LSP"}

tool_name := object.get(input, "tool_name", "")

# ---------------------------------------------------------------------------
# THE SAME GATE FROM THE BASH TOOL (2026-09-22, bd er-effects-rs-wuij)
#
# Everything above reads `tool_input.file_path`, which a Bash call does not have. What
# follows asks the same question of a command's text: does this command WRITE a
# `crates/**/*.rs` path? It is deliberately written to catch the write verbs and to leave
# every read alone, because a gate whose purpose is to make an agent go and look must never
# be the thing that stops it looking -- the same reasoning that put `Read` in the list above.
#
# RESIDUE, stated rather than hidden. Three write paths are invisible here and none is
# closed by pretending otherwise:
#   * `git apply <patch>` and `patch < <diff>`, where the target path lives in the patch
#     file and not in the command;
#   * `cargo fmt` / `cargo clippy --fix`, which rewrite sources while naming none;
#   * a committed `python3 scripts/<name>.py` that writes a crate source, which the
#     sibling `bash_no_python_file_write` exempts by design.
# The first two are mechanical transformations rather than the guessed change this gate
# exists to stop. The third is a reviewed script, which is the same argument.

# Words of one segment. Tabs folded first, because a segment arrives as raw shell text and
# `split` on a single space would otherwise weld a tab-separated pair into one word.
segment_words(segment) := [word |
	some word in split(replace(segment, "\t", " "), " ")
	word != ""
]

# The program this segment runs: the first word that is not a wrapper. A segment carries no
# separators -- `shell_segments` has already split on them -- so "first non-wrapper" and "in
# a command slot" are the same word, and this takes the cheaper of the two readings.
#
# Cheaper is load-bearing, not a preference. The first draft asked
# `word_in_command_slot(words, index)` for every index, and that predicate counts the words
# before `index`, so the rule was quadratic in the length of a segment. A 3.5 KB command
# whose heredoc body is one segment of ~400 words exhausted the engine's wasm memory and
# aborted the whole evaluation -- which is not a refusal but `{}` at exit 0, every policy
# silent at once. Measured 2026-09-22, while adding the tests below.
program_index(words) := index if {
	indexes := [j |
		some j, _ in words
		not commands.command_slot_wrapper_at(words, j)
	]
	count(indexes) > 0
	index := min(indexes)
}

program_is(words, name) if {
	index := program_index(words)
	words[index] == name
}

program_is(words, name) if {
	index := program_index(words)
	endswith(words[index], concat("", ["/", name]))
}

# The subcommand, for the programs whose verb is their second word: `git checkout` writes
# and `git diff` does not, and the difference is not visible in the program alone.
subcommand(words) := words[index] if {
	start := program_index(words)
	indexes := [j |
		some j, word in words
		j > start
		not startswith(word, "-")
	]
	count(indexes) > 0
	index := min(indexes)
}

# The last operand, which is where `cp`, `mv` and `ln` put the file they overwrite. It is
# what tells `cp crates/a.rs /tmp/b.rs` (a read of the crate) from `cp /tmp/b.rs
# crates/a.rs` (a write of it).
last_operand_index(words) := index if {
	start := program_index(words)
	indexes := [j |
		some j, word in words
		j > start
		not startswith(word, "-")
	]
	count(indexes) > 0
	index := max(indexes)
}

# A Rust source under `crates/`, with any leading redirect character stripped so `>file`
# and `> file` are the same target.
crate_rust_path(word) := path if {
	path := trim_left(word, ">|")
	endswith(path, ".rs")
	contains(path, "crates/")
}

redirect_token(word) if {
	startswith(word, ">")
}

redirect_token(word) if {
	endswith(word, ">")
}

redirect_token(word) if {
	endswith(word, ">>")
}

# Shell redirection into the path, in either spelling.
segment_writes(words, index) if {
	startswith(words[index], ">")
}

segment_writes(words, index) if {
	index > 0
	redirect_token(words[index - 1])
}

# Programs whose whole job is to write the file they are handed.
segment_writes(words, _) if {
	some name in {"tee", "patch", "truncate", "install", "dd", "shred"}
	program_is(words, name)
}

# In-place editors, which read like a filter until the flag is there.
segment_writes(words, _) if {
	some name in {"sed", "perl", "ruby"}
	program_is(words, name)
	some word in words
	startswith(word, "-i")
}

segment_writes(words, _) if {
	some name in {"sed", "perl"}
	program_is(words, name)
	some word in words
	word == "--in-place"
}

# Copy and move write their LAST operand and read the rest.
segment_writes(words, index) if {
	some name in {"cp", "mv", "ln", "rsync"}
	program_is(words, name)
	index == last_operand_index(words)
}

# Removal destroys every operand it is given.
segment_writes(words, index) if {
	program_is(words, "rm")
	index > program_index(words)
}

# `git` by subcommand: `checkout`, `restore`, `apply`, `rm`, `mv`, `clean` and `stash`
# rewrite the working tree; `diff`, `log`, `show`, `grep` and `blame` read it.
segment_writes(words, _) if {
	program_is(words, "git")
	subcommand(words) in {"apply", "checkout", "restore", "clean", "stash", "rm", "mv"}
}

# Every `crates/**/*.rs` path this command would write.
written_crate_paths contains path if {
	some text in commands.input_executed_texts
	some segment in commands.shell_segments(text)
	words := segment_words(segment)
	some index, word in words
	path := crate_rust_path(word)
	segment_writes(words, index)
}

deny contains decision if {
	input.hook_event_name == "PreToolUse"
	not read_only_tools[tool_name]
	rust_under_crates
	not proven

	decision := {
		"rule_id": "DS2-MODS-NO-RUST-EDIT-WITHOUT-FRIDA-PROOF",
		"severity": "HIGH",
		"reason": concat("", [
			block_reason,
			"\n\nTarget: ",
			file_path,
			"\nEvidence reader said: ",
			said,
		]),
	}
}

deny contains decision if {
	input.hook_event_name == "PreToolUse"
	tool_name == "Bash"
	some path in written_crate_paths
	not proven_for(path)

	decision := {
		"rule_id": "DS2-MODS-NO-RUST-EDIT-WITHOUT-FRIDA-PROOF",
		"severity": "HIGH",
		"reason": concat("", [
			block_reason,
			"\n\nThis is the Bash spelling of the same edit. A `sed -i`, a `> file`, a `tee`, a `cp` or a `git checkout --` writes the file exactly as the Edit tool does, and until 2026-09-22 this rule could not see any of them because it read `tool_input.file_path` and a Bash call has none.\n\nTarget: ",
			path,
			"\nEvidence reader said: ",
			said,
		]),
	}
}
