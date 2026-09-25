# ds2-mods-rs

**DARK SOULS II: Scholar of the First Sin mods, written in Rust.** Cross-compiled to
`x86_64-pc-windows-msvc` from Linux with `cargo-xwin`, run under Proton.

Sibling of [`Banon-Labs/er-mods-rs`](https://github.com/Banon-Labs/er-mods-rs) and the target of
a deliberate, tiered port from it. **The engines are not the same** -- DS2 shipped in 2014 and
Elden Ring in 2022, and what survives the gap is the substrate, not the game knowledge. See
[`docs/PORTING.md`](docs/PORTING.md) for what ports, what is blocked, and what is dead.

Targets **build 9527516** (Steam appid 335300).

## Status

Loading, running in-game, and hooking. [`ds2-rva`](crates/ds2-rva/src/lib.rs) -- the one crate
allowed to contain DS2 addresses -- is no longer empty: it holds the title state machine, both boot
service chains and the substate timers, each entry carrying the disassembly it was read from.

The boot to the title menu is measured end to end in
[`docs/DS2-BOOT-WORK.md`](docs/DS2-BOOT-WORK.md), and is 875 ms shorter than it was.

**Runs offline, and by default.** Everything here patches `.text` in a live copy of the game,
which is what FromSoftware's matchmaking servers watch for, so
[`ds2-offline`](crates/ds2-offline/src/lib.rs) pins the network service's online flag to the zero
its own constructor writes and refuses the game's non-loopback sockets. Verified in-game: a boot
logged `refused api=getaddrinfo host=frpg2-steam64-ope-login.fromsoftware-game.net`. Seamless
Co-op, loaded by default, turns it off for its own sockets. See
[`docs/DS2-OFFLINE.md`](docs/DS2-OFFLINE.md).

**Four rows on the pause menu, and room for twelve.** `load-build-from-url`,
`load-character-from-file`, `save-game-to-file` and `quit-to-desktop` -- in that order, the quit row
last because it is the one the cursor must not start on -- all register through
[`ds2-menu-row`](crates/ds2-menu-row/src/lib.rs)'s public API, and `[menu_row] rows = [...]` picks
which appear and in what order. There were two slots, because the tab's item vector is a
`DLFixedVector` of capacity five and the game ships three rows on it -- and there is no seventh tab
to move to either, for five separate reasons the game spells as code literals. What there is instead
is one function that reads an item and one that reads a cell, both detoured, so the rows live in
storage the DLL owns and the ceiling becomes the grid's own bind loop: fifteen rows, less the three
shipped. A thirteenth is still refused at registration with the numbers. **Where the rows stop being
legible on the banner is not measured** and is a smaller number than twelve, and **none of the new
machinery has been in front of a running game**. **The two save-file rows have not been run either**;
the static reading behind them, and the reason loading a save from a file takes effect on the next
launch rather than this one, are in [`docs/DS2-SAVE-FILE-ROWS.md`](docs/DS2-SAVE-FILE-ROWS.md).

**With `[save_block] enabled = true` and `save-game-to-file` on the menu, the game stops saving by
itself.** One detour on `SaveLoadSystem::update` erases every request the game makes of itself --
the five-minute autosave, the save on the way out to the title, the bonfire's -- so that row is the
only thing that writes the container. The key defaults to `false`, which leaves the game's own saving
alone, and it is ignored in a run without that row. A run on 2026-09-24 loaded a character, spent five minutes in the
world and quit from the pause menu: the log answered the exit with `refused a save kind=10` and the
container's length, mtime and MD5 were unchanged. What is still unmeasured -- a `Save Game to File`
press under the feature, quit to the title rather than the desktop, a full autosave interval -- and the
two designs that were rejected are in [`docs/DS2-SAVE-BLOCK.md`](docs/DS2-SAVE-BLOCK.md).

## Three facts that shape everything here

1. **me3 cannot load DS2.** `me3 profile create --game` accepts `darksouls3, sekiro, eldenring,
   armoredcore6, nightreign`. The `[[natives]]` mechanism every er-mods-rs crate assumes does
   not exist for this game, so mods load through a `dinput8.dll` proxy instead.
2. **There are no DS2 bindings.** `fromsoftware-rs` has `darksouls3`, `eldenring`, `nightreign`
   and `sekiro` members and no `darksouls2`. 21 of er-mods-rs's 57 crates depend on
   `eldenring` + `fromsoftware-shared` and are blocked until one exists.
3. **Arxan is present.** 48 stubs, measured with [`dearxan`](https://github.com/tremwil/dearxan).
   No code is encrypted -- every one of the 2969 candidate encrypted-region lists was eliminated
   as a false positive -- but the stubs are live anti-debug and integrity checks, and MinHook
   patches prologues in `.text` for a living. Neuter Arxan before installing a hook.

## Building

```bash
# A mod DLL:
cargo xwin build --release --target x86_64-pc-windows-msvc -p ds2-loader

# The gate: rustfmt + clippy at er-mods-rs parity, against the Windows target.
bash scripts/check.sh
```

Commits are conventional commits -- `type(scope): subject`, with the subject still written as prose.
The gate checks every commit a branch adds, and a `commit-msg` hook checks each one as it is
written. See [`docs/COMMITS.md`](docs/COMMITS.md).

Check the output hash before staging or launching. A build that succeeded without recompiling
leaves the previous DLL in place, and a run against it produces evidence for code that is not
the code under test:

```bash
sha256sum target/x86_64-pc-windows-msvc/release/ds2_loader.dll
```

## Reverse engineering

The authoritative artifact is `darksoulsii-deobf.bin` at the repo root -- a flat mapped image
where **file offset == RVA**, produced by dearxan. It is gitignored; it is the game binary.

```bash
cargo run --release --manifest-path ../dearxan/Cargo.toml --example deobfuscate \
  --no-default-features --features rayon -- \
  "$HOME/.local/share/Steam/steamapps/common/Dark Souls II Scholar of the First Sin/Game/DarkSoulsII.exe" \
  darksoulsii-deobf.bin
```

Issue tracking and long-form findings live in [beads](https://github.com/steveyegge/beads)
(`.beads/`), not in markdown TODOs. Run `bd ready`.

## Repo layout

```text
crates/            mods (cdylib shells) and libraries
scripts/           gate and helpers
docs/              porting analysis, RE notes
vendor/minhook/    MinHook C source, committed (not gitignored -- worktrees need it)
```
