# Agent Instructions

This project uses **bd** (beads) for issue tracking. Run `bd prime` for full workflow context.

> **Architecture in one line:** Issues live in a local Dolt database
> (`.beads/dolt/`); cross-machine sync uses `bd dolt push/pull` (a
> git-compatible protocol), stored under `refs/dolt/data` on your git
> remote -- separate from `refs/heads/*` where your code lives.
> `.beads/issues.jsonl` is a passive export, not the wire protocol.
>
> See [SYNC_CONCEPTS.md](https://github.com/gastownhall/beads/blob/main/docs/SYNC_CONCEPTS.md)
> for the one-screen overview and anti-patterns (don't treat JSONL as the
> source of truth; don't `bd import` during normal operation; don't
> reach for third-party Dolt hosting before trying the default).

## Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work atomically
bd close <id>         # Complete work
bd dolt push          # Push beads data to remote
```

## Non-Interactive Shell Commands

**ALWAYS use non-interactive flags** with file operations to avoid hanging on confirmation prompts.

Shell commands like `cp`, `mv`, and `rm` may be aliased to include `-i` (interactive) mode on some systems, causing the agent to hang indefinitely waiting for y/n input.

**Use these forms instead:**
```bash
# Force overwrite without prompting
cp -f source dest           # NOT: cp source dest
mv -f source dest           # NOT: mv source dest
rm -f file                  # NOT: rm file

# For recursive operations
rm -rf directory            # NOT: rm -r directory
cp -rf source dest          # NOT: cp -r source dest
```

**Other commands that may prompt:**
- `scp` - use `-o BatchMode=yes` for non-interactive
- `ssh` - use `-o BatchMode=yes` to fail instead of prompting
- `apt-get` - use `-y` flag
- `brew` - use `HOMEBREW_NO_AUTO_UPDATE=1` env var

<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:7510c1e2 -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking -- do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge -- do NOT use MEMORY.md files

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/SYNC_CONCEPTS.md for details and anti-patterns.

## Session Completion

**When ending a work session**, you MUST complete ALL steps below. Work is NOT complete until `git push` succeeds.

**MANDATORY WORKFLOW:**

1. **File issues for remaining work** - Create issues for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **RUN THE GAME** (if anything under `crates/` or `scripts/ds2-run.py` changed) -- see below
4. **Update issue status** - Close finished work, update in-progress items
5. **PUSH TO REMOTE** - This is MANDATORY:
   ```bash
   git pull --rebase
   git push
   git status  # MUST show "up to date with origin"
   ```
6. **Clean up** - Clear stashes, prune remote branches
7. **Verify** - All changes committed AND pushed
8. **Hand off** - Provide context for next session

**CRITICAL RULES:**
- Game code is not pushed until it has been in a process. `scripts/check.sh` compiles for MSVC and
  runs pure-logic tests under wine, and cannot observe one thing that makes this repo hard: Arxan's
  stubs, whether a detour survives, a `.flo` child-count substitution, an item vector's inline
  storage. A crate can be green and panic the game's allocator on the first menu open. Launch with
  `scripts/ds2-run.py`, which prints its block only after reading the DLL's own line out of the log
  the DLL wrote during that run. Enforced by `DS2-MODS-REQUIRE-RUNTIME-BEFORE-PUSH`, not by this
  paragraph -- this paragraph is why.
- Work is NOT complete until `git push` succeeds
- NEVER stop before pushing - that leaves work stranded locally
- NEVER say "ready to push when you are" - YOU must push
- If push fails, resolve and retry until it succeeds
<!-- END BEADS INTEGRATION -->

## Opening a PR from this repo

Two global policy guards apply to `gh` and will block a non-compliant command. Satisfy them up
front rather than discovering them at the block:

- **`github_pr_draft_guard`** -- PRs must be created **as drafts**, and taking one out of draft is
  blocked. Use `gh pr create --draft`. Note that `--draft` has been observed not to take on
  creation; check with `gh pr view <n> --json isDraft` and fix with `gh pr ready <n> --undo`.
- **`github_attribution_guard`** -- issue and PR bodies written through `gh` must carry the
  footer ` Written by <agent>, authorized by @<github-username>`. The guard resolves the body
  from inline `--body`, `--body-file` contents, and MCP `body` alike, so there is no spelling of
  the command that avoids it.

Both guards are correct. If a command is blocked, fix the command; do not work around the guard.

## Every address, offset and enum value a feature touches is a named constant

A magic number in a feature crate is a research finding that was written down in the wrong place.
When research establishes a field -- an offset, an RVA, a state value, a prologue, a session type --
it goes into `crates/ds2-rva` as a `pub const` with a name, before any code uses it, and the crate
that needs it refers to it by that name. The comment beside the constant is where the evidence and
the disassembly go; the comment is never the only place the value exists.

This is not style. A named constant is the only thing that survives being wrong:

- `SL_CONTENT_DIRECTORY_OFFSET` was `0x08` and was named for a directory. A live run read it and
  got `length=356486873167 capacity=7` -- UTF-16 `"OFS\0"` and a length of seven, which is the
  container name `"DS2SOFS"` and not a directory at all. The name is what made the measurement
  legible; a bare `+ 0x08` in the reader would have been four lines of arithmetic with nothing to
  contradict.
- A value spelled once can be corrected once. The same number appearing inline in three crates is
  three separate things to find when the meaning turns out to be different.
- `grep` over `ds2-rva` is the only inventory of what has been established about the binary. A
  finding that lives in a comment in a feature crate is not in that inventory, so the next agent
  re-derives it.

So: no bare hex in a feature crate, and no offset explained only in prose. If a value is worth
acting on, it is worth naming, and if it is not yet understood well enough to name, that is the
thing to say -- name it for what was observed (`SL_CONTENT_NAME_OFFSET`) rather than for what it
was hoped to be.
