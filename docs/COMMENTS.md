# Comment convention

What a comment in this repo has to be. One rule per section, each one decided
deliberately and each one meant to be enforceable by `scripts/check.sh` rather
than by taste. Sections land here as they are settled; an unsettled question is
absent rather than guessed.

`docs/COMMITS.md` is the same kind of document for commit messages.

## Addresses name the `ds2_rva` constant

A comment that names an address in `DarkSoulsII.exe` names the `ds2_rva`
constant, as an intra-doc link. The literal may stand beside it; it may not
stand alone.

```rust
//! `NetService::isOnline` ([`ds2_rva::NET_IS_ONLINE`], `0x140513600`) is five bytes.
```

not

```rust
//! `NetService::isOnline` (`0x140513600`) is five bytes.
```

**Why:** a bare literal is a claim about the image that nothing in the build can
check, so it survives the entry being renamed, re-measured or deleted, and it
survives being wrong in the first place. The link does not: rustdoc fails on a
path that no longer resolves, which turns a rotting comment into a broken build.
An RVA is a number, and on a build these tables were not read from it points at
something else that would accept the write.

**Exempt: `crates/ds2-rva` itself.** It is the table, so its comments are where
the literals are defined and a self-link would say nothing. 859 of the 1,057
hex literals currently in doc comments are in that crate and stay exactly as
they are; the other 198 are the migration.

## The disassembly lives in `docs/`, not in the module doc

Transcribed instruction bytes, hook-site tables, call-site censuses, timings and
the measurement runs that produced them go in a `docs/*.md`. A module doc says
what the crate does, why it is built the way it is, and what it refuses to do --
and names the markdown file, as plain text, for the evidence:

```rust
//! `docs/DS2-INGAME-MENU.md` has the disassembly, the measurements, and the
//! corrections.
```

**Not a markdown link.** MEASURED: a doc comment containing
`[notes](../../../docs/GONE.md)` documents clean under `#![deny(warnings)]` --
rustdoc has no lint for a dead relative path, so the link rots in exactly the
silent way a bare RVA does. `#[doc = include_str!(..)]` *is* checked, and fails
the build on a missing file, but it inlines the whole document (1435 lines for
`DS2-INGAME-MENU.md`) into the crate's front page, which is not a reference.

So the plain-text mention is the form, and `scripts/check.sh` is what makes it
load-bearing: every `docs/<name>.md` string appearing in `crates/**/*.rs` must
name a file that exists.

**Why the split at all:** `ds2-menu-row` currently states the same seven hook
sites in both places, 117 doc lines against 1435 markdown ones, with separate
lifecycles and no gate tying them together. Two copies of a claim about the
image is one copy more than can be kept true.

## No length cap. Seven lints instead

There is no ceiling on a module doc, and no comment-to-code ratio.

MEASURED, against the whole installed lint set (`clippy-driver -Whelp`, 1122 lints) and against
published crates on this machine: **nobody enforces one, and practice does not converge.**

| crate | comment lines | longest `//!` block |
|---|---|---|
| `clap` 4.6.7 | 35% | 532 |
| `syn` 3.0.6 | 9% | 363 |
| `thiserror` 2.0.20 | 14% | 260 |
| `serde` 1.0.229 | 23% | 113 |
| `regex` 1.13.1 | 58% | -- |
| `rayon` 1.12.0 | 20% | 79 |
| this repo | 40% | 133 |

A ratio is also the wrong instrument here even if a number could be picked. A crate root that is
nothing but `mod` and `pub use` has almost no code by construction -- `ds2-offline/src/lib.rs` is
73 doc lines over 9 -- so a ratio fails exactly the file where the essay belongs, and passes a
2000-line module carrying forty lines of comment. The only length lint that exists anywhere is
`clippy::too_long_first_doc_paragraph`, which caps the FIRST PARAGRAPH so a summary line stays a
summary line, and that one is denied below.

What is enforced is presence and structure, which is what a tool can actually judge. Every one of
these is in `clippy::pedantic` or `clippy::restriction`, neither of which `clippy::all` includes,
so denying `all` switched on none of them:

