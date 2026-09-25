# Commit messages: conventional commits, in this repo's voice

Every commit added on a branch reads:

```text
type(scope): subject

body

footers
```

`scripts/check-commit-message.py` is the rule. It runs twice: the `commit-msg` hook checks the
message as you write it, and `scripts/check.sh` checks every commit in `origin/main..HEAD` before
the branch goes out. History that predates the rule is not judged and is not rewritten.

## The subject is still prose

Adopting the convention does not mean adopting terse imperative headlines. This repo's subjects say
what changed and what it means -- `the save dialog opens where the load dialog opens`, `a matching
item id is not an item you own` -- and that survives untouched. The type and scope go on the front
of it, and nothing else about the sentence changes:

```text
fix(ds2-build-import): a matching item id is not an item you own
feat(ds2-save-block): stop the game saving by itself while the save row is on
feat(scripts): --all-menu-rows, so a run can reach the two save-file rows
```

The header is capped at 120 characters rather than the usual hundred, because the longest subject in
the history that predates this rule needed a little over a hundred on its own and the prefix has to
fit somewhere. The body wraps at 100, which is what the repo already writes by hand.

## Types

The type is the kind of change, never the place it happened -- that is what the scope is for. A new
flag in a script is a `feat`, not a `chore`, because someone can do something they could not do
before, and the directory it lives in does not change that.

| Type | Means |
| --- | --- |
| `feat` | New behaviour a player, an agent or a caller can reach. |
| `fix` | A defect in behaviour that already shipped. |
| `perf` | The same behaviour, measurably faster. |
| `refactor` | The same behaviour, a different shape. |
| `docs` | Documentation and reverse-engineering notes only. |
| `test` | Tests and selftests only. |
| `build` | The workspace, its dependencies, the toolchain, vendored sources. |
| `ci` | Automation that runs outside a developer's shell. |
| `chore` | Housekeeping that changes nobody's behaviour. |
| `revert` | Backs out an earlier commit. |

## Scopes

A scope is optional. When present it is either a crate directory under `crates/` -- spelled in full,
`ds2-menu-row`, not `menu-row` -- or one of the directories that are not crates:

`scripts`, `docs`, `cupcake`, `beads`, `workspace`, `vendor`, `github`.

The crate half of that list is read off disk on every run, so a new crate is a legal scope the
moment it exists and a scope naming no crate is a typo rather than a new area.

## Body and footers

The body is where the evidence goes: the run, the disassembly, the measurement, the thing that was
assumed and turned out to be false. A blank line separates it from the header. Footers are ordinary
git trailers and go at the end, the `Co-Authored-By` line among them.

Track work in beads and describe the behaviour in the message. A commit body pointing at an issue id
instead of saying what changed is a dangling reference aimed at a database the reader of a clone does
not have.

## Breaking changes

Put a `!` before the colon, and explain what breaks in the body under a `BREAKING CHANGE:` footer:

```text
feat(ds2-menu-row)!: a row is registered by name, and the numbers are gone
```

## What the rule does not judge

Wording git composes itself, because refusing it would refuse git rather than the author:

- merge commits
- the `Revert "..."` subject `git revert` writes
- the `fixup!`, `squash!` and `amend!` prefixes used for autosquash

## When a commit is refused

The hook prints the header, one sentence per problem, and exits non-zero; the commit is not made and
the message is still in the editor buffer or in `.git/COMMIT_EDITMSG`. Fix the header and commit
again. Do not reach for `--no-verify` -- a policy guard blocks it, and the gate would catch the same
commit later anyway.

To check a message or a branch by hand:

```bash
python3 scripts/check-commit-message.py .git/COMMIT_EDITMSG
python3 scripts/check-commit-message.py --range origin/main..HEAD
python3 scripts/check-commit-message.py --selftest
```

## Pull requests

The title of a pull request is a commit header and follows the same rule. The body follows
`.github/pull_request_template.md`, which github fills in automatically: three headings, spelled
exactly, and the whole body under 2500 characters. Open it as a draft and keep the attribution
footer. All four are enforced by global policy guards rather than by anything in this repo.
