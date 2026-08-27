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
