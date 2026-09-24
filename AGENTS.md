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

## Frida first, and only then a DLL

**Ported from `er-mods-rs/AGENTS.md` on 2026-09-22, after a session that proved the cost of not
having it.** The crates were ported from that repo; the workflow that made them tractable was not.

**The order is Frida, then Frida, then Frida, and only then a DLL: prototype with it, run the
experiment with it, and fix the thing with it if a hook can. Build a DLL when the mechanism is
already known and the code is the product, never to find something out.** A build plus a stage plus
a launch is three minutes of the user's session per attempt, and it puts a polling oracle where an
event belongs; a Frida agent file reloads in place and costs nothing.

Two tests for reaching for the wrong one:

- The question is **who wrote this**, **when did it change**, **which function ran**, or **what is
  actually in that buffer**. A rebuild can only sample the value afterwards and guess at the cause.
- The answer will arrive through a **log line you then have to read**. That is a poll wearing a
  different hat, and a `tail -f` of `ds2-loader.log` is not an oracle, it is the absence of one.

**The failure this was written from (2026-09-22).** `ds2-invasion-path` drew an arrow that pointed
the wrong way. Ten build/stage/launch cycles went into theories about the arrow's arithmetic --
world-space versus screen-space, pinning the base, bearings, clip space, a mirrored axis -- across
roughly three hours, with the user sending screenshots because no instrument in the repo could
answer. The cause was sixteen floats that had never once been printed:

```text
[0.0000 3.4405 0.0000 0.0000 | 0.0000 0.0000 2.4751 2.4751
 | 0.0000 0.0000 -0.1000 0.0000 | 1.4 0.0 -0.1 0.0]
```

Not a view-projection at all. Its first column is zero, so `clip.x` was the constant `1.4` for
every point in the world, and no arrow arithmetic could ever have been right. One attach, one
`hexdump` of the constant buffer, and the session ends in a minute.

### Bringing it up

- **`python3 /home/banon/projects/ds2-mods-rs/scripts/ds2-frida-up.py`** runs the Windows
  `frida-server.exe` INSIDE the game's pressure-vessel container via `nsenter`, which is the whole
  trick. `pid:` and `net:` are shared with the host while `mnt:` and `user:` are not, so entering
  needs no privilege and the port is reachable afterwards. A server started BESIDE the container
  talks to a different wineserver: it accepts the TCP connection and then never answers
  `enumerate_processes`, so a watcher sits silent instead of failing. **An open port is not proof
  of a working server**; `--force` replaces one that has gone stale.
- **The world gate is the overlay's own `roster:` line.** `ds2-invasion-path` writes it only after
  walking `CharacterManager` from a live `GameManagerImp` and resolving the local player, so it
  cannot appear at the title screen or during a load. Launch with `--invasion-path
  --invasion-path-on`, or pass `--allow-early` when the boot itself is what you are measuring.
- **Attach once and hot-reload the agent file:**
  `uv run --with frida python3 /home/banon/projects/ds2-mods-rs/scripts/ds2-frida-watch.py --agent
  scripts/frida/<agent>.js`. Editing the `.js` reloads it in place. Every attach/detach cycle
  installs and reverts trampolines under running game threads, so one attach and many edits is both
  the intended workflow and the safer one.
- **A plain `frida.attach()` fails** -- it injects a Linux bootstrapper into a Windows process.
  Measured on Frida 17.17.0 against Elden Ring; not re-measured here, and there is no reason to.
- **`python3 .../ds2-frida-up.py --selftest` needs no game** and proves the container entry, the
  download pin, the stop-by-comm behaviour and the world gate.

### What NOT to do

- **Never arm `MemoryAccessMonitor` on a live game object.** It revokes access to a whole 4 KB page
  and turns every access by every thread into a fault Frida must resume. Use a hardware watchpoint
  instead: four per thread, eight bytes each, no protection change.
- **It is a method on a THREAD OBJECT, not on `Thread`.** `Thread.setHardwareWatchpoint` does not
  exist. The real one comes off `Process.enumerateThreads()`, slot id first:

  ```js
  for (const thread of Process.enumerateThreads()) {
    thread.setHardwareWatchpoint(0, address, 4, 'w');   // slot, address, size, 'r'|'w'|'rw'
  }
  Process.setExceptionHandler(function (details) { /* details.address is the writer */ });
  ```

- **Arm ONE thread -- the one that would do the write. Arming every thread kills the game.** Inside
  an `Interceptor`, `Process.getCurrentThreadId()` is the thread that just ran the function you
  care about.
- **A watchpoint outlives the agent that set it, so NEVER hard-kill a watcher holding one.** It
  lives in the thread's debug registers. Once the agent is gone nothing services the exception and
  the next write to that address kills the game with nothing in the crash log. The agent must
  export `dispose` and unset every slot there, and the caller must not wrap the watcher in
  `timeout` or any other hard kill.
- **Give the watcher its own background task. Never chain it after anything** -- the harness's cap
  applies to the whole chain and lands on the watcher at the end of it.
- **Count the successes and report the failures.** An instrument that reports an absence it cannot
  detect is worse than no instrument.
- **A probe that catches nothing has told you nothing. Rule out the near end first.**

### Not yet ported

`scripts/ds2_run_lib.py` still carries Elden Ring's `WorldChrMan` walk (`player_in_a_world`,
`world_read_selftest`, the RVA constants). Its selftest passes because it plants its own memory, so
it proves arithmetic rather than DARK SOULS II offsets. Nothing in the Frida path depends on it --
the world gate reads the loader log instead -- but do not read those functions as DS2 facts.

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