| lint | what it demands |
|---|---|
| `rust::missing_docs` | a public item carries a doc comment |
| `clippy::undocumented_unsafe_blocks` | every `unsafe` block carries `// SAFETY:` |
| `clippy::missing_panics_doc` | a `pub fn` that can panic says so under `# Panics` |
| `clippy::missing_errors_doc` | a `pub fn -> Result` says what the error means under `# Errors` |
| `clippy::too_long_first_doc_paragraph` | the summary paragraph stays short |
| `clippy::doc_markdown` | type and symbol names wear backticks |
| `rustdoc::broken_intra_doc_links` | the rule in the first section, enforced |

**All 27 crates pass.** Turning the seven on cost about 690 fixes, and they were not evenly
spread: roughly 350 were `unsafe` blocks with no `// SAFETY:`, and almost all of those wrap a
call that cannot fault, because `ds2_game_base::mem::safe_read_*` validates its range in the
kernel and answers `None` for an unmapped one. A crate that patches a live image uses it
precisely so a structure that moved between game versions produces a refusal in the log rather
than a crash in a player's lap -- and that reasoning now sits at each block rather than only in
the callee's own docs.

The rest were what a name alone does not say: `Adaptability` is a stat, but which one; `Ambiguous
{ name, ids }` lists the ids so a HUMAN can pick, which is why that variant refuses to pick one
itself.

## Every lint allow names the issue that will remove it

An `#[allow(...)]` switches off a lint the root manifest denies on purpose. It is a hole in the
gate, and a hole nobody is tracking cannot be told from one nobody noticed. So every allow in
`crates/` carries a comment directly above it, and that comment names a `bd` issue:

```rust
// DEBT: <issue-id> -- not debt to be paid: this crate ships as a Windows DLL and the
// attribute is what keeps its Rust half parseable on the host.
#![cfg_attr(not(windows), allow(unused))]
```

**A prose reason is not enough on its own.** MEASURED before the rule landed: 30 allow sites, 4
carrying a reason and 26 bare, none naming an issue -- so the four good ones read exactly like a
decision someone made and forgot, and the rest could not be told from an accident.

The issue only has to EXIST, not to be open. `ds2-hook` transcribes MinHook's C ABI and its
`non_camel_case_types` allow is permanent; its issue says why and closes. That is still a record
a reader can follow, which a bare attribute is not. Ten crate roots share one issue for one
decision rather than filing the same debt ten times.

`#[expect(...)]` is deliberately outside the rule: it fails when the lint STOPS firing, so it
cannot rot into a silent hole the way an allow does.

Enforced by `scripts/check-allow-debt.py`, in the gate ahead of clippy, with `--selftest` for the
nine cases the rule is supposed to get right. Every site in the workspace is accounted for, under
five issues. Four of them record standing decisions rather than work: the attribute that keeps a
Windows-only crate's Rust half parseable on the host, the file that transcribes MinHook's C header
verbatim, the menu-tree dump that is disarmed but kept for re-arming, and the two export names the
Windows loader and the game's import descriptor demand letter for letter. The fifth is real debt:
one crate carries module-wide `dead_code` with no recorded reason, which also hides the next thing
that goes unused in those modules.

## A disproved belief is deleted once it can no longer mislead anyone

These comments record something someone believed and then disproved -- "the row count is
code-driven", "the layout authors five cells", "`0x14160de19` is the switch". They are worth
writing. They are not worth keeping forever: nothing ever deletes one, every session adds another,
and a module doc that is mostly a list of things that are not true is a worse guide to the code
than a short one that says what is.

**The test is whether the reader can still arrive at the wrong belief from what is in front of
them.** `FUN_140021b30` is still in the image and still writes only the logical item count, so the
correction beside it still does work. A wrong sentence in a markdown file is different: the place
to fix that is the markdown file.

Worked example, applied in the same commit as this section. `docs/DS2-BOOT-WORK.md` asked whether
`0x14160de19` removes the network boot chain and marked it "not established". It had been
established -- in `ds2-offline`'s module doc, in a section titled "this is where that was settled".
So the open question in the document sat next to an answer nobody reading the document would find.
The answer moved into `DS2-BOOT-WORK.md` where the question was asked, and the section came out of
the crate.

**Nothing is lost by deleting.** The measurement is in `docs/`, the reasoning is in the commit
message, and the whole text is in git history. Deleting the paragraph removes a copy, not a record.

**This one is not enforced by a script and cannot be.** No lint can tell a correction that is still
load-bearing from one whose subject is gone -- that judgement needs to know what the reader is
looking at. It is a review rule, and a review rule is exactly as binding as the reviewer, which is
why it is written down here with a worked example rather than as a slogan.
