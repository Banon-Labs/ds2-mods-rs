#!/usr/bin/env python3
"""Stage `dinput8.dll` into DARK SOULS II and launch it, gated on the DLL's own log line.

WHY A TOOL AND NOT AN AGENT'S SENTENCE
--------------------------------------
"I launched it and it worked" is a claim a tool can make and a person can check. This script
prints its success block ONLY after reading the loader DLL's `ds2-loader: arxan ...` line out of
the log file the DLL itself wrote during THIS run. Nothing else -- not the process existing, not
Steam returning zero, not a window appearing -- is accepted as evidence, because a copy-pasted
block is a promise to whoever reads it. On timeout it says so plainly and exits non-zero.

Patterned on `../er-mods-rs/scripts/er-run-branch.py`, whose testimony discipline (and whose two
hard-won log-tailing bugs -- rotation and partial lines, both commented below) this reproduces.

THE PIPELINE
------------
  1. PREFLIGHT   -- the built DLL exists; the game directory exists and is writable; `steam` is
                    on PATH. Refuse rather than guess.
  2. STAGE       -- copy the built DLL over `<Game>/dinput8.dll`.
  3. FINGERPRINT -- print the SHA-256 of the STAGED file, read back off disk. A build that
                    failed, or that "succeeded" without recompiling, leaves the previous DLL in
                    place; a run against that produces evidence for code which is not the code
                    under test. The hash is what makes that visible instead of invisible.
  4. CONFIGURE   -- write `<Game>/ds2-mods.toml` for the requested arm, and print it verbatim.
                    The DLL reads this itself in `DllMain`; see below.
  5. LAUNCH      -- `steam -applaunch 335300` with WINEDLLOVERRIDES="dinput8=n,b". Without that
                    override Wine's builtin dinput8 wins the load and our DLL never runs.
  6. TESTIMONY   -- poll `<Game>/ds2-loader.log` for the DLL's own line. Success block only if
                    it appears; a FAILED block, and a non-zero exit, otherwise.

WHY THE ARM IS IN A FILE AND NOT IN THE ENVIRONMENT
---------------------------------------------------
It was `DS2_ARXAN_PROBE=1` / `DS2_ARXAN_PROBE_SKIP_NEUTER=1`, set here, and it did not work. A
real run produced, from the DLL's own attach line:

    ds2-loader: attach awaiting-arxan-callback probe=off arm=neuter-arxan
                DS2_ARXAN_PROBE=<unset> DS2_ARXAN_PROBE_SKIP_NEUTER=<unset>

`steam -applaunch` hands the request to an ALREADY-RUNNING Steam client over IPC, and that client
starts the game from ITS environment. `WINEDLLOVERRIDES` survives only because it is in the
per-app Steam launch options -- a different channel, and the one setting that cannot move into the
config file, because Wine reads it to decide whether to map our DLL at all.

The fixes available for the environment were "quit Steam before every run" and "edit the launch
options between the two arms". Both are manual steps BETWEEN THE TWO HALVES OF ONE EXPERIMENT, and
a manual step there is a step that eventually gets skipped. A file beside the DLL travels through
no IPC, and both arms now run back to back with nothing to do in between.

THE M1 EXPERIMENT
-----------------
`--probe` runs the Arxan-survival experiment in `crates/ds2-loader/src/arxan_probe.rs`: one
MinHook detour, watched byte-for-byte to see whether Arxan reverts it. It has TWO ARMS and both
have to be run, because "the detour survived" means nothing on its own -- it is equally
consistent with "dearxan saved us" and with "Arxan never cared about this page":

    --probe neuter       neuter_arxan runs first.  Answers: does hooking work WITH dearxan?
    --probe skip-neuter  Arxan's 48 stubs left live. Answers: was Arxan ever a threat here?

The verdict block is assembled from the DLL's own `ds2-probe:` lines and from nothing else. In
particular the arm is READ BACK out of the log and compared against the one that was requested.
That guard was written for environment variables that could silently vanish, and moving to a file
did not retire it: a file can fail to be written, be written to the wrong directory, or be left
over from a previous run -- and a run against a stale file would report a perfectly well-formed
verdict for an experiment nobody asked for.

Usage:
    python3 scripts/ds2-run.py --dry-run   # stage nothing, launch nothing, report what it would do
    python3 scripts/ds2-run.py --dry-run --probe skip-neuter   # ... for that arm
    python3 scripts/ds2-run.py --selftest  # exercise the log tailer and the verdict logic
    python3 scripts/ds2-run.py             # the real run, probe off
    python3 scripts/ds2-run.py --probe neuter        # arm A
    python3 scripts/ds2-run.py --probe skip-neuter   # arm B
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone
from collections.abc import Sequence
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

GAME_DIR = (
    Path.home()
    / ".local/share/Steam/steamapps/common/Dark Souls II Scholar of the First Sin/Game"
)
APPID = "335300"

#: cargo emits this because `crates/ds2-loader/Cargo.toml` sets `[lib] name = "dinput8"`.
BUILT_DLL = REPO_ROOT / "target/x86_64-pc-windows-msvc/release/dinput8.dll"
STAGED_DLL_NAME = "dinput8.dll"

#: Where runs play out of unless `--save-dir` says otherwise.
#:
#: A real folder outside the Proton prefix, chosen so that the default run does not touch the
#: container Steam syncs. The game opens its own container name INSIDE it, so this one directory
#: serves both extensions: `DS2SOFS0000.sl2` in an ordinary run, `DS2SOFS0000.co2` under
#: `--seamless`, because the name is Seamless's to choose and not this script's.
#:
#: Pass `--save-dir ''` to turn the redirect off and play out of the prefix's own AppData again.
DEFAULT_SAVE_DIR = "~/Downloads/DS2 Saves/new"

#: Our own injector, from `crates/ds2-launcher`. Needed only when something has to be loaded
#: from OUTSIDE the process -- see `[launcher]` below and that crate's docs for the measurement
#: that says why an in-process load is not an option for every mod.
BUILT_LAUNCHER = REPO_ROOT / "target/x86_64-pc-windows-msvc/release/ds2-launcher.exe"
STAGED_LAUNCHER_NAME = "ds2-launcher.exe"

#: `[launcher]` -- the DLLs our own injector puts into the suspended process.
#: Mirrors `CONFIG_SECTION` in `crates/ds2-launcher/src/plan.rs`.
LAUNCHER_SECTION = "launcher"
#: Mirrors `KEY_DLLS` there.
KEY_LAUNCHER_DLLS = "dlls"
#: Mirrors `LOG_PREFIX` there.
LAUNCHER_LOG_PREFIX = "ds2-launcher:"

#: Mirrors `LOG_FILE_NAME` in `crates/ds2-loader/src/lib.rs`. Both halves of this contract have
#: to change together: rename it on one side only and every run reports a false "did not load".
LOG_NAME = "ds2-loader.log"

#: The strong witness. Written from dearxan's callback, so it proves the DLL loaded AND that
#: `neuter_arxan` reached its callback -- which is the whole point of this build.
ARXAN_LINE_PREFIX = "ds2-loader: arxan"

#: The weak witness. Written at `DLL_PROCESS_ATTACH`. It proves only that the DLL was loaded.
#: Kept separate from the strong one on purpose: "loaded, and dearxan never reported" is a
#: completely different failure from "never loaded", and conflating them sends you hunting a
#: WINEDLLOVERRIDES problem that does not exist.
ATTACH_LINE_PREFIX = "ds2-loader: attach"

#: Mirrors `PROBE_LINE_PREFIX` in `crates/ds2-loader/src/arxan_probe.rs`. A different prefix from
#: the loader's on purpose: one of them is testimony about LOADING and the other is testimony
#: about an EXPERIMENT, and a run that only did the first must not be able to read as the second.
PROBE_LINE_PREFIX = "ds2-probe:"

#: Mirrors `CONFIG_LINE_PREFIX` in that module. The DLL echoes the config it read back into the
#: log under this prefix -- the path, whether the file was there at all, and every key verbatim --
#: before it acts on any of it. `ds2-loader:` rather than `ds2-probe:` because it is written even
#: when the probe is off, which is exactly the run where you most need to know what it read.
CONFIG_LINE_PREFIX = "ds2-loader: config"

#: Mirrors `CONFIG_FILE_NAME` in `crates/ds2-loader/src/arxan_probe.rs`. The DLL reads this out of
#: its own game directory -- the directory of the running executable -- so this script writes it
#: to exactly the directory it stages the DLL into. `--selftest` checks the spelling against the
#: Rust source, because a rename on one side alone turns every run into a silent "probe off".
CONFIG_NAME = "ds2-mods.toml"

#: The config the release package ships. `--release-config` stages this file verbatim instead of
#: writing one from the flags, so a run can show a player's game rather than a harness arm.
RELEASE_CONFIG = REPO_ROOT / ".github" / "dist-ds2-mods.toml"

#: Set by `--release-config`; `config_text` returns this in place of its own when it is set.
release_config_text: str | None = None

#: Mirrors `CONFIG_SECTION` and the four `KEY_*` constants in that module.
CONFIG_SECTION = "arxan_probe"
KEY_ENABLED = "enabled"
KEY_SKIP_NEUTER = "skip_neuter"
#: Mirrors `KEY_SITE` in `arxan_probe.rs`. Which function the detour goes on.
KEY_SITE = "site"

#: Mirrors `CONFIG_SECTION`/`KEY_ENABLED` in `crates/ds2-loader/src/intro_skip.rs`.
INTRO_SECTION = "intro_skip"
KEY_INTRO_ENABLED = "enabled"

#: Mirrors `CONFIG_SECTION`/`KEY_ENABLED` in `crates/ds2-loader/src/dialog_skip.rs`.
DIALOG_SECTION = "dialog_skip"
KEY_DIALOG_ENABLED = "enabled"

#: Mirrors `CONFIG_SECTION` and the two `KEY_*` in `crates/ds2-loader/src/title_skip.rs`.
TITLE_SECTION = "title_skip"
KEY_PRESS_ANY_BUTTON = "press_any_button"
KEY_PROCESS_WINDOWS = "process_windows"
KEY_HIDE_PROCESS_WINDOWS = "hide_process_windows"
KEY_TITLE_ANIMATION = "title_animation"
KEY_TITLE_SEQUENCE_GATE = "title_sequence_gate"
KEY_TITLE_SETTLE = "title_settle"
KEY_SUBSTATE_FLOORS = "substate_floors"
#: Mirrors `CONFIG_SECTION`/`KEY_SHOW_UNAVAILABLE` in `crates/ds2-loader/src/title_menu.rs`.
#: Mirrors `CONFIG_SECTION`/`KEY_ENABLED` in `crates/ds2-loader/src/boot_timeline.rs`.
#: OFF by default -- the only feature here that is. It is a measuring instrument, not a fix.
TIMELINE_SECTION = "boot_timeline"
KEY_TIMELINE_ENABLED = "enabled"

#: Mirrors `CONFIG_SECTION`/`KEY_RECORD` in `crates/ds2-loader/src/continue_flow.rs`.
#: OFF by default, for the same reason as the timeline: it measures, it does not fix.
CONTINUE_SECTION = "continue"
KEY_CONTINUE_RECORD = "record"
KEY_CONTINUE_SLOT = "slot"
KEY_CONTINUE_SILENCE = "silence"
KEY_CONTINUE_HIDE_MENUS = "hide_menus"

MENU_SECTION = "title_menu"
KEY_SHOW_UNAVAILABLE = "show_unavailable"

#: Mirrors `CONFIG_SECTION`/`KEY_ENABLED` in `crates/ds2-loader/src/menu_row.rs`.
#: OFF by default, and the ONLY key in this file where a typo leaves the feature off rather than
#: on. It is an instrument: a probe that switched itself on because a value was misspelled would
#: put an unexplained row in the pause menu and call it a measurement.
MENU_ROW_SECTION = "menu_row"
KEY_MENU_ROW_ENABLED = "enabled"

#: Mirrors `KEY_ROWS` in `crates/ds2-loader/src/menu_row.rs`. The modern key: a LIST, because the
#: System tab holds two added rows and there are four that want one. This script only ever writes
#: it COMMENTED OUT: present, it overrides `enabled` outright, and a launcher able to narrow the row
#: set produced a run missing two shipped rows that read as a regression in the DLL.
KEY_MENU_ROW_ROWS = "rows"

#: Mirrors `Row::name` for every variant of `EVERY_ROW` in `crates/ds2-loader/src/menu_row.rs`, in
#: the same order. `--selftest` checks each one appears there, because a name this script offers and
#: the DLL does not know is a row that arms nothing and says so only in a log nobody is reading yet.
MENU_ROW_ROW_NAMES = (
    "load-build-from-url",
    "load-character-from-file",
    "save-game-to-file",
    "quit-to-desktop",
)

#: Mirrors `MAX_ADDED_ROWS` in `crates/ds2-menu-row/src/api.rs`: the most rows the grid's layout
#: bind will ever go looking for (15) less the rows the System tab ships (3). Named here so the
#: commented line this script emits offers a list the DLL will not refuse.
#:
#: IT WAS 2 -- the item vector's capacity (5) less the same three -- until `ds2-menu-row` stopped
#: keeping its rows in the game's two fixed vectors and started answering the two functions that
#: read them. `--selftest` pins this against the crate, so the two cannot drift.
MENU_ROW_MAX_ADDED = 12

#: The prefix `ds2-save-file` writes. Grep for it when an export or a pick disappoints.
SAVE_FILE_LOG_PREFIX = "ds2-save-file:"

#: Mirrors `LOG_PREFIX` in `crates/ds2-save-block/src/lib.rs`. That crate installs only when the
#: `save-game-to-file` row registers, and its lines are the only evidence that a run is refusing the
#: game's own saves rather than quietly taking them. `--selftest` pins this against the crate.
SAVE_BLOCK_LOG_PREFIX = "ds2-save-block:"

#: Mirrors `HANDOFF_FILE_NAME` in `crates/ds2-save-file-core/src/handoff.rs`. The one-line file the
#: Load Character from File row writes and the loader consumes on the NEXT launch.
SAVE_FILE_HANDOFF_NAME = "ds2-load-next-save.txt"

#: Mirrors `CONFIG_SECTION`/`KEY_DIRECTORY` in `crates/ds2-loader/src/save_redirect.rs`.
SAVE_REDIRECT_SECTION = "save_redirect"
KEY_SAVE_REDIRECT_DIRECTORY = "directory"

#: Wine maps `Z:` to `/`, and the DLL runs INSIDE the prefix, so a folder on this machine reaches
#: the game as `Z:\home\you\...`. `--save-dir` takes either spelling and this is the conversion --
#: done here rather than asked of the person typing it, because a path that is one backslash wrong
#: produces a game with no saves and nothing on screen to say why.
def windows_path(path: str) -> str:
    """`/home/you/DS2 Saves` -> `Z:\\home\\you\\DS2 Saves`. A drive-lettered path is left alone."""
    text = path.strip()
    if not text:
        return ""
    # Already Windows: a drive letter and a colon, or a UNC root. Nothing to do, and guessing at it
    # would mangle the one spelling that was already correct.
    if len(text) >= 2 and text[1] == ":" or text.startswith("\\\\"):
        return text
    return "Z:" + str(Path(text).expanduser().resolve()).replace("/", "\\")

#: Mirrors `CONFIG_SECTION`/`KEY_ENABLED` in `crates/ds2-loader/src/build_import.rs`.
BUILD_IMPORT_SECTION = "build_import"

#: Mirrors `CONFIG_SECTION`/`KEY_ENABLED` in `crates/ds2-loader/src/inventory_sort.rs`, and the
#: two binding keys in `crates/ds2-inventory-sort/src/lib.rs`.
INVENTORY_SORT_SECTION = "inventory_sort"
KEY_INVENTORY_SORT_ENABLED = "enabled"
KEY_INVENTORY_SORT_KEY = "key"
KEY_INVENTORY_SORT_PAD = "pad"

#: Mirrors `CONFIG_SECTION`/`KEY_ENABLED` in `crates/ds2-loader/src/invasion_path.rs`, and the
#: live settings in `crates/ds2-invasion-path/src/config.rs`.
INVASION_PATH_SECTION = "invasion_path"
KEY_INVASION_PATH_ENABLED = "enabled"
KEY_INVASION_PATH_TOGGLE = "toggle_key"
KEY_INVASION_PATH_START_ENABLED = "start_enabled"
KEY_INVASION_PATH_MARKER_EFFECT_ID = "marker_effect_id"
KEY_INVASION_PATH_NPC_SELF_CHECK = "npc_self_check"
#: Mirrors `LOG_PREFIX` in `crates/ds2-invasion-path/src/log.rs`. Grep for it when a run
#: disappoints: `roster:` and `camera:` under this prefix are the two lines that say whether the
#: overlay found anything, and `overlay:` is the one that says whether it could draw at all.
INVASION_PATH_LOG_PREFIX = "ds2-invasion-path:"

#: Mirrors `CONFIG_SECTION`/`KEY_ENABLED` in `crates/ds2-loader/src/input_harness.rs`.
INPUT_HARNESS_SECTION = "input_harness"
KEY_INPUT_HARNESS_ENABLED = "enabled"
#: Mirrors `LOG_PREFIX` in `crates/ds2-input-harness/src/log.rs`. Grep for it to find out
#: whether the three device polls were hooked, which one owns the frame tick, and what every
#: command the run sent actually did.
INPUT_HARNESS_LOG_PREFIX = "ds2-input-harness:"
#: Mirrors `COMMAND_FILE_NAME` in `crates/ds2-input-harness/src/device.rs`. The file an agent
#: writes to drive the camera while the game is already running; it lives beside the exe.
INPUT_HARNESS_COMMAND_FILE = "ds2-input-harness-cmd.txt"
KEY_BUILD_IMPORT_ENABLED = "enabled"

#: Mirrors `CONFIG_SECTION`/`KEY_ENABLED` in `crates/ds2-loader/src/item_warn.rs`.
#:
#: OFF by default here, matching the DLL's own default, and for the DLL's own reason: the badge
#: patches the frontend's layout builder and its cell bind and has never been in front of a running
#: game. `--item-warn` is how a run turns it on, which is also the only way to change that.
ITEM_WARN_SECTION = "item_warn"
KEY_ITEM_WARN_ENABLED = "enabled"
#: Mirrors `LOG_PREFIX` in `crates/ds2-item-warn/src/lib.rs`. Grep for it when a run disappoints.
ITEM_WARN_LOG_PREFIX = "ds2-item-warn:"

#: `[seamless]` -- loading a SECOND, THIRD-PARTY mod's DLL into the same process.
#:
#: Nothing here ships that mod and nothing here copies it. The key is a path; the player installs
#: the other mod themselves, from its own download, under its own licence, next to
#: `DarkSoulsII.exe`. If the file is not there the DLL logs that and carries on without it.
#:
#: OFF by default, and unlike every other feature its key is read as opt-IN: only an exact `true`
#: arms it, because the failure direction of a typo here is "a foreign binary was loaded into the
#: game" rather than "a feature stayed off".
SEAMLESS_SECTION = "seamless"
KEY_SEAMLESS_ENABLED = "enabled"
KEY_SEAMLESS_DLL = "dll"
#: Where Seamless Co-op's own launcher injects from, so where the DLL looks by default. Mirrors
#: `DEFAULT_DLL` in `crates/ds2-loader/src/seamless.rs`.
SEAMLESS_DEFAULT_DLL = "SeamlessCoop/ds2sc.dll"
#: Mirrors `LOG_PREFIX` in `crates/ds2-loader/src/seamless.rs`.
SEAMLESS_LOG_PREFIX = "ds2-seamless:"
#: The other mod's own settings file, which it keeps next to its DLL.
SEAMLESS_SETTINGS_NAME = "ds2sc_settings.ini"
#: The key in that file that renames the save container. Mirrored in `crates/ds2-seamless`.
KEY_SEAMLESS_SAVE_EXTENSION = "save_file_extension"
#: The one container name SOTFS builds, and the extension it builds it with.
SAVE_FILE_STEM = "DS2SOFS0000"
VANILLA_SAVE_EXTENSION = "sl2"
#: Where Proton keeps the prefix for this appid, and where the game's saves land inside it.
#: `ds2-teardown.py` names the same prefix; the game's own log lines name the same folders as
#: `C:\users\steamuser\AppData\Roaming\DarkSoulsII\<account>\`.
PREFIX_DIR = (
    Path.home() / ".local" / "share" / "Steam" / "steamapps" / "compatdata" / APPID / "pfx"
)
SAVE_ROOT = (
    PREFIX_DIR
    / "drive_c"
    / "users"
    / "steamuser"
    / "AppData"
    / "Roaming"
    / "DarkSoulsII"
)
#: Mirrors `LOG_PREFIX` in `crates/ds2-menu-row/src/lib.rs`. Named here because the generated
#: config tells the reader which line to look for, and a prefix that drifted would send them
#: looking for a line that is never written.
MENU_ROW_LOG_PREFIX = "ds2-menu-row:"
#: The prefix `ds2-build-import` writes. Grep for it when a run disappoints.
BUILD_IMPORT_LOG_PREFIX = "ds2-build-import:"

#: Mirrors `CONFIG_SECTION` and the four `KEY_*` in `crates/ds2-loader/src/offline.rs`.
#: ON by default, unlike every other feature's switch here, because this workspace patches
#: `.text` in a running copy of the game and FromSoftware's servers are what watch for that.
OFFLINE_SECTION = "offline"
KEY_OFFLINE_ENABLED = "enabled"
KEY_PIN_FLAG = "pin_flag"
KEY_REPORT_OFFLINE = "report_offline"
KEY_BLOCK_SOCKETS = "block_sockets"


#: The only save file name SOTFS builds, and the tool that can read its slot table.
#: `ds2-sl2.py` owns every fact about the `.sl2` format; this module only asks it questions.
SAVE_FILE_NAME = "DS2SOFS0000.sl2"
SL2_TOOL = REPO_ROOT / "scripts" / "ds2-sl2.py"

KEY_POLL_INTERVAL_MS = "poll_interval_ms"
KEY_HEARTBEAT_INTERVAL_MS = "heartbeat_interval_ms"

#: Defaults for the two LIVE keys, written into the generated file as commented-out lines so the
#: file documents them without changing behaviour. Mirrors `DEFAULT_*` in the same module.
DEFAULT_POLL_INTERVAL_MS = 1000
DEFAULT_HEARTBEAT_INTERVAL_MS = 10000

#: `crates/ds2-loader/src/crash_logging.rs`'s section and keys.
CRASH_SECTION = "crash_logging"
KEY_CRASH_ENABLED = "enabled"
KEY_FAULT_AFTER_MS = "fault_after_ms"
KEY_REINSTALL_FILTER_AFTER_MS = "reinstall_filter_after_ms"

#: The DLL's default, written explicitly so the file shows it. See the config comment below for
#: why re-asserting the filter is not optional in this game.
DEFAULT_REINSTALL_FILTER_AFTER_MS = 5000

#: `fault_after_ms = 0` means "never", and is what every run that is not a crash test writes.
NO_FAULT_MS = 0

#: The five files `ds2-crash-logging-core` writes next to the executable. Checked by name after a
#: crash test, because "the game died" is not evidence that the crash LOGGER worked -- a game that
#: crashed on its own looks identical from outside.
CRASH_ARTIFACTS = (
    "ds2-crash-log.txt",
    "ds2-crash-latest.txt",
    "ds2-crash-breadcrumb-latest.txt",
    "ds2-crash-modules.txt",
    "ds2-crash-minidump.dmp",
)

#: CLI arm -> (`[arxan_probe]` settings to write, the `arm=` token the DLL must report back).
#:
#: THE SECOND HALF OF EACH ENTRY IS A GUARD, not decoration, and it did not stop being one when
#: these moved out of the environment. A config file has its own ways to be wrong: it can fail to
#: be written, be written to the wrong directory (a game dir that moved, a second install), or be
#: left over from a previous run against a DLL that no longer reads it. A run that lost the file
#: entirely installs no probe and is caught by the missing install line -- but a run against a
#: STALE file would quietly execute whichever arm that file names and produce a perfectly
#: well-formed verdict for an experiment nobody asked for. That is the failure this comparison
#: exists to make impossible, and it is why the arm is read back out of the log.
#: Which site the probe hooks, mirroring `Site` in `arxan_probe.rs`.
#:
#: `m1` is the CONTROL and is the default. `scripts/ds2-arxan-chain.py` terminates at hop 0 on
#: that address -- its own prologue is at its own entry -- so Arxan has no presence there and a
#: surviving detour says only that hooking works in this game. `redirected` is `applySpEffect`,
#: whose five entry bytes are Arxan's own redirect; it is the only site where survival is
#: evidence about Arxan. Keeping the control as the default means an operator who forgets the
#: flag gets a run that is merely uninformative rather than one that is quietly mislabelled.
PROBE_SITES: tuple[str, ...] = ("m1", "redirected")

PROBE_ARMS: dict[str, tuple[dict[str, bool], str | None]] = {
    "off": ({KEY_ENABLED: False, KEY_SKIP_NEUTER: False}, None),
    "neuter": ({KEY_ENABLED: True, KEY_SKIP_NEUTER: False}, "neuter-arxan"),
    "skip-neuter": ({KEY_ENABLED: True, KEY_SKIP_NEUTER: True}, "skip-neuter-arxan"),
}

#: How long to keep reading the log after the probe says it is installed. The probe heartbeats
#: every 10s, so the default is 18 of them. It is a WINDOW and not a proof of anything past its
#: end, which is why the verdict block prints the number rather than rounding it to "it survived".
OBSERVE_SECONDS = 180.0

#: Native first, then builtin. Wine prefers its own `dinput8` without this and the proxy is
#: simply never mapped.
DLL_OVERRIDE = "dinput8=n,b"

#: How long to wait for the DLL to speak. DS2 boots through Proton and dearxan analyses 48 Arxan
#: stubs single-threaded before its callback runs; neither has been timed on this machine, so
#: this is a deliberately loose bound, not a measurement.
TESTIMONY_BUDGET_SECONDS = 240.0
POLL_SECONDS = 0.5

#: /proc/<pid>/comm is truncated to 15 characters and "DarkSoulsII.exe" is exactly 15, so the
#: exact-comm match works. `pgrep -f 'DarkSoulsII.exe'` does NOT: it matches this script's own
#: command line, which contains the string, and reports the game running before it has started.
GAME_COMM = "DarkSoulsII.exe"

EXIT_OK = 0
EXIT_NO_TESTIMONY = 2
EXIT_ERROR = 3
#: The probe was asked for and did not produce a verdict: it never installed, it refused the site,
#: it never heartbeat, or the arm in the log is not the arm that was requested. Distinct from
#: EXIT_NO_TESTIMONY because the DLL may have loaded and reported perfectly well -- the thing that
#: did not happen is the EXPERIMENT, and those two send you to different places to look.
EXIT_NO_PROBE_VERDICT = 4

#: `--crash-test` ran, the game died as asked, and the crash logger did NOT produce its evidence.
#: Distinct from EXIT_ERROR because the run itself was fine -- the logger is what failed.
EXIT_NO_CRASH_EVIDENCE = 5


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def pgrep_exact(comm: str) -> list[int]:
    """PIDs whose exact comm is `comm`. Empty on no match; `pgrep` exits 1 for that."""
    proc = subprocess.run(
        ["pgrep", "-x", comm], capture_output=True, text=True, timeout=10
    )
    return [int(line) for line in proc.stdout.split() if line.isdigit()]


def steam_running() -> bool:
    """Is the Steam client up? Native Linux only -- this repo's game runs through Proton."""
    return bool(pgrep_exact("steam") or pgrep_exact("steamwebhelper"))


class LogTail:
    """Read only what THIS run appended, surviving the DLL's startup log rotation.

    `ds2_game_base::log` renames `<log>` to `<log>.prev` and truncates on the DLL's first write,
    so a plain offset would either miss the new file or replay the old one. Remembering the
    inode as well makes "everything since launch" exact. Both rotation shapes are handled, and
    missing the second one cost `../er-mods-rs` a live run:

      * REPLACED           -- new inode, so read the whole new file.
      * TRUNCATED IN PLACE -- same inode, size dropped below our offset. Seeking to the old
                              offset lands past EOF and reads NOTHING, so the DLL's startup
                              lines are invisible and a perfectly healthy run is reported silent.
    """

    def __init__(self, path: Path) -> None:
        self.path = path
        try:
            stat = path.stat()
            self.inode: int | None = stat.st_ino
            self.offset = stat.st_size
        except OSError:
            self.inode, self.offset = None, 0

    def new_text(self) -> str:
        try:
            stat = self.path.stat()
        except OSError:
            return ""
        rotated = stat.st_ino != self.inode or stat.st_size < self.offset
        start = 0 if rotated else self.offset
        try:
            with self.path.open("rb") as handle:
                handle.seek(min(start, stat.st_size))
                data = handle.read()
        except OSError:
            return ""
        # A LINE IS EVIDENCE ONLY ONCE IT IS TERMINATED. A read landing mid-write returns a
        # PREFIX, and a prefix of "ds2-loader: arxan status=ok detected=true ..." is
        # indistinguishable from a DLL that reported less than it did. Hand back whole lines
        # only; the remainder arrives complete on the next poll, milliseconds later.
        end = data.rfind(b"\n")
        if end < 0:
            return ""
        consumed = (0 if rotated else start) + end + 1
        self.inode, self.offset = stat.st_ino, consumed
        return data[: end + 1].decode("utf-8", errors="replace")


def await_testimony(tail: LogTail) -> dict:
    """Block until the DLL reports its Arxan result, or the budget runs out.

    Verdicts:
      confirmed        -- the `ds2-loader: arxan` line is on disk. This run is what it says.
      attached-silent  -- the DLL loaded and said so, but dearxan's callback never reported.
                          The proxy and the override WORKED; the Arxan step is what did not.
      silent           -- no line at all. Most likely the DLL was never mapped.
    """
    deadline = time.monotonic() + TESTIMONY_BUDGET_SECONDS
    attach_line: str | None = None
    game_seen = False
    while True:
        chunk = tail.new_text().splitlines()
        for index, line in enumerate(chunk):
            stripped = line.strip()
            if stripped.startswith(ARXAN_LINE_PREFIX):
                return {
                    "status": "confirmed",
                    "line": stripped,
                    "attach_line": attach_line,
                    "waited": TESTIMONY_BUDGET_SECONDS - (deadline - time.monotonic()),
                    "leftover": chunk[index + 1 :],
                }
            if stripped.startswith(ATTACH_LINE_PREFIX):
                attach_line = stripped

        alive = bool(pgrep_exact(GAME_COMM))
        game_seen = game_seen or alive
        if game_seen and not alive:
            # The game came up and went away. Waiting longer cannot produce a line, because
            # there is no longer a process to write one. Re-read once first: the exit path and
            # the last write race, and losing that race would report silence over real evidence.
            chunk = tail.new_text().splitlines()
            for index, line in enumerate(chunk):
                stripped = line.strip()
                if stripped.startswith(ARXAN_LINE_PREFIX):
                    return {
                        "status": "confirmed",
                        "line": stripped,
                        "attach_line": attach_line,
                        "waited": TESTIMONY_BUDGET_SECONDS - (deadline - time.monotonic()),
                        "leftover": chunk[index + 1 :],
                    }
                if stripped.startswith(ATTACH_LINE_PREFIX):
                    attach_line = stripped
            return {
                "status": "attached-silent" if attach_line else "silent",
                "attach_line": attach_line,
                "game_exited": True,
                "game_seen": True,
            }

        if time.monotonic() >= deadline:
            return {
                "status": "attached-silent" if attach_line else "silent",
                "attach_line": attach_line,
                "game_exited": False,
                "game_seen": game_seen,
            }
        time.sleep(POLL_SECONDS)


# ================================================================================================
# THE M1 PROBE: reading a verdict out of the DLL's own lines.
#
# Nothing below infers anything. It parses `ds2-probe:` lines and reports what they say. The one
# judgement it makes -- "survived" vs "reverted" -- is a restatement of two fields on the last
# heartbeat, and the block prints those fields next to it so the restatement can be checked.
# ================================================================================================


def fields(line: str) -> dict[str, str]:
    """The `key=value` tokens of a log line.

    Byte windows are logged as `expected=[48 89 5c ...]`, which splits into an `expected=[48`
    token followed by bare hex tokens. The bare ones have no `=` and are dropped; the truncated
    `expected` is never read by anything here. That is deliberate rather than sloppy -- the hex is
    for a human comparing two windows by eye, and the scalars are what this code reasons about.
    """
    parsed: dict[str, str] = {}
    for token in line.split():
        key, sep, value = token.partition("=")
        if sep and key not in parsed:
            parsed[key] = value
    return parsed


def absorb_probe_line(line: str, state: dict) -> None:
    """Sort one log line into `state`. Non-probe lines are ignored."""
    if not line.startswith(PROBE_LINE_PREFIX):
        return
    body = line[len(PROBE_LINE_PREFIX) :].strip()
    kind = body.split(" ", 1)[0] if body else ""
    parsed = fields(line)

    if kind == "install-failed":
        state["install_failed"] = state["install_failed"] or line
    elif kind == "VOID":
        state["void"] = state["void"] or line
    elif kind == "install":
        # Three lines share this word. `rva=` is on the first (the one that names the arm and the
        # address), `minhook=` on the one that reports what was written. Matching on a field
        # rather than on word position means a reordered line does not become a parse failure.
        if "rva" in parsed:
            state["install"] = state["install"] or line
            state["arm"] = state["arm"] or parsed.get("arm")
            # A DLL that predates the `site` key reports no site at all, which is precisely the
            # case that has to be caught rather than defaulted.
            state["site"] = state["site"] or parsed.get("site")
        elif "minhook" in parsed:
            state["patched"] = state["patched"] or line
        elif "prologue-match" in parsed:
            state["prologue"] = state["prologue"] or line
    elif kind == "watching":
        state["watching"] = state["watching"] or line
        state["arm"] = state["arm"] or parsed.get("arm")
    elif kind == "heartbeat":
        state["heartbeats"].append(line)
    elif kind in ("SITE", "TRAMP"):
        state["events"].append(line)
    elif kind == "config":
        # The probe re-read its config mid-run and something changed, or someone edited a
        # startup-only key and was told it does not apply. Either way the run was touched while
        # it was being measured, and the verdict block must say so rather than quietly average
        # over it.
        state["config_events"].append(line)
    elif kind == "detach":
        state["detach"] = line


def new_probe_state() -> dict:
    return {
        "install": None,
        "prologue": None,
        "patched": None,
        "watching": None,
        "install_failed": None,
        "void": None,
        "arm": None,
        "site": None,
        "heartbeats": [],
        "events": [],
        "config_events": [],
        "detach": None,
        "game_exited": False,
        "observed": 0.0,
    }


def watch_probe(tail: LogTail, seconds: float, leftover: Sequence[str] = ()) -> dict:
    """Read probe lines until the observation window closes or the game goes away.

    `leftover` is the tail of the chunk `await_testimony` was reading when it found the Arxan
    line, and it is not an optimisation -- without it the install lines are LOST.

    MEASURED 2026-08-27, first real M1 run, arm A. The run reported "NO VERDICT -- the probe never
    installed", while the log on disk plainly contained

        ds2-probe: install arm=neuter-arxan base=0x140000000 rva=0x00832e70 va=0x140832e70

    `LogTail.new_text()` consumes a whole chunk and advances its offset past ALL of it, so a
    `return` from the middle of the loop that walks that chunk throws away every line after the
    one it returned on. The probe writes its install lines from the same Arxan callback that
    writes `ds2-loader: arxan`, milliseconds apart, so both land in one read essentially always.
    `watch_probe` then started AFTER them, collected eighteen heartbeats, and refused a verdict --
    correctly, by its own "no install line, no verdict" rule. The guard behaved; the evidence had
    already been destroyed upstream.

    The comment at the call site anticipated exactly this ("the probe's install lines may already
    be in the buffer this loop is about to read") and concluded that passing the same `tail` was
    enough. It is not: the tail object is shared, but the chunk already read out of it is not.
    """
    state = new_probe_state()
    for line in leftover:
        absorb_probe_line(line.strip(), state)
    started = time.monotonic()
    deadline = started + seconds
    game_seen = False
    while True:
        for line in tail.new_text().splitlines():
            absorb_probe_line(line.strip(), state)

        alive = bool(pgrep_exact(GAME_COMM))
        game_seen = game_seen or alive
        if game_seen and not alive:
            # Same race as `await_testimony`: the detach line and the process disappearing happen
            # at the same moment, and losing that race would drop the one line that says the exit
            # was orderly. Drain once more before concluding.
            time.sleep(POLL_SECONDS)
            for line in tail.new_text().splitlines():
                absorb_probe_line(line.strip(), state)
            state["game_exited"] = True
            break

        if time.monotonic() >= deadline:
            break
        time.sleep(POLL_SECONDS)
    state["observed"] = time.monotonic() - started
    return state


def probe_block(
    requested: str, state: dict, expected_site: str | None = None
) -> tuple[str, int]:
    """Render the verdict for one arm, and the exit code that goes with it.

    Returns `EXIT_OK` whenever the experiment RAN, whether or not the detour survived: a detour
    that Arxan reverted is a successful experiment with an unwelcome answer, and exiting non-zero
    on it would train whoever reads this into treating the real finding as a tooling failure.
    Non-zero means no verdict was produced at all.
    """
    _, expected_arm = PROBE_ARMS[requested]
    head = ["```", "================== M1: ARXAN vs A MINHOOK DETOUR =================="]
    tail_rule = ["==================================================================", "```"]

    def refused(reason: str, detail: list[str]) -> tuple[str, int]:
        lines = head + [f"  NO VERDICT     {reason}"] + [f"  {d}" for d in detail] + tail_rule
        return "\n".join(lines), EXIT_NO_PROBE_VERDICT

    if state["void"]:
        return refused(
            "the probe refused the hook site and patched nothing",
            [
                state["void"],
                "",
                "The five bytes at the hook site were not the prologue recorded in",
                "`ds2-rva::ARXAN_PROBE_HOOK_SITE_PROLOGUE`, so something reached that",
                "function before the probe did. Anything measured after that would have",
                "been a measurement of THAT, not of Arxan. Check the game build against",
                "`ds2_rva::BUILD_ID`, and check for another mod in the game directory.",
            ],
        )

    if not state["install"]:
        detail = [state["install_failed"]] if state["install_failed"] else []
        return refused(
            "the probe never installed",
            detail
            + [
                "",
                f"No `{PROBE_LINE_PREFIX} install ... rva=` line appeared.",
                "",
                f"Check the loader's `{CONFIG_LINE_PREFIX}` lines, which say what it read and",
                "from where:",
                f"  status=MISSING  -- the DLL looked in a different directory than this script",
                f"                     wrote to. Compare the path on that line against",
                f"                     {GAME_DIR / CONFIG_NAME}.",
                f"  status=found and {KEY_ENABLED}=\"false\"",
                "                  -- the file that was read is not the one this run wrote.",
                f"  a REJECTED line -- the file was read and that key could not be used.",
                "",
                "If instead there are no config lines at all, the DLL that loaded is an older",
                "build than the one staged: check the sha256 above.",
            ],
        )

    if expected_site is not None and state.get("site") != expected_site:
        reported = state.get("site")
        return refused(
            f"WRONG SITE -- asked for {expected_site!r}, the DLL hooked {reported or 'an unnamed site'!r}",
            [
                "The verdict is withheld because a run that hooks a different function than the",
                "one requested is not a weaker version of the experiment, it is a DIFFERENT",
                "experiment wearing its label. `m1` is a clean function Arxan never touched, so",
                "a detour there surviving says nothing about Arxan; `redirected` overwrites",
                "Arxan's own five entry bytes.",
                "",
                "A DLL reporting NO site predates the `site` key entirely -- rebuild it:",
                "  cargo xwin build --release --target x86_64-pc-windows-msvc -p ds2-loader",
                "",
                "This check exists because it already happened once, on 2026-08-26: a stale",
                "staged DLL silently ran the control while the launcher reported the arm it was",
                "asked for, and only an UNKNOWN-key line in the log gave it away.",
            ],
        )

    if state["arm"] != expected_arm:
        return refused(
            f"WRONG ARM -- asked for {expected_arm!r}, the DLL ran {state['arm']!r}",
            [
                state["install"],
                "",
                "This is the dangerous case, which is why it is a hard failure rather than a",
                "note: the run is well-formed and its numbers are real, but they answer a",
                "different question than the one that was asked.",
                "",
                "The config file this run wrote is quoted in the block above. If the DLL read a",
                "different one, its own `" + CONFIG_LINE_PREFIX + "` line names the path it",
                "read -- most likely a second game install, or a stale file the DLL reached",
                "before this script rewrote it.",
            ],
        )

    if not state["heartbeats"]:
        return refused(
            "the probe installed but never reported a heartbeat",
            [
                state["install"],
                "",
                f"Observed {state['observed']:.1f}s. The probe heartbeats every 10s, so either",
                "the window was too short, or the poller thread did not run. Every event line",
                "seen, if any, follows.",
            ]
            + state["events"],
        )

    last = fields(state["heartbeats"][-1])
    hits = last.get("hits", "?")
    site = last.get("site", "?")
    tramp = last.get("tramp", "?")
    site_diverged = last.get("site-diverged", "?")
    tramp_diverged = last.get("tramp-diverged", "?")
    site_ok = site == "intact" and site_diverged == "0"
    tramp_ok = tramp == "intact" and tramp_diverged == "0"
    fired = hits not in ("0", "?")

    lines = head + [
        f"  arm            {state['arm']}   (requested --probe {requested})",
        f"  observed       {state['observed']:.1f}s, {len(state['heartbeats'])} heartbeats,"
        f" game exited: {'yes' if state['game_exited'] else 'no'}",
        "",
        f"  {state['install']}",
    ]
    if state["prologue"]:
        lines.append(f"  {state['prologue']}")
    if state["patched"]:
        lines.append(f"  {state['patched']}")
    lines += [
        "",
        "  THE FOUR MEASUREMENTS, from the last heartbeat:",
        f"    {state['heartbeats'][-1]}",
        "",
        f"    1. detour fired    {'YES' if fired else 'NO'}   hits={hits}",
        f"    2. hook site       {site}   ({site_diverged} divergence(s))",
        f"    3. trampoline      {tramp}   ({tramp_diverged} divergence(s))",
        f"    4. arm             {state['arm']}",
    ]

    if state["events"]:
        lines += ["", "  STATE CHANGES (each with the observed bytes):"]
        lines += [f"    {event}" for event in state["events"]]

    if state["config_events"]:
        # NOT a footnote. The config file was edited while the measurement was running, so the
        # window above is not homogeneous, and a reader comparing two arms needs to know that
        # before comparing anything.
        lines += [
            "",
            "  THE CONFIG FILE WAS TOUCHED DURING THIS WINDOW:",
        ]
        lines += [f"    {event}" for event in state["config_events"]]
        lines += [
            "    The arm cannot change mid-run and the DLL says so when asked to; a poll or",
            "    heartbeat interval CAN, and if one did, the cadence above is not uniform.",
        ]

    lines += [
        "",
        "  exit           "
        + (
            state["detach"]
            if state["detach"]
            else "no `ds2-probe: detach` line -- the process did not wind down through"
        ),
    ]
    if not state["detach"]:
        lines.append(
            "                 ExitProcess. It crashed, was killed, or is still running; the"
        )
        lines.append("                 last heartbeat above is the time of death if it died.")

    lines += ["", "  READING"]
    if site_ok and tramp_ok and fired:
        lines.append(
            f"    The detour SURVIVED and FIRED for the whole {state['observed']:.0f}s window."
        )
    elif site_ok and tramp_ok and not fired:
        lines += [
            "    The patch was never touched -- AND the function was never called.",
            "    The hook site held, but a detour that never runs is not much of a test",
            "    of whether a running one survives. Play further into the game, or move to",
            "    `ds2_rva::ARXAN_PROBE_HOOK_SITE_BACKUP`.",
        ]
    else:
        if not site_ok:
            lines.append("    THE HOOK SITE WAS REVERTED. Read the SITE lines above for when and")
            lines.append("    to what. This is the finding the experiment was built to catch.")
        if not tramp_ok:
            lines += [
                "    THE TRAMPOLINE WAS CORRUPTED. Note that this is exactly the failure a hit",
                "    counter alone would have misread: the counter goes quiet while the hook",
                "    site still looks pristine, and the obvious conclusion -- 'the function",
                "    stopped being called' -- would have been wrong.",
            ]

    if requested == "neuter":
        lines += [
            "",
            "    THIS IS HALF THE EXPERIMENT. dearxan neutered Arxan's 48 stubs before the",
            "    probe installed, so this says nothing about whether Arxan was ever a threat.",
            "    Run `--probe skip-neuter` for the other half.",
        ]
    else:
        lines += [
            "",
            "    THIS IS THE ARM WITHOUT dearxan: Arxan's 48 stubs were live for the whole",
            "    window. Compare against a `--probe neuter` run before concluding whether",
            "    dearxan is load-bearing for hooking.",
        ]

    lines += [
        "",
        "  NOT claimed    anything past the window above; that every Arxan check ran during",
        "                 it; or anything about any hook site other than the one named above.",
    ]
    return "\n".join(lines + tail_rule), EXIT_OK


def running_block(context: dict) -> str:
    lines = [
        "```",
        "============= DARK SOULS II IS RUNNING, WITH OUR DLL IN IT =============",
        f"  started        {context['started']}",
        f"  appid          {APPID}",
        f"  override       WINEDLLOVERRIDES={DLL_OVERRIDE}",
        "",
        f"  staged         {context['staged']}",
        f"  sha256         {context['sha256']}",
        f"  game pids      {context['game_pids'] or '<none right now>'}",
        "",
        # THE CONFIGURATION UNDER TEST, VERBATIM, in the block that gets copy-pasted. The arm is
        # the variable the whole experiment turns on and it no longer travels in a command line
        # anyone can see, so the transcript has to carry the file itself or it carries nothing.
        f"  config         {context['config_path']}",
        quoted_config(context["config"], indent="    | "),
        "",
        "  PROVEN BY      the DLL's own log line, not by the process existing:",
        f"    {context['testimony']}",
    ]
    if context.get("attach_line"):
        lines.append(f"    {context['attach_line']}")
    lines += [
        f"  log            {context['log']}",
        f"  waited         {context['waited']:.1f}s",
        "",
        "  NOT claimed    window visible / menu reached / input working / Arxan actually",
        "                 defeated. This block says the proxy loaded and dearxan reported;",
        "                 read the line above for WHAT it reported.",
        "========================================================================",
        "```",
    ]
    return "\n".join(lines)


def failed_block(reason: str, detail: list[str]) -> str:
    lines = [
        "```",
        "================ DARK SOULS II: NO DLL TESTIMONY ================",
        f"  reason  {reason}",
    ]
    lines.extend(f"  {line}" for line in detail)
    lines += ["=================================================================", "```"]
    return "\n".join(lines)


#: Files whose modification invalidates a built DLL. Cargo manifests count: a feature or
#: dependency change alters the binary without any `.rs` being touched.
SOURCE_GLOBS: tuple[str, ...] = ("crates/**/*.rs", "crates/**/Cargo.toml", "Cargo.toml")


def stale_sources() -> list[Path]:
    """Source files newer than the built DLL.

    Freshness is checked rather than assumed because `stage()` copies whatever happens to sit at
    `BUILT_DLL`, and a DLL built from older code is the one failure that produces a confident,
    well-formatted, entirely wrong verdict. `--release` is the profile that gets staged; building
    `dev` leaves this file untouched and is exactly how the mismatch arises.
    """
    if not BUILT_DLL.is_file():
        return []
    built = BUILT_DLL.stat().st_mtime
    newer = []
    for pattern in SOURCE_GLOBS:
        for path in REPO_ROOT.glob(pattern):
            if "target" in path.parts:
                continue
            if path.is_file() and path.stat().st_mtime > built:
                newer.append(path)
    return newer


def preflight(dry_run: bool) -> list[str]:
    """Return the problems that make a real run impossible. Empty list means go."""
    problems: list[str] = []
    if not BUILT_DLL.is_file():
        problems.append(
            f"built DLL not found: {BUILT_DLL}\n"
            "    build it: cargo xwin build --release --target x86_64-pc-windows-msvc -p ds2-loader"
        )
    else:
        stale = stale_sources()
        if stale:
            shown = ", ".join(sorted(q.name for q in stale[:4]))
            more = f" (+{len(stale) - 4} more)" if len(stale) > 4 else ""
            problems.append(
                f"the built DLL is OLDER than {len(stale)} source file(s): {shown}{more}\n"
                f"    {BUILT_DLL}\n"
                "    A stale DLL does not fail loudly -- it runs the PREVIOUS behaviour under\n"
                "    the CURRENT config and reports a verdict for an experiment nobody ran.\n"
                "    That happened on 2026-08-26: a run requested the `redirected` probe site,\n"
                "    the staged DLL predated that key, and the log shows it quietly hooking the\n"
                "    m1 CONTROL instead. Only the DLL's own UNKNOWN-key line gave it away.\n"
                "    rebuild: cargo xwin build --release --target x86_64-pc-windows-msvc -p ds2-loader"
            )
    if not GAME_DIR.is_dir():
        problems.append(f"game directory not found: {GAME_DIR}")
    elif not os.access(GAME_DIR, os.W_OK):
        problems.append(f"game directory is not writable: {GAME_DIR}")
    if not dry_run and shutil.which("steam") is None:
        problems.append("`steam` is not on PATH")
    return problems


def launch_env(probe: str) -> dict[str, str]:
    """The environment a run needs, over this process's own.

    ONE VARIABLE, and it does not depend on the arm. `WINEDLLOVERRIDES` is the only setting that
    genuinely has to travel through Steam, because Wine reads it to decide whether to map our DLL
    at all -- before there is a DLL running to read anything. Everything the DLL itself decides
    now comes out of the config file; see `config_text`.
    """
    del probe  # the arm is in the config file now, not in the environment
    return {"WINEDLLOVERRIDES": DLL_OVERRIDE}


def host_path_of(win_path):
    """The host path for a `Z:`-rooted Windows path, or None when it is not one.

    Wine maps `Z:` to `/`, which is the only drive this can translate without reading the prefix's
    `dosdevices`. Anything else returns None and the caller SKIPS its check rather than guessing --
    a redirect to `C:\\...` is perfectly valid, it just cannot be inspected from here.
    """
    p = win_path.strip().strip('"')
    if len(p) < 2 or p[1] != ":" or p[0].upper() != "Z":
        return None
    return Path("/" + p[2:].replace("\\", "/").lstrip("/"))


def save_bytes_from(host_path):
    """The `DS2SOFS0000.sl2` bytes named by `host_path`, or None with a reason printed.

    Mirrors what the DLL does at staging time -- see `ds2-save-redirect`'s `stage` module -- so the
    slot table the launcher prints is the one the game will see. Archives are read with the tools
    already on this machine rather than by reimplementing three container formats in the launcher:
    the DLL does that itself, in Rust, because it has no shell to call out to.
    """
    suffix = host_path.suffix.lower()
    if suffix == ".sl2":
        return host_path.read_bytes()
    if suffix == ".zip":
        import zipfile

        with zipfile.ZipFile(host_path) as archive:
            names = [n for n in archive.namelist()
                     if n.rsplit("/", 1)[-1].lower() == SAVE_FILE_NAME.lower()]
            if len(names) != 1:
                print(f"[slots] {len(names)} copies of {SAVE_FILE_NAME} in the archive; not reading slots.")
                return None
            return archive.read(names[0])
    if suffix in (".7z", ".rar"):
        # LIST FIRST, then ask for the member by its FULL name. A bare `DS2SOFS0000.sl2` matches
        # nothing when the archive stores it as `DarkSoulsII/<steamid>/DS2SOFS0000.sl2`, which is
        # the ordinary shape of a downloaded save and is how this was caught.
        lister = ["7z", "l", "-slt", "-ba", str(host_path)] if suffix == ".7z" else \
                 ["unrar", "lb", str(host_path)]
        listed = subprocess.run(lister, capture_output=True, text=True)
        if listed.returncode:
            return None
        if suffix == ".7z":
            names = [line[7:].strip() for line in listed.stdout.splitlines()
                     if line.startswith("Path = ")]
        else:
            names = [line.strip() for line in listed.stdout.splitlines() if line.strip()]
        names = [n for n in names
                 if n.replace("\\", "/").rsplit("/", 1)[-1].lower() == SAVE_FILE_NAME.lower()]
        if len(names) != 1:
            print(f"[slots] {len(names)} copies of {SAVE_FILE_NAME} in the archive; not reading slots.")
            return None
        # `-so`/`p` write the member to stdout, so nothing is extracted to disk to read a table.
        argv = ["7z", "e", "-so", str(host_path), names[0]] if suffix == ".7z" else \
               ["unrar", "p", "-inul", str(host_path), names[0]]
        r = subprocess.run(argv, capture_output=True)
        if r.returncode or not r.stdout:
            return None
        return r.stdout
    return None



def config_text(
    probe: str,
    fault_after_ms: int = NO_FAULT_MS,
    site: str = "m1",
    intro_skip: bool = True,
    dialog_skip: bool = True,
    press_any_button: bool = True,
    process_windows: bool = True,
    hide_process_windows: bool = True,
    title_animation: bool = True,
    title_sequence_gate: bool = True,
    title_settle: bool = True,
    substate_floors: bool = True,
    show_unavailable: bool = False,
    boot_timeline: bool = False,
    continue_record: bool = False,
    continue_slot: int = -1,
    continue_silence: bool = True,
    continue_hide_menus: bool = True,
    offline: bool = True,
    block_sockets: bool = True,
    inventory_sort: bool = True,
    inventory_sort_key: str = "F7",
    inventory_sort_pad: str = "lthumb",
    item_warn: bool = False,
    seamless: bool = False,
    seamless_dll: str = SEAMLESS_DEFAULT_DLL,
    invasion_path: bool = False,
    invasion_path_key: str = "semicolon",
    invasion_path_start_enabled: bool = False,
    invasion_path_marker_effect_id: int = 0,
    invasion_path_npc_self_check: bool = False,
    input_harness: bool = False,
    save_directory: str = "",
    menu_rows_all: bool = False,
    menu_rows_no_save: bool = False,
    launcher_dlls: tuple[str, ...] = (),
) -> str:
    """The exact bytes of `<Game>/ds2-mods.toml` for this arm.

    Deterministic: the same arm produces the same file every time, with no timestamp and no
    hostname, so two runs of the same arm are trivially comparable and `--selftest` can assert on
    the content rather than around it.
    """
    if release_config_text is not None:
        return release_config_text
    settings, _ = PROBE_ARMS[probe]
    # THE LIST KEY OVERRIDES `enabled`, AND IT IS NEVER WRITTEN FROM HERE. It was, through a
    # `--rows` flag, and that flag cost a session: a run launched with two of the four names
    # narrowed the menu to two rows, and the missing pair read as a regression in the DLL rather
    # than as the launcher having been told to leave them out. Every row is on by default and the
    # launcher's job is to launch the default; a player who wants fewer edits this line in the file
    # it is commented into, where the choice is visible next to the thing it changes.
    every_row = ", ".join(f'"{name}"' for name in MENU_ROW_ROW_NAMES[:MENU_ROW_MAX_ADDED])
    if menu_rows_all:
        # ALL of them or none, never a subset, which is the whole lesson of the comment above: a
        # subset written from here looks like a DLL that lost rows. `--all-menu-rows` exists because
        # two features can only be reached through a row -- the save-file pair -- and one of them,
        # `ds2-save-block`, installs only when `save-game-to-file` registers. Without this there is no
        # way to launch a run that exercises either.
        menu_row_rows_line = (
            f"# WRITTEN BY --all-menu-rows: every row this table knows, in the default order.\n"
            f"# The game's own saving is OFF in this run, because `save-game-to-file` is among them.\n"
            f"{KEY_MENU_ROW_ROWS} = [{every_row}]"
        )
    elif menu_rows_no_save:
        # Every row EXCEPT the one that turns the game's own saving off.
        #
        # This is a named mode rather than an arbitrary subset, which is the distinction the
        # comment above cares about: a run missing two rows nobody asked to remove looked like a
        # DLL regression, and this removes exactly one, for a stated reason, and says so in the
        # file. `save-game-to-file` registering is what installs `ds2-save-block`, and that block
        # refuses every save the game makes for itself -- no autosave, nothing at a bonfire,
        # nothing on the way out. Measured 2026-09-24: a session played under it logged
        # `refused a save kind=10 ... nothing was written` and the container's mtime never moved,
        # so the character's progress was being dropped on the floor.
        kept = [name for name in MENU_ROW_ROW_NAMES[:MENU_ROW_MAX_ADDED] if name != "save-game-to-file"]
        menu_row_rows_line = (
            f"# WRITTEN BY --no-save-file-row: every row except `save-game-to-file`, so the\n"
            f"# game saves itself normally. That row is the only thing that installs\n"
            f"# `ds2-save-block`, and it is off here for exactly that reason.\n"
            f"{KEY_MENU_ROW_ROWS} = [" + ", ".join(f'"{name}"' for name in kept) + "]"
        )
    else:
        menu_row_rows_line = (
            f"# {KEY_MENU_ROW_ROWS} = ["
            + every_row
            + f"]   # at most {MENU_ROW_MAX_ADDED} of: "
            + ", ".join(MENU_ROW_ROW_NAMES)
        )
    # `[seamless] enabled` is NOT folded in here. The injector does that itself, from the
    # `[seamless]` keys written above, and doing it in both places would put the same path in the
    # list twice -- which its deduplication would survive, but only by silently disagreeing with
    # the file a human is reading. One writer per fact.
    launcher_dll_list = ", ".join(f'"{name}"' for name in launcher_dlls)
    crash_banner = (
        ""
        if fault_after_ms == NO_FAULT_MS
        else (
            "#\n"
            "# *** THIS RUN IS ARMED TO CRASH ON PURPOSE. *** The loader raises 0xc0000005 on a\n"
            "# dedicated thread after the delay below, to exercise the crash logger's FATAL path --\n"
            "# the top-level filter and the minidump tier -- which a first-chance exception cannot\n"
            "# reach. The game dying is the expected result, not a failure.\n"
        )
    )
    return f"""\
# DARK SOULS II mod settings. Read by `dinput8.dll` out of this directory -- the directory of the
# running executable -- in `DllMain`, before the game's entry point.
#
# WRITTEN BY scripts/ds2-run.py ON EVERY LAUNCH. Edits to the two startup-only keys below are
# overwritten by the next run, deliberately: the arm under test has to be the arm that was asked
# for, and the launcher reads the arm back out of the log to prove it was.
#
# THIS FILE REPLACED TWO ENVIRONMENT VARIABLES, and the reason is measured rather than stylistic.
# `DS2_ARXAN_PROBE` and `DS2_ARXAN_PROBE_SKIP_NEUTER` were set in the launcher's environment and
# arrived at the game UNSET: `steam -applaunch` hands the request to an already-running Steam
# client over IPC, and the game inherits THAT client's environment. A file beside the DLL travels
# through no IPC at all, and both arms can now be run back to back with nothing to do in between.

[{CONFIG_SECTION}]
# STARTUP-ONLY. Both are consumed in DllMain, before the game's entry point, because that is the
# only moment the choice can be made: `skip_neuter` decides whether Arxan's 48 stubs are patched
# before the Arxan entry stub runs, and there is no un-neutering a live process. Editing either
# one while the game is running changes nothing and says so in the log.
{KEY_ENABLED} = {str(settings[KEY_ENABLED]).lower()}
{KEY_SKIP_NEUTER} = {str(settings[KEY_SKIP_NEUTER]).lower()}
# STARTUP-ONLY. "m1" is the control: a clean function Arxan never touched, where a surviving
# detour says only that hooking works at all. "redirected" is applySpEffect, whose five entry
# bytes ARE Arxan's redirect -- the only site where survival is evidence about Arxan.
{KEY_SITE} = "{site}"

[{INTRO_SECTION}]
# STARTUP-ONLY. Detours the `enter` of the three boot substates -- FeSubStateWarningNoCopy,
# FeSubStateTitleLogo, FeSubStateTitleUserPolicy -- and writes each one's terminal phase, which
# is a transition every one of them already performs on itself under some condition the game
# knows about. There are THREE logo screens, not one.
#
# ON by default; `--no-intro-skip` writes false. The key exists so that a boot failure can be
# tested against this feature by editing one line, with no rebuild and nothing to re-stage. A
# default that cannot be switched off is a default that cannot be ruled out.
{KEY_INTRO_ENABLED} = {str(intro_skip).lower()}

[{DIALOG_SECTION}]
# STARTUP-ONLY. Detours ONE function -- FeSubStateCommonWindowBase::v3, the update every message
# box in the title flow shares -- and writes the result byte a button press writes. The dispatch
# that closes the box reads that byte and nothing else about the press, so this is the press
# rather than an imitation of it; the close, the animation and the phase transition all stay the
# game's own.
#
# It answers three allowlisted boot dialogs and re-checks at runtime that their two decision
# handlers are still the base class's empty stubs, so a box whose answer would DO something --
# FeSubStateTitleDeleteProfile shares the same update and overrides one of them -- is left for the
# player. Anything it declines to answer is named in the log.
#
# ON by default; `--no-dialog-skip` writes false. Separate from [{INTRO_SECTION}] on purpose: two
# switches mean a boot failure can be pinned on one feature without rebuilding either.
{KEY_DIALOG_ENABLED} = {str(dialog_skip).lower()}

[{TITLE_SECTION}]
# STARTUP-ONLY, both keys. The last two things between boot and a usable menu, and they are NOT
# the same kind of thing as the notice boxes above -- neither is suppressed.
#
# `{KEY_PRESS_ANY_BUTTON}` detours the poll behind the PRESS ANY BUTTON gate so it always reports a
# press. That poll has exactly ONE caller in the whole image (inside FeSubStateTitleMain::v3), so
# this reaches one gate rather than input handling. The gate that waits for the title sequence to
# be up is left alone, and the game's own phase-1 body -- which is what builds the top menu -- runs
# in full.
#
# `{KEY_PROCESS_WINDOWS}` zeroes the minimum display time on the "please wait" windows
# (network check, server login, system-data save, profile load). Those wrap REAL asynchronous work
# and are never skipped: the wait on "is it finished yet" is untouched, and only the artificial
# floor that keeps the window up after the work is already done is removed.
#
# ON by default; `--no-press-any-button-skip` and `--no-process-window-skip` write false. Two keys
# rather than one so a boot failure can be pinned on one hook.
{KEY_PRESS_ANY_BUTTON} = {str(press_any_button).lower()}
{KEY_PROCESS_WINDOWS} = {str(process_windows).lower()}
# STARTUP-ONLY. `{KEY_HIDE_PROCESS_WINDOWS}` goes further: it reproduces the wait window's `enter`
# WITHOUT its one call that draws a window, so the box never appears at all. The call that starts
# the work is still made and the wait for that work is still honoured -- only the drawing is
# dropped. It rides on the `{KEY_PROCESS_WINDOWS}` detour, so turning THAT off leaves the wait
# windows completely alone.
{KEY_HIDE_PROCESS_WINDOWS} = {str(hide_process_windows).lower()}
# STARTUP-ONLY. `{KEY_TITLE_ANIMATION}` writes FeSubStateTitleMain's terminal phase once its phase-1
# body has run, skipping phases 2 and 3 -- the flourish that plays after the press is registered.
# Phase 1 is where the top-menu setup happens, so observing phase 2 or 3 means that setup is
# already done and only the animation is left.
{KEY_TITLE_ANIMATION} = {str(title_animation).lower()}
# STARTUP-ONLY, and this is the one that removes the title logo animating in. Phase 1 of
# FeSubStateTitleMain will not even LOOK for a button press until 0x1400f37f0 reports the scene's
# current sequence is 0x67 -- the idle "press any button" state. That wait is the animation, so
# `{KEY_PRESS_ANY_BUTTON}` on its own skips nothing visible. Forcing this gate too lets phase 1 run
# on the first frame; its own body then calls the game's finish-sequence routine, which is how the
# press path already handles being taken mid-animation.
{KEY_TITLE_SEQUENCE_GATE} = {str(title_sequence_gate).lower()}
# STARTUP-ONLY. `{KEY_TITLE_SETTLE}` puts FeSceneTitle straight into its settled state by playing
# sequence 0x67 -- the state the gate above waits to observe -- rather than letting the 0x66 intro
# sequence FeSubStateTitleMain::v1 started play out. THIS is what makes the menu usable as soon as
# its data is available instead of being paced by an animation. It shares the
# FeSubStateTitleMain::v3 detour with `{KEY_TITLE_ANIMATION}`, so either key installs that hook and
# each behaviour is gated separately inside.
{KEY_TITLE_SETTLE} = {str(title_settle).lower()}
# STARTUP-ONLY. `{KEY_SUBSTATE_FLOORS}` lifts the ONE-SECOND FLOORS that
# FeSubStateTitleSteamLoadSystemData and FeSubStateTitleInformation keep for themselves. Measured
# at 879ms and 985ms, both spent AFTER the work they were waiting for had finished. They are not
# process windows, so they have no min_duration field for `{KEY_PROCESS_WINDOWS}` to zero -- each
# inlines a comparison against the pooled 1.0f literal instead, which is why the existing fix
# never reached them. This sets each substate's OWN elapsed field at enter so the game's own
# comparison passes; the shared constant is NOT touched (it has 2042 references). Only the floor
# goes: Information's branch still returns while its download job is running.
{KEY_SUBSTATE_FLOORS} = {str(substate_floors).lower()}

[{MENU_SECTION}]
# STARTUP-ONLY. NOT a skip -- this is the only key here that changes what the title menu DRAWS
# rather than how long it takes to get there, which is why it is its own section.
#
# The top menu is a fixed vector of SIX rows and the game never inserts or removes one; the only
# per-row variable is one byte, computed from whether a save exists and whether online is
# available. That byte does two separate things: it sets the cell state that decides whether the
# cursor can land on the row, and it picks which sequence the row plays. On this layout the
# unavailable sequence does not grey a row, it takes it off the screen -- which is why LOAD GAME
# is simply absent until a save exists.
#
# `{KEY_SHOW_UNAVAILABLE}` swaps which of the two carries the meaning: the row is styled as
# available so it is DRAWN, and its cell state is put straight back to the unavailable value so it
# is STILL NOT SELECTABLE. Both are the game's own values in the game's own fields.
#
# The row is drawn in its normal style, not a greyed one: sequence ids index a layout resource
# inside GameDataEbl.bdt, so which id looks greyed cannot be read out of the executable and is not
# guessed at here. What this delivers is "visible and inert".
#
# ON by default; `--no-show-unavailable-menu-rows` writes false.
{KEY_SHOW_UNAVAILABLE} = {str(show_unavailable).lower()}

[{TIMELINE_SECTION}]
# STARTUP-ONLY, and THE ONLY FEATURE HERE THAT DEFAULTS OFF. It measures; it does not fix.
#
# Detours two pieces of machinery rather than any named screen: FeStateFlow::update, which drives
# whichever substate is resident, and FeSubStateBase::v6, the "drop my transitions" slot the flow
# calls immediately before every leave -- checked against all 36 substate vtables, not one
# overrides it. So every arrival and every departure is seen, including from classes nobody
# thought to name. That is the point: per-class hooking already missed steps once in this repo.
#
# The two hooks are each other's check. A `leave` line whose `mismatch=true` means the arrival
# sampler missed a transition and the durations after it are attributed to the wrong step.
#
# Timestamps are milliseconds from DllMain, which runs during import resolution -- BEFORE the
# game's entry point. So the gap between t=0 and the first substate is the engine starting up,
# and under Proton that may be the largest number in the log.
#
# OFF by default; `--boot-timeline` writes true. Only an exact `true` turns it on: for an
# instrument the harmless direction of a typo is "did not measure", never "patched two extra
# sites in a run that was not meant to be instrumented".
{KEY_TIMELINE_ENABLED} = {str(boot_timeline).lower()}

[{CONTINUE_SECTION}]
# STARTUP-ONLY, and OFF by default. The first half of a native continue flow, and the half that
# only watches.
#
# Detours FeSubStateTitleLoadDataList::v3 -- the character list's per-frame update, and the only
# place the selected slot, the group's confirmed action and the outgoing phase are all in scope at
# once. It logs one line per change: which slot the cursor is on, whether that slot is occupied,
# what the ownership word says, and which substate the phase it wrote will transition to.
#
# It writes NOTHING into the game -- `slot` below is what drives the flow. This is the instrument
# that made that possible: it is what showed that the two fields a shortcut would want to write are
# outputs of the list group rather than inputs to it.
#
# OFF by default; `--continue-record` writes true. Only an exact `true` turns it on.
{KEY_CONTINUE_RECORD} = {str(continue_record).lower()}
# STARTUP-ONLY. The slot the character list opens on, 0-9. Negative leaves the game's own
# selection alone, which is the default.
#
# Written in the list's own `enter`, before the list is built, because the list group reads this
# field when it lays itself out and writes it back on every cursor move -- a later write would be
# erased by the first press of a direction key. The list still opens and still does all of its own
# setup: this is the cheap half of a continue flow, and if it is wrong you are looking at a
# character list rather than a black screen.
#
# It refuses to select a slot the game would refuse: same bound the game applies, and the slot must
# be occupied and not excluded. A rejected slot is named in the log and changes nothing.
#
# With a slot set, the game also skips the top menu: boot goes straight into that character with no
# input at all, measured at 5841ms.
{KEY_CONTINUE_SLOT} = {continue_slot}
# STARTUP-ONLY. ON by default, and inert unless `slot` above is set.
#
# Holds FMOD's master channel group at zero from audio init until FeSubStateTitleStartIngame, so
# the title music and the confirm sounds for the two menus nobody pressed do not play. The volume
# is then restored to the number the game itself last applied -- read out of the sound manager
# rather than reset to 1.0, so whatever you set in the options menu survives.
#
# Measured: armed during engine init, before the title state machine exists, and restored at
# t=5740ms on the frame the game enters StartIngame. Both calls returned FMOD_OK.
#
# The lever is the game's own: [0x14166dfa8] is the MOFmodSoundManager singleton (named by RTTI),
# +0x9f8 is the master channel group FMOD wrote there through getMasterChannelGroup, and setVolume
# is called through the same fmodex64.dll import slot the game uses. Nothing in .text is patched
# for the mute itself.
#
# Set to false to hear the title as the game plays it. Only an exact `false` turns it off.
{KEY_CONTINUE_SILENCE} = {str(continue_silence).lower()}
# STARTUP-ONLY. ON by default, and inert unless `slot` above is set.
#
# Calls the frontend's own FeGroupBase::close (0x1400f18b0) on the six-row top menu and on the
# character list, from the detours the shortcut already owns -- so neither is drawn while the
# autocontinue walks through it. It patches no extra sites.
#
# That close is nineteen instructions: play sequence 0x68 on the group's scene, bump a counter,
# clear the group's open byte. Its mirror plays 0x66 to open, and ds2-dialog-skip already plays
# 0x67 for settled, so 0x66/0x67/0x68 are one family. Both open and close test the open byte
# first, which is what makes calling this from a per-frame detour safe.
#
# It hides two menus. It does NOT cover the logos or the title screen -- that wants the game's own
# NOW LOADING page, and the call that raises it is still unknown.
#
# Set to false to watch the menus flash past. Only an exact `false` turns it off.
{KEY_CONTINUE_HIDE_MENUS} = {str(continue_hide_menus).lower()}

[{OFFLINE_SECTION}]
# STARTUP-ONLY, all four. ON BY DEFAULT, and it is the only feature here whose default is on for a
# reason that is not convenience: everything else in this mod patches `.text` in a running copy of
# DARK SOULS II, which is exactly what FromSoftware's matchmaking servers watch for. A modded
# client that logs in is a client that can be soft-banned.
#
# `{KEY_PIN_FLAG}` replaces `NetService::setOnline` (0x140513820) with `ret`. The service's own
# constructor writes 0 into that flag four instructions in, so this does not IMPOSE offline -- it
# prevents the game leaving the state it is built in.
#
# `{KEY_REPORT_OFFLINE}` replaces `NetService::isOnline` (0x140513600) with `xor eax,eax; ret`.
# 34 call sites, every one followed by `test al,al`; `FeSubStateTitleOnlineCheck`'s own work
# starter is one of them and returns without starting anything on a zero.
#
# `{KEY_BLOCK_SOCKETS}` fronts the game's own WS2_32 imports -- connect, sendto, getaddrinfo,
# gethostbyname -- and refuses anything that is not loopback with WSAENETUNREACH, the error a
# machine with no route gives. IT IS NOT REDUNDANT WITH THE OTHER TWO:
# `FeSubStateTitleGameServerLogin`'s work starter (0x1400f9820) does NOT read the online flag, so
# without this the login goes out on the wire while the menu says you are offline. Steam's own
# sockets are untouched -- this patches one executable's import table, not the process.
#
# `--no-offline` writes false and PLAYS ONLINE WITH A MODDED CLIENT. `--offline-no-socket-block`
# keeps the flag patches and drops the socket guard, which is the arm that measures how much
# traffic the flag layer never reaches.
{KEY_OFFLINE_ENABLED} = {str(offline).lower()}
{KEY_PIN_FLAG} = {str(offline).lower()}
{KEY_REPORT_OFFLINE} = {str(offline).lower()}
{KEY_BLOCK_SOCKETS} = {str(offline and block_sockets).lower()}


[{MENU_ROW_SECTION}]
# NOT STARTUP. The only feature here whose hook is never reached during boot: it detours the item
# builder for one PAUSE menu tab, which the game runs when FeGroupInGameTopSelect is constructed --
# a game in progress and a press of the pause button away.
#
# OFF unless this says exactly `true`. The pause menu is six tabs; each tab's contents are a
# DLFixedVector of (action, gate) entries, capacity five. The tab holding the quit item -- action 9,
# FeGroupInGameReturnTitleCheck, the dialog that offers to save on the way to the title -- holds
# three: Game Options, Screen Settings, Quit.
#
# THIS ADDS A FOURTH THAT QUITS TO DESKTOP WITHOUT ASKING. FeSubStateTitleShutdown::v1 is three
# instructions -- load the title singleton, write 1 to +0x13a, return -- and GameManagerImp's
# per-frame master update polls that byte, so the shutdown is the game's own. It does not save and
# it does not ask; the quit-to-title row offers to save because THAT flow asks.
#
# FOUR HOOKS, and it is worth knowing which does what when a run disappoints:
#   0x000a5900  the tab's item builder     appends the item
#   0x000a6090  the tab's item dispatch    turns action 0x1000 into the shutdown
#   0x000a5b50  the tab's cell namer       names the fourth cell, id 0x1eaccd
#   0x00b54740  findDefinition             supplies that cell
#
# The last two are a pair. A row is drawn only if the grid's layout bind can resolve the cell's
# scene path, so naming a cell the layout does not have is a measured null -- which is exactly what
# an earlier run got, and which makes it this arm's control. The layout half substitutes the quit
# tab's container definition with a copy declaring two more children (a row and its mark) whose
# records are clones of row 0's, moved down one step. Nothing is written to disk.
#
# EVERY HOOK REFUSES RATHER THAN GUESSES. The builder demands (0x7,0) (0x8,0) (0x9,4); the namer
# demands the quit tab's own base path; findDefinition demands seven children carrying seven known
# ids. Read the log before believing a screen -- a refusal and a menu that was never opened look
# identical there. `{MENU_ROW_LOG_PREFIX} container substituted ...` and `... cell named ...` are
# the two lines that say the row should exist, and `row-extent` on the tab line is what says it
# does.
# {KEY_MENU_ROW_ENABLED} = true
#
# COMMENTED OUT BY THIS SCRIPT, ALWAYS, and it is the same reason the `{KEY_MENU_ROW_ROWS}` line below is.
# Set, this key is the LEGACY spelling and the DLL's legacy branch adds ONE row -- quit-to-desktop --
# and none of the other three. Every row is registered only when neither legacy key is set, which is
# the `Default` source in the log. So `--menu-row`, a flag named after the rows, was the thing that
# took three of them away: a run launched with it on 2026-09-23 to exercise a save-file row logged
# `source=LegacyEnabled rows=[quit-to-desktop]` and did not have the row under test on screen. The
# flag is gone and this line is a comment; uncomment it to get the one-row behaviour back.
#
# WHICH ROWS, and why this is a list. The ceiling is the grid's own layout bind, which stops looking
# for cells after fifteen rows -- and the System tab ships three, so {MENU_ROW_MAX_ADDED} rows can be added and
# there are {len(MENU_ROW_ROW_NAMES)} that want one. ALL OF THEM FIT; this key is now about ORDER and about turning
# rows off, not about rationing slots. Naming more than {MENU_ROW_MAX_ADDED} is refused at registration with the
# numbers in the log. A name this table does not know arms NOTHING and is reported: a typo must lose
# a row you asked for, never add one you did not.
#
# It held TWO until the DLL stopped keeping its rows in the game's own item vector (capacity five)
# and cell-namer list (capacity six), both of which are inline arrays whose sixth element would
# land on their own count field. Nothing has measured how far down the banner stays legible, so
# twelve is what the engine allows rather than a number anyone has looked at.
#
# Present, this key OVERRIDES `{KEY_MENU_ROW_ENABLED}` above and `[{BUILD_IMPORT_SECTION}] {KEY_BUILD_IMPORT_ENABLED}` below; absent,
# those two still mean what they always did. The log line says which of the two a run read.
#
#   quit-to-desktop           quits to DESKTOP with no confirmation and no save
#   load-build-from-url       the soulsplanner row described under [{BUILD_IMPORT_SECTION}]
#   load-character-from-file  picks a .sl2/.zip/.7z/.rar for the NEXT launch to load
#   save-game-to-file         asks the game to save, then copies the container where you say
#
# The load row takes effect on the next launch and not this one, and that is the GAME's doing rather
# than a shortfall: DS2 saves on the way out of a game, so a session that re-points the save
# directory and then quits writes the CURRENT character into the staged copy, and the LOAD GAME that
# follows reads back the character you were replacing. So the row writes `{SAVE_FILE_HANDOFF_NAME}`
# beside this file and the loader arms the redirect from it at attach, then DELETES it -- a handoff
# that persisted would put you in somebody else's save on every launch from then on. Delete that file
# by hand to cancel a pick.
#
# The save row refuses to write onto the container the game is playing, which is the DEFAULT path to
# that mistake: its dialog opens in the save's own folder with the save's own name already filled in.
# `{SAVE_FILE_LOG_PREFIX} exported bytes=... destination=...` is the line that says a copy happened,
# and `... THE FLUSH WAS NEVER OBSERVED` is the one that says the copy is your last autosave rather
# than the moment you pressed the row.
{menu_row_rows_line}

[{BUILD_IMPORT_SECTION}]
# NOT STARTUP either, and a different kind of risk from the row above it. This one adds a "Load from
# URL" row that reads a soulsplanner link off the clipboard (or takes a typed build number), fetches
# the build over HTTPS, and APPLIES IT TO THE LIVE CHARACTER: soul memory, then the nine stats and
# therefore the level, then the items, then every weapon, armour piece, ring, spell, hotbar slot and
# the covenant, and finally the Estus Flask, taken to the maximum this game allows.
#
# EVERY WRITE IS A CALL INTO THE GAME'S OWN FUNCTION. Nothing here pokes a field the engine
# maintains -- stats through PlayerParam::SetAllStats, souls through AddSouls, items through
# ItemGive, slots through SetEquip, the covenant through PlayerCtrl's own setter, and the flask
# through the same ItemInventory2::SetEstusProperty the Emerald Herald's dialogue calls. Each one is
# read back afterwards, because several of them fail silently.
#
# OFF unless this says exactly `true`. It changes a character, and while a redirected save is
# re-staged every launch, the game's own save is not.
#
# WHY IT NEEDS A NEWER STEAM INTERFACE. The game's own steam_api64.dll knows SteamUtils005, whose
# ShowGamepadTextInput takes four arguments and cannot prefill; pchExistingText arrives in
# SteamUtils007. ISteamClient012::GetISteamUtils takes a version STRING and passes it through to
# steamclient64.dll, which vends up to 011 -- so this asks for 007 and gets a second interface whose
# slot 0xa0 takes the prefill, while the game keeps its own 005 pointer untouched. Handing the game
# the newer pointer would be a bug: 007 keeps the method at the SAME slot, so the game's four-arg
# call would pass whatever was in r9 as a const char*.
#
# WHY IT WRITES THE GAME'S OWN KEYBOARD STATE. The game's GamepadTextInputDismissed_t listener
# (0x00ff2040, fourteen branchless bytes) does not check whether the GAME asked for the keyboard,
# and it is registered process-wide, so it fires for a session this mod opened. It writes m_state,
# and that field is load-bearing twice over:
#   * left dirty, SoftwareKeyboardManagerImpl::show refuses forever (it demands -1 at 0x140ff2317)
#     and character naming silently falls back to the in-game widget for the rest of the process;
#   * opened over the game's own session, FeSoftKeyImputJob harvests OUR text as the player's name.
# So the mod refuses unless the field reads -1, claims it while open, and restores it on every path
# including the error ones.
#
# READ THE LOG, NOT THE SCREEN. `{BUILD_IMPORT_LOG_PREFIX} field open, prefilled ...` is the line
# that says the overlay drew something; `... no field: the Steam overlay is disabled` is the one
# failure no amount of reading the executable could predict, because it is a property of the running
# Steam client rather than of the game.
# {KEY_BUILD_IMPORT_ENABLED} = true
#
# COMMENTED OUT BY THIS SCRIPT, ALWAYS, for the reason written under `[{MENU_ROW_SECTION}] {KEY_MENU_ROW_ENABLED}`:
# it is the other half of the DLL's legacy branch, and setting either one narrows the menu to the
# rows those two keys name. The row itself is registered by the `{KEY_MENU_ROW_ROWS}` list -- or, as
# here, by the default that list falls back to -- not by this key.

[{INVENTORY_SORT_SECTION}]
# NOT STARTUP. Four detours -- the constructor and destructor of the Inventory tab AND of the equip
# screen's item picker -- installed in the same post-Arxan callback as everything else, plus a
# per-frame tick borrowed from `ds2-menu-row`.
#
# IT ADDS NO FEATURE. DARK SOULS II already sorts the inventory -- `①：Sort`, the dialog headed
# "How should the list be sorted?", and per-category keys including a real total attack rating
# (the sum of all seven attack components). What the game does not ship is a way to MOVE that
# button: `win32onlymessage.fmg` 10332..10341 is the complete list of rebindable menu actions and
# sorting is not among them, riding instead on one of two generic Function keys, and there is no
# controller remapping in this game at all. So this opens the shipped dialog from a button you
# choose, and leaves the shipped `①` prompt working.
#
# THE EQUIP SCREEN IS THE OTHER HALF, and there the game ships no sort prompt at all -- so this is
# not a rebinding there, it is the button that was never there. The sorting itself already is: the
# picker's list is rebuilt by the same shared builder the Inventory tab uses, reading the same
# per-category sort key, so a sort chosen in the Inventory tab already reorders the equip list.
# What was missing is only a way to choose one without leaving the screen.
#
# It calls one function -- the shipped sort-dialog entry, which takes the group and nothing else and
# refuses itself while another dialog is up. One copy serves both menus: they share a base class, so
# the guard field and the dialog's parent mean the same thing in each, and everything else is
# reached through virtual slots each class implements for itself. Nothing is injected into the input
# path; a synthesised button press would also fire every OTHER thing the shipped Function key does
# in whatever menu happened to be open.
{KEY_INVENTORY_SORT_ENABLED} = {str(inventory_sort).lower()}
# NOT startup-only, unlike everything above: these two are re-read about once a second while the
# game runs, so a button can be moved without a restart. A value that does not parse keeps the one
# already working and says so in the log.
#
# `{KEY_INVENTORY_SORT_KEY}` is a key NAME from `ds2-hotkey-config` ("F7", "]", "KP_Plus", "Insert"), empty for none.
# `{KEY_INVENTORY_SORT_PAD}` is an XInput button: a b x y lb rb back start lthumb rthumb dpad_up dpad_down
# dpad_left dpad_right. Empty for none. BOTH may be set; either one opens the dialog.
#
# `lthumb` -- LEFT STICK CLICK, L3 -- is the default because that is where ELDEN RING puts Sort, and
# mirroring it is the whole point. That one value did NOT come out of a binary: ELDEN RING registers
# its Sort prompt with menu-input id 0x2E and ships NO keyboard key-name strings at all (it draws
# inputs as sprites), so there is no table to resolve the id against. The player named the button.
# The keyboard default beside it, `F7`, is still just what the first test runs happened to use.
{KEY_INVENTORY_SORT_KEY} = "{inventory_sort_key}"
{KEY_INVENTORY_SORT_PAD} = "{inventory_sort_pad}"

[{SAVE_REDIRECT_SECTION}]
# Startup-only, and the one key here that can change where your progress is written. The game's own
# save-directory builder is answered with this folder, so DS2 opens its own container name inside it
# and both reads and writes that file for the rest of the session.
#
# Nothing is copied, and that is the whole design. The key this replaces named a `.sl2` file, and a
# file cannot be played in place by a game that builds its own container name -- so it copied the
# file into a staging folder, pointed the game there, and rewrote the copy from the same source on
# the next launch. Every session started that way silently threw away its own progress. A folder
# needs no copy, so there is no duplicate to play and nothing to overwrite.
#
# Empty means the game's own directory, which is the only safe default: a save location guessed on
# the player's behalf is one they did not choose. `--save-dir` writes this; it takes a Linux path
# and converts it, since the DLL runs inside the Proton prefix and sees `/home/you` as
# `Z:\\home\\you`.
#
# A folder that is not there is refused, and the loader says so. It is not created, because DS2
# hides the `LOAD GAME` row when it finds no container -- so a typo'd path would look exactly like
# a save that had gone missing. A folder that exists but is empty is fine and starts a fresh
# character there.
{KEY_SAVE_REDIRECT_DIRECTORY} = "{save_directory}"

[{ITEM_WARN_SECTION}]
# STARTUP-ONLY. A red badge on the icon of any weapon whose stat requirements the character does
# not meet, in the bottom-left of the cell, drawn by `ds2-item-warn`.
#
# OFF unless `--item-warn` asked for it, and the default is not taste. This feature patches the
# frontend's layout builder and its cell bind, and the case that it is safe is a case from static
# reading alone -- no run has put it on screen. `inventory_sort` above defaults ON because three
# runs put its dialog there; this has no such line to point at.
#
# The check it uses is the PRESENTATION one (`FUN_1400bcde0`, the detail pane's), not the mechanics
# one (`FUN_14034d3c0`). The two disagree and share no predicate: the mechanics check honours grip
# -- two-handing HALVES a weapon's Strength requirement (`shr cx,1` at `0x14034d44c`) -- and the
# presentation one takes no grip argument at all. An item in a list is not being held, so it has no
# grip, which is the argument for the pane's answer. `ds2-mods-rs-6tz` revisits it after a run.
#
# Grep the log for `{ITEM_WARN_LOG_PREFIX}`; it names every site it patched and every one it refused.
{KEY_ITEM_WARN_ENABLED} = {str(item_warn).lower()}

[{SEAMLESS_SECTION}]
# A SECOND MOD, written by someone else, loaded into this same process.
#
# NOTHING HERE SHIPS IT. This key is a path. The other mod is installed by you, from its own
# download, under its own licence, next to `DarkSoulsII.exe`; if the file is not at the path below
# the DLL says so in the log and the game runs with this repo's features alone.
#
# THIS KEY DOES NOT LOAD IT. It says that mod is in this run, and the only thing the DLL does
# with that is follow the save file it renames: `save_file_extension` in `ds2sc_settings.ini` is
# `co2` by default, so the container becomes `DS2SOFS0000.co2` and every save feature above -- load
# from file, save to file, load a build -- has to open that name instead of `DS2SOFS0000.sl2`.
#
# The loading is done by `{STAGED_LAUNCHER_NAME}`, this repo's own injector, which
# `scripts/ds2-run.py --seamless` runs in place of the mod's `ds2sc_launcher.exe`. An in-process
# `LoadLibraryW` was tried from four slots and every one mapped the DLL and left it inert -- it
# read no settings, installed no hooks, and the game went on opening `.sl2`. The injector creates
# the process suspended, injects, then resumes, and that ordering is the thing that matters: a DLL
# loaded by an import cannot reproduce it, because its own DllMain is part of the initialisation
# that has to not have happened yet. See `[{LAUNCHER_SECTION}]` below for the list it works from.
#
# BEFORE THE FIRST CO-OP RUN, two things that have no in-game explanation:
#   * `cooppassword` in `SeamlessCoop/ds2sc_settings.ini` must not be empty, or the mod stops the
#     boot on its own dialog. Everyone in a session needs the same string.
#   * the co-op save starts empty. `--seamless` copies `DS2SOFS0000.sl2` to the co-op extension
#     when that file does not exist yet, and never overwrites one that does.
#
# `[offline]` above and this are mutually exclusive: that section fronts the socket imports, so a
# co-op mod under it would run, report success and never connect. `--seamless` therefore turns
# `[offline]` off for the run and says so.
#
# Grep the log for `{SEAMLESS_LOG_PREFIX}`.
{KEY_SEAMLESS_ENABLED} = {str(seamless).lower()}
{KEY_SEAMLESS_DLL} = "{seamless_dll}"

[{LAUNCHER_SECTION}]
# Extra DLLs that get into the game from OUTSIDE it, injected before it runs an instruction.
#
# Read by `{STAGED_LAUNCHER_NAME}` rather than by `{STAGED_DLL_NAME}`, because by the time this
# repo's own DLL is running it is already too late: the process exists and its main thread has run
# `LdrInitializeThunk`, which is exactly the state the four-slot measurement above showed a mod
# cannot be loaded into. The injector is the only thing here that runs before that.
#
# Paths are relative to the game directory, or absolute. They are injected in the order written,
# each fully loaded before the next is asked for, because two mods that hook the same function
# resolve in load order and a list whose order did not survive would make that unfixable from
# here. `[{SEAMLESS_SECTION}] {KEY_SEAMLESS_ENABLED}` puts its own DLL at the front of this list
# and is deduplicated against it, so naming it in both places still loads it once.
#
# Every entry must exist before anything starts. A missing file is refused with the line to edit,
# and no process is created -- a session gets the whole list or does not exist, because a game
# that came up with some of its mods in it has no way to say from the inside which ones.
#
# `{STAGED_DLL_NAME}` is deliberately not in this list. It is a static import of
# `DarkSoulsII.exe`, so the loader maps it unasked, and the injected thread runs process
# initialisation before its own start routine -- meaning this repo's loader is in first and these
# follow.
#
# Grep the log for `{LAUNCHER_LOG_PREFIX}`.
{KEY_LAUNCHER_DLLS} = [{launcher_dll_list}]

[{INVASION_PATH_SECTION}]
# A direction to every other player in your session, drawn over the world.
#
# THE ONLY FEATURE IN THIS FILE THAT DETOURS A RENDERING FUNCTION. Everything else here hooks a
# menu method or reads memory; this one hooks `IDXGISwapChain::Present` -- in `dxgi.dll`, outside
# the game image and therefore outside everything this repo has learned about Arxan -- and appends
# triangles to a frame that was already finished. It is OFF by default for that reason and because
# it draws through the screen during multiplayer, which is a thing to opt into rather than to
# discover.
#
# What it draws is a walkable ROUTE along DARK SOULS II's own navigation mesh, and an arrow when
# the planner reports no way to walk there. Asking for a route was believed impossible here for a
# while -- the request takes navigation-graph ids, and the conversion from a world position was
# recorded as an asynchronous engine job. It is not one: it is a plain synchronous call that
# returns the id in a register. See `crates/ds2-invasion-path/src/navquery.rs`.
{KEY_INVASION_PATH_ENABLED} = {str(invasion_path).lower()}
# Live, like the two sort bindings above: re-read about once a second, so the key moves without a
# restart. A name that does not parse keeps the one already working and says so in the log.
#
# `{KEY_INVASION_PATH_TOGGLE}` is a key NAME from `ds2-hotkey-config`. `semicolon` rather than a function key,
# and that default was bought with a live failure in the sibling workspace: its overlay shipped on
# F7, a 15-DLL run found another mod polling VK_F7 in the same process, and the key warped the
# player instead of drawing anything with nothing warning about it.
#
# `{KEY_INVASION_PATH_START_ENABLED}` begins with the overlay already on, which is what a test run wants: it means
# the roster and camera code runs without anyone having to press anything.
{KEY_INVASION_PATH_TOGGLE} = "{invasion_path_key}"
{KEY_INVASION_PATH_START_ENABLED} = {str(invasion_path_start_enabled).lower()}
# THE PRISM STONE TRAIL. `0` is off. `833` is the Prism Stone's own effect -- the item DARK SOULS
# II calls a Prism Stone and ELDEN RING renamed to Rainbow Stone, whose glowing pebble is the
# whole reason the sibling crate places effects along its route at all. `833..=839` are its seven
# colours, a seven-entry table the game reaches through an emevd instruction named
# `七色石発射`, "fire seven-colour stone".
#
# ON by nothing: spawning an effect is the only thing this DLL does that changes the game rather
# than drawing over it, and the stones are placed by the engine's own spawn from the game's own
# tick. Live, like every setting here -- change the id with the game running.
{KEY_INVASION_PATH_MARKER_EFFECT_ID} = {invasion_path_marker_effect_id}
# THE ONLY WAY A SOLO PLAYER CAN SEE ANY OF THIS WORK.
#
# Everything above starts with ANOTHER PLAYER in your session. Alone, the roster reads
# `remotes=0`, no route is ever asked for, no stone is ever placed, and every line in the log is
# an install line -- "hooked", "armed", "requested" -- with not one execution line among them. A
# real session read `characters=6 players=1 remotes=0`: six objects walked and nothing to point
# at.
#
# The other five objects are the answer. Turn this on and the overlay routes to the nearest
# NON-PLAYER character instead: a live object at a real world position, standing on the navmesh,
# so the whole chain runs -- snap both ends, ask the planner, decode the path, space the stones
# along it, spawn each one. A solo player standing in Majula exercises every line of it.
#
# It narrates. Three failures look identical on the ground -- an effect that is not in this map,
# a spawn the engine's quality throttle discarded, and one that worked and is simply not where
# you are looking -- so it reads the quality byte and the missing-effect tree AT the attempt,
# when two of the three are still visible. It sweeps all seven colours, one per stone, so one run
# says which of them appear. Then it watches them at 1 s, 3 s and 10 s -- which is what says
# whether a Prism Stone LINGERS or merely flashes -- and takes them down again.
#
# OFF unless you are testing. It routes to something you did not ask for and spawns effects to do
# it. Grep the log for `self-check:`.
{KEY_INVASION_PATH_NPC_SELF_CHECK} = {str(invasion_path_npc_self_check).lower()}

[{INPUT_HARNESS_SECTION}]
# LETS AN AGENT MOVE THE CAMERA, AND TAKES YOUR CONTROLLER AWAY WHILE IT DOES.
#
# It detours four device polls and, after each one has run, writes the fields the engine reads.
# Three are the `DLUID` devices -- pad (XInput OR a DirectInput joystick OR a third backend, all
# normalising into the same six floats), DirectInput mouse, keyboard. The fourth is the one that
# actually matters for the camera: `WindowsMouseDevice`, which reads `GetCursorPos` and stores a
# clamped client-space position that `parseCameraInput` DIFFERENCES frame to frame. Writing at
# the device means the deadzone, the sensitivity setting and the key mapping all still apply, so
# an injected input behaves like a real one.
#
# OFF by default because it is the only thing in this file that can stop your own input reaching
# the game. Every command it takes is frame-bounded and the input block has a hard cap of about
# ten minutes, so a harness that wedges lets go on its own.
#
# DRIVE IT while the game runs by writing two lines to `{INPUT_HARNESS_COMMAND_FILE}` beside the
# exe: a sequence number, then a command. It runs when the NUMBER changes. Commands:
#   block <frames> | unblock | release | status
#   axis <index> <value> <frames> | mouse <dx> <dy> <frames> | buttons <hex> <frames>
#   turn <degrees> [frames]   -- closed loop on the camera's own yaw
#   probe [frames]            -- hold each channel and report what the camera did
#   channel <name>            -- what `turn` drives: mouse-x (default), mouse-y, pad0..pad5
#
# `turn` and `probe` MEASURE against the camera `[{INVASION_PATH_SECTION}]` draws through, so they
# need that feature on; without it they refuse rather than guess. `status` also reports how many
# times each poll has fired, which is how you tell "pressed nothing" from "never reached". Grep
# the log for `{INPUT_HARNESS_LOG_PREFIX}`.
{KEY_INPUT_HARNESS_ENABLED} = {str(input_harness).lower()}

[{CRASH_SECTION}]
{crash_banner}# STARTUP-ONLY, both of them. The handler is installed in DllMain BEFORE `neuter_arxan`, because
# that call patches code from static analysis and is the likeliest crash in the whole startup path
# -- a logger installed after it could not report the crash it most exists to report.
#
# `{KEY_CRASH_ENABLED}` defaults to true in the DLL and is written explicitly here anyway: a crash logger
# that has to be switched on is off on the run that needed it, so the file says so out loud.
{KEY_CRASH_ENABLED} = true
# RE-ASSERT THE TOP-LEVEL FILTER, and this is not optional in DARK SOULS II. The unhandled-exception
# filter is ONE global slot, not a chain, and whoever sets it last owns it. This DLL sets it in
# DllMain, before the entry point; the game's CRT then sets its own from an initializer and throws
# ours away. Measured statically from the shipped binary: SetUnhandledExceptionFilter has exactly
# one call site, 0x140c43293, in a function listed in the CRT initializer table at 0x1410ac2c8, and
# it ends `CALL SetUnhandledExceptionFilter; XOR EAX,EAX` -- the previous filter is discarded, not
# chained. Without this re-assert the vectored handler still sees first-chance exceptions and
# NOTHING FATAL is ever recorded, which is exactly what the first in-game crash test measured.
# 0 disables it. 5000ms is a loose bound on "CRT startup is over", not a measurement.
{KEY_REINSTALL_FILTER_AFTER_MS} = {DEFAULT_REINSTALL_FILTER_AFTER_MS}
# `{KEY_FAULT_AFTER_MS} = 0` means never. Anything else DELIBERATELY KILLS THE GAME after that many
# milliseconds. Armed only by `--crash-test`.
{KEY_FAULT_AFTER_MS} = {fault_after_ms}

# LIVE. Re-read by the probe's poller thread through `ds2_hotkey_config::reload::HotFile`, which
# compares the file's TEXT rather than its mtime -- a Proton prefix sits on filesystems that stamp
# mtime to a whole second, so two edits inside one second would be invisible to an mtime watcher.
# An edit here takes effect within one poll interval, without restarting the game. Neither changes
# WHAT is measured: the byte windows and their baselines are fixed when the hook goes in.
#
# Defaults shown. Uncomment to change.
# {KEY_POLL_INTERVAL_MS} = {DEFAULT_POLL_INTERVAL_MS}
# {KEY_HEARTBEAT_INTERVAL_MS} = {DEFAULT_HEARTBEAT_INTERVAL_MS}
"""


def write_config(
    directory: Path,
    probe: str,
    fault_after_ms: int = NO_FAULT_MS,
    site: str = "m1",
    intro_skip: bool = True,
    dialog_skip: bool = True,
    press_any_button: bool = True,
    process_windows: bool = True,
    hide_process_windows: bool = True,
    title_animation: bool = True,
    title_sequence_gate: bool = True,
    title_settle: bool = True,
    substate_floors: bool = True,
    show_unavailable: bool = False,
    boot_timeline: bool = False,
    continue_record: bool = False,
    continue_slot: int = -1,
    continue_silence: bool = True,
    continue_hide_menus: bool = True,
    offline: bool = True,
    block_sockets: bool = True,
    inventory_sort: bool = True,
    inventory_sort_key: str = "F7",
    inventory_sort_pad: str = "lthumb",
    item_warn: bool = False,
    seamless: bool = False,
    seamless_dll: str = SEAMLESS_DEFAULT_DLL,
    invasion_path: bool = False,
    invasion_path_key: str = "semicolon",
    invasion_path_start_enabled: bool = False,
    invasion_path_marker_effect_id: int = 0,
    invasion_path_npc_self_check: bool = False,
    input_harness: bool = False,
    save_directory: str = "",
    menu_rows_all: bool = False,
    menu_rows_no_save: bool = False,
    launcher_dlls: tuple[str, ...] = (),
) -> tuple[Path, str]:
    """Write the config for `probe` into `directory`; return the path and what was written."""
    path = directory / CONFIG_NAME
    text = config_text(
        probe,
        fault_after_ms,
        site,
        intro_skip,
        dialog_skip,
        press_any_button,
        process_windows,
        hide_process_windows,
        title_animation,
        title_sequence_gate,
        title_settle,
        substate_floors,
        show_unavailable,
        boot_timeline,
        continue_record,
        continue_slot,
        continue_silence,
        continue_hide_menus,
        offline,
        block_sockets,
        inventory_sort,
        inventory_sort_key,
        inventory_sort_pad,
        item_warn,
        seamless,
        seamless_dll,
        invasion_path,
        invasion_path_key,
        invasion_path_start_enabled,
        invasion_path_marker_effect_id,
        invasion_path_npc_self_check,
        input_harness,
        save_directory,
        menu_rows_all,
        menu_rows_no_save,
        launcher_dlls,
    )
    path.write_text(text, encoding="utf-8")
    return path, text


def quoted_config(text: str, indent: str = "    ") -> str:
    """The config file's own lines, indented, for a transcript.

    PRINTED IN FULL rather than summarised. The configuration under test is the variable this
    whole change exists to make visible, and a block that says "wrote the config" is exactly the
    claim that turned out to be false last time.
    """
    return "\n".join(f"{indent}{line}" if line else indent.rstrip() for line in text.splitlines())


def report_environment(probe: str) -> None:
    """Print the environment facts that decide whether the run reaches the game intact."""
    print(f"[env] game dir     {GAME_DIR}")
    print(f"[env] built DLL    {BUILT_DLL}")
    variables = launch_env(probe)
    print("[env] launch with  " + " ".join(f"{k}={v}" for k, v in variables.items()))
    print(f"[env] config       {GAME_DIR / CONFIG_NAME}")
    if probe != "off":
        _, expected_arm = PROBE_ARMS[probe]
        print(f"[env] probe arm    {probe} -- the DLL must report back arm={expected_arm}")
    if steam_running():
        # ONLY the override rides this channel now. The probe settings used to be named here too,
        # and were the reason this warning existed at all; they are in the config file precisely
        # so that an already-running Steam client cannot lose them.
        print(
            "[env] steam        ALREADY RUNNING -- `steam -applaunch` hands the request to the\n"
            "                   running client over IPC, and the game then inherits THAT\n"
            "                   client's environment. WINEDLLOVERRIDES is therefore at risk, and\n"
            "                   it is the one setting that cannot move into the config file:\n"
            "                   Wine reads it to decide whether to map our DLL at all. If the run\n"
            "                   comes back with no testimony, that is the first thing to rule\n"
            "                   out -- quit Steam and re-run so this invocation starts the\n"
            "                   client, or set the per-app launch options to\n"
            "                     "
            + " ".join(f'{k}="{v}"' for k, v in variables.items())
            + " %command%\n"
            "                   The probe settings are NOT at risk: they are in the config file,\n"
            "                   which the DLL reads off disk itself."
        )
    else:
        print(
            "[env] steam        not running -- this invocation starts the client, so the\n"
            "                   variables above are inherited by everything it launches."
        )


def stage() -> tuple[Path, str]:
    """Copy the built DLL into the game directory; return the staged path and ITS hash."""
    staged = GAME_DIR / STAGED_DLL_NAME
    shutil.copyfile(BUILT_DLL, staged)
    # Hash the STAGED file, read back off disk, not the source. The point of printing a hash is
    # to describe the bytes that will actually be loaded.
    return staged, sha256(staged)


def stage_launcher() -> tuple[Path, str]:
    """Copy our injector next to the game; return the staged path and ITS hash.

    Staged rather than run out of `target/` for one reason that matters: the injector resolves
    the game directory from its OWN location, so that it keeps working when Steam or a Proton
    chain hands it a working directory it did not choose. Running it from the build tree would
    make it look for `DarkSoulsII.exe` in `target/`.
    """
    staged = GAME_DIR / STAGED_LAUNCHER_NAME
    shutil.copyfile(BUILT_LAUNCHER, staged)
    staged.chmod(0o755)
    return staged, sha256(staged)


def dry_run(
    probe: str,
    observe: float,
    fault_after_ms: int = NO_FAULT_MS,
    site: str = "m1",
    intro_skip: bool = True,
    dialog_skip: bool = True,
    press_any_button: bool = True,
    process_windows: bool = True,
    hide_process_windows: bool = True,
    title_animation: bool = True,
    title_sequence_gate: bool = True,
    title_settle: bool = True,
    substate_floors: bool = True,
    show_unavailable: bool = False,
    boot_timeline: bool = False,
    continue_record: bool = False,
    continue_slot: int = -1,
    continue_silence: bool = True,
    continue_hide_menus: bool = True,
    offline: bool = True,
    block_sockets: bool = True,
    inventory_sort: bool = True,
    inventory_sort_key: str = "F7",
    inventory_sort_pad: str = "lthumb",
    item_warn: bool = False,
    seamless: bool = False,
    seamless_dll: str = SEAMLESS_DEFAULT_DLL,
    invasion_path: bool = False,
    invasion_path_key: str = "semicolon",
    invasion_path_start_enabled: bool = False,
    invasion_path_marker_effect_id: int = 0,
    invasion_path_npc_self_check: bool = False,
    input_harness: bool = False,
    save_directory: str = "",
    menu_rows_all: bool = False,
    menu_rows_no_save: bool = False,
    launcher_dlls: tuple[str, ...] = (),
) -> int:
    print("[dry-run] staging nothing, launching nothing.")
    report_environment(probe)
    problems = preflight(dry_run=True)
    for problem in problems:
        print(f"[dry-run] WOULD REFUSE: {problem}")

    staged = GAME_DIR / STAGED_DLL_NAME
    if BUILT_DLL.is_file():
        print(f"[dry-run] built    sha256 {sha256(BUILT_DLL)}  {BUILT_DLL}")
    if staged.is_file():
        current = sha256(staged)
        print(f"[dry-run] staged   sha256 {current}  {staged}")
        if BUILT_DLL.is_file() and current != sha256(BUILT_DLL):
            print("[dry-run] staged DLL DIFFERS from the built one; a real run would replace it.")
    else:
        print(f"[dry-run] staged   <absent>  {staged}")

    config_path = GAME_DIR / CONFIG_NAME
    if config_path.is_file():
        current = config_path.read_text(encoding="utf-8")
        if current == config_text(
            probe,
            fault_after_ms,
            site,
            intro_skip,
            dialog_skip,
            press_any_button,
            process_windows,
            hide_process_windows,
            title_animation,
            title_sequence_gate,
            title_settle,
            substate_floors,
            show_unavailable,
            boot_timeline,
            continue_record,
            continue_slot,
            continue_silence,
            continue_hide_menus,
            offline,
            block_sockets,
            inventory_sort,
            inventory_sort_key,
            inventory_sort_pad,
            item_warn,
            seamless,
            seamless_dll,
            invasion_path,
            invasion_path_key,
            invasion_path_start_enabled,
            invasion_path_marker_effect_id,
            invasion_path_npc_self_check,
            input_harness,
            save_directory,
            menu_rows_all,
            menu_rows_no_save,
            launcher_dlls,
        ):
            print(f"[dry-run] config   present and ALREADY MATCHES this arm  {config_path}")
        else:
            print("[dry-run] config   present and DIFFERS; a real run would replace it")
            print(f"[dry-run]          {config_path}")
    else:
        print(f"[dry-run] config   <absent>  {config_path}")

    log_path = GAME_DIR / LOG_NAME
    environment = " ".join(f"{k}={v}" for k, v in launch_env(probe).items())
    print(f"[dry-run] would copy   {BUILT_DLL}")
    print(f"[dry-run]         to   {staged}")
    # THE CONFIGURATION UNDER TEST, VERBATIM. It is the whole variable this run turns on, so a
    # dry-run that did not show it would be hiding the one thing it exists to preview.
    print(f"[dry-run] would write  {config_path}")
    print(
        quoted_config(
            config_text(
                probe,
                fault_after_ms,
                site,
                intro_skip,
                dialog_skip,
                press_any_button,
                process_windows,
                hide_process_windows,
                title_animation,
                title_sequence_gate,
                title_settle,
                substate_floors,
                show_unavailable,
                boot_timeline,
                continue_record,
                continue_slot,
                continue_silence,
                continue_hide_menus,
                offline,
                block_sockets,
                inventory_sort=inventory_sort,
                inventory_sort_key=inventory_sort_key,
                inventory_sort_pad=inventory_sort_pad,
                item_warn=item_warn,
                seamless=seamless,
                seamless_dll=seamless_dll,
                invasion_path=invasion_path,
                invasion_path_key=invasion_path_key,
                invasion_path_start_enabled=invasion_path_start_enabled,
                invasion_path_marker_effect_id=invasion_path_marker_effect_id,
                invasion_path_npc_self_check=invasion_path_npc_self_check,
                input_harness=input_harness,
                save_directory=save_directory,
                menu_rows_all=menu_rows_all,
                menu_rows_no_save=menu_rows_no_save,
                launcher_dlls=launcher_dlls,
            ),
            indent="[dry-run]   | ",
        )
    )
    print(f"[dry-run] would launch env {environment} steam -applaunch {APPID}")
    print(f"[dry-run] would poll   {log_path}")
    # The DLL echoes the config back BEFORE it decides anything, so these are the first lines a
    # real run puts in the log. Previewing them in that order is what lets someone reading a real
    # log compare it line for line.
    settings, expected_arm = PROBE_ARMS[probe]
    enabled = str(settings[KEY_ENABLED]).lower()
    skip = str(settings[KEY_SKIP_NEUTER]).lower()
    print(f"[dry-run]     expect  {CONFIG_LINE_PREFIX} file=\"{config_path}\" status=found bytes=..")
    print(f"[dry-run]             {CONFIG_LINE_PREFIX} [{CONFIG_SECTION}] {KEY_ENABLED}=\"{enabled}\" {KEY_SKIP_NEUTER}=\"{skip}\" {KEY_POLL_INTERVAL_MS}=<absent> {KEY_HEARTBEAT_INTERVAL_MS}=<absent>")
    print(f"[dry-run]             {CONFIG_LINE_PREFIX} resolved probe={'on' if probe != 'off' else 'off'} arm={expected_arm or 'neuter-arxan'} poll={DEFAULT_POLL_INTERVAL_MS}ms heartbeat={DEFAULT_HEARTBEAT_INTERVAL_MS}ms ...")
    print(f"[dry-run]         for  a line starting {ARXAN_LINE_PREFIX!r}")
    print(f"[dry-run]         upto {TESTIMONY_BUDGET_SECONDS:.0f}s, then FAIL with exit {EXIT_NO_TESTIMONY}")

    if probe == "off":
        print("[dry-run] probe     off -- no probe lines expected, no verdict block")
    else:
        print(f"[dry-run] probe     {probe}")
        print(f"[dry-run]         then observe {observe:.0f}s for {PROBE_LINE_PREFIX!r} lines:")
        print(f"[dry-run]           {PROBE_LINE_PREFIX} install ... arm={expected_arm} rva=... va=...")
        print(f"[dry-run]           {PROBE_LINE_PREFIX} install original=[..] expected=[..] prologue-match=true")
        print(f"[dry-run]           {PROBE_LINE_PREFIX} install minhook=ok trampoline=0x.. patched=[..] site-jmp=true")
        print(f"[dry-run]           {PROBE_LINE_PREFIX} watching arm={expected_arm} poll={DEFAULT_POLL_INTERVAL_MS}ms heartbeat={DEFAULT_HEARTBEAT_INTERVAL_MS}ms site-window=.. trampoline-window=..")
        print(f"[dry-run]           {PROBE_LINE_PREFIX} heartbeat uptime=..s arm={expected_arm} hits=.. site=intact tramp=intact site-diverged=0 tramp-diverged=0")
        print(f"[dry-run]           {PROBE_LINE_PREFIX} SITE|TRAMP  ... state=DIVERGED ... expected=[..] observed=[..]   (on any change)")
        print(f"[dry-run]           {PROBE_LINE_PREFIX} config ... RELOADED|STARTUP-ONLY-IGNORED  (only if the file is edited mid-run)")
        print(f"[dry-run]           {PROBE_LINE_PREFIX} detach ...                                          (orderly exit only)")
        print(f"[dry-run]         and REFUSE a verdict (exit {EXIT_NO_PROBE_VERDICT}) if the log's arm is not {expected_arm!r}")

    print(f"[dry-run] log present now: {log_path.is_file()}")
    print(f"[dry-run] {GAME_COMM} running now: {bool(pgrep_exact(GAME_COMM))}")
    return EXIT_ERROR if problems else EXIT_OK


def game_dialog_lines() -> list[str]:
    """Any modal window the game process has open, TITLE AND BODY, as text.

    A boot that stops dead with a dialog on it is the single most expensive failure to diagnose
    from here, because every log this repo writes says the run went perfectly -- the DLL loaded,
    every hook installed -- and then nothing further happens. Measured 2026-09-24: a run sat at 19
    threads and zero CPU for minutes and the reason was a message box reading *"Dark Souls II
    seamless co-op (%s) is depreciated and requires an update. The application will now exit."*
    Nothing on disk said so. The mod's DLL is packed, so that sentence is not in the file; it only
    exists decrypted in the process.

    So the body is read the only place it exists: the process's own memory. The window title comes
    from the compositor, and the title is then used as the needle to find the message around it.
    That is general -- it reports whatever dialog is up, not a list of sentences this script knows
    in advance, which is what makes it still work on the next failure nobody has seen yet.

    Returns [] when there is no dialog, no compositor to ask, or no permission to read the memory.
    Never raises: this runs on the reporting path, where a crash would replace a diagnosis with a
    traceback.
    """
    pid = pgrep_exact(GAME_COMM)
    if not pid:
        return []
    pid = pid[0]
    if shutil.which("hyprctl") is None or shutil.which("jq") is None:
        return []
    try:
        # ONLY this pid's windows. The filter is in the query, not in what is printed afterwards:
        # the rest of the desktop is nobody's business and is never read into this process.
        listed = subprocess.run(
            [
                "hyprctl", "-j", "clients",
            ],
            capture_output=True, text=True, timeout=10,
        )
        clients = json.loads(listed.stdout or "[]")
    except (OSError, subprocess.SubprocessError, json.JSONDecodeError):
        return []
    titles = [
        str(c.get("title", ""))
        for c in clients
        if c.get("pid") == pid and str(c.get("title", "")).strip()
    ]
    # The game's own window is not a dialog. Everything else on this pid is one.
    titles = [t for t in titles if "DARK SOULS" not in t.upper()]
    if not titles:
        return []

    lines = []
    for title in titles:
        lines.append(f"dialog title={title!r}")
        body = dialog_body(pid, title)
        if body:
            lines.append(f"dialog body={body!r}")
    return lines


def dialog_body(pid: int, title: str) -> str:
    """The readable text around `title` in the process's memory, or "".

    A message box keeps its caption and its message as neighbouring strings, so finding the
    caption finds the message. Both encodings are tried because Win32 has both, and the window
    manager reports whichever one Wine handed it.
    """
    needles = [("ascii", title.encode()), ("utf-16", title.encode("utf-16-le"))]
    try:
        with open(f"/proc/{pid}/maps", encoding="utf-8") as handle:
            regions = handle.read().splitlines()
        with open(f"/proc/{pid}/mem", "rb", 0) as memory:
            for region in regions:
                fields = region.split()
                if "r" not in fields[1]:
                    continue
                low, high = (int(x, 16) for x in fields[0].split("-"))
                # Whole-image mappings are not where a formatted message lives, and reading a
                # gigabyte to look would cost more than the answer is worth.
                if high - low > 64 * 1024 * 1024:
                    continue
                try:
                    memory.seek(low)
                    buffer = memory.read(high - low)
                except OSError:
                    continue
                for encoding, needle in needles:
                    at = buffer.find(needle)
                    if at < 0:
                        continue
                    window = buffer[at : at + 1400]
                    text = (
                        window.decode("utf-16-le", "replace")
                        if encoding == "utf-16"
                        else window.decode("latin1", "replace")
                    )
                    # Runs of non-printable bytes separate adjacent strings. Measured: a message
                    # box's caption and its message are separated by a SINGLE NUL, so they land
                    # in one part together -- which is the whole answer and is why only the first
                    # part is returned. Taking more appends whatever string happens to be next in
                    # the heap, which reads like part of the message and is not.
                    parts = re.split(r"[^\x20-\x7e\n]{2,}", text)
                    parts = [p.strip() for p in parts if len(p.strip()) > 12]
                    for part in parts:
                        if len(part) > len(title) + 8:
                            return part
                    if parts:
                        return parts[0]
    except OSError:
        return ""
    return ""


def seamless_launcher() -> Path:
    """Seamless Co-op's own launcher, which is the only thing that successfully loads its DLL.

    `LoadLibraryW` from inside the game process was tried from four slots and every one of them
    produced a DLL that mapped and then did nothing -- see `crates/ds2-loader/src/seamless.rs` for
    the table and the control run. Its launcher creates the process suspended, injects, and
    resumes, and the suspended part is load-bearing: a statically imported DLL cannot reproduce it,
    because its own `DllMain` is part of the initialisation that has to not have happened yet.
    """
    return GAME_DIR / "ds2sc_launcher.exe"


def proton_chain() -> tuple[list[str], list[str]]:
    """The argv prefix that runs a Windows executable inside this game's Proton prefix.

    Resolved from the prefix rather than hard-coded, because the Proton a player is on is their
    choice and a wrong one here would run the game under a different Wine than the prefix was
    built with. `compatdata/<appid>/config_info` names the Proton directory on its second line;
    that tool's `toolmanifest.vdf` names the runtime it requires by appid, and that appid's
    `appmanifest` names the runtime's directory.

    Returns `(argv_prefix, problems)`. A non-empty `problems` means do not launch.
    """
    problems: list[str] = []
    config_info = PREFIX_DIR.parent / "config_info"
    try:
        lines = config_info.read_text(encoding="utf-8").splitlines()
    except OSError:
        return [], [f"cannot read {config_info}; launch the game through Steam once first"]
    # Line 2 is a path INSIDE the tool: `<tool>/files/share/fonts/`. Three parents up is the tool.
    if len(lines) < 2:
        return [], [f"{config_info} does not name a Proton directory"]
    tool = Path(lines[1]).parent.parent.parent
    proton = tool / "proton"
    if not proton.is_file():
        problems.append(f"no `proton` at {proton}")

    runtime_entry = ""
    manifest = tool / "toolmanifest.vdf"
    try:
        for line in manifest.read_text(encoding="utf-8").splitlines():
            if "require_tool_appid" in line:
                runtime_entry = line.split('"')[3]
                break
    except OSError:
        problems.append(f"cannot read {manifest}")
    if not runtime_entry:
        # Proton without a container runtime runs directly. Rare now, and not an error.
        return ([str(proton), "run"], problems)

    # `<steamapps>/compatdata/<appid>/pfx` -- three up is `steamapps`, where the manifests live.
    steamapps = PREFIX_DIR.parents[2]
    installdir = ""
    try:
        text = (steamapps / f"appmanifest_{runtime_entry}.acf").read_text(encoding="utf-8")
        for line in text.splitlines():
            if '"installdir"' in line:
                installdir = line.split('"')[3]
                break
    except OSError:
        problems.append(f"the runtime Proton requires (appid {runtime_entry}) is not installed")
    if not installdir:
        return [], problems or [f"appid {runtime_entry} names no install directory"]

    entry = steamapps / "common" / installdir / "_v2-entry-point"
    if not entry.is_file():
        problems.append(f"no runtime entry point at {entry}")
    return ([str(entry), "--verb=run", "--", str(proton), "run"], problems)


def first_loadable_slot(save_dir: str, seamless: bool, seamless_dll: str) -> int | None:
    """The lowest slot of the redirected save that the game will load, or None.

    This is what `--continue-slot` with no value means, and until now the flag's own help
    promised it while the code quietly resolved to -1 and autoloaded nothing.

    BOTH `occupied` AND `blank` COUNT. A slot whose nine stats are all 1 has no name and stats
    below any DS2 starting value, and it is still a character the game loads -- `ds2-sl2.py`
    records the run that settled it (`autoload slot=9 refused=false ... occupied=true`). Treating
    those as empty is the mistake that makes a folder full of fresh characters look like an empty
    file.

    The extension is not assumed. Under Seamless the container is renamed, so the name is built
    from that mod's own settings rather than from `SAVE_FILE_NAME`; a run that looked for `.sl2`
    beside a `.co2` would report an empty folder that is not empty.
    """
    if not save_dir:
        return None
    extension = (seamless and seamless_save_extension(seamless_dll)) or VANILLA_SAVE_EXTENSION
    container = Path(save_dir).expanduser() / f"{SAVE_FILE_STEM}.{extension}"
    if not container.is_file():
        return None
    try:
        listed = subprocess.run(
            [sys.executable, str(SL2_TOOL), "--slots", str(container)],
            capture_output=True,
            text=True,
            timeout=20,
            check=False,
        )
    except (OSError, subprocess.SubprocessError):
        return None
    if listed.returncode != 0:
        return None
    # A NAMED CHARACTER BEATS A FRESH ONE, and the lowest index does not decide it. Both classes
    # load, so "first loadable" alone picks slot 0 out of a container whose slot 0 is an unnamed
    # all-ones blank and whose slot 1 is the character the player actually wants -- which is the
    # exact shape of a save file that has been through `--seamless`. Occupied wins; blank is the
    # fallback for a container that holds nothing else.
    occupied: int | None = None
    blank: int | None = None
    for line in listed.stdout.splitlines():
        parts = line.split()
        # `slot <n> <occupied|blank|empty> ...`
        if len(parts) < 3 or parts[0] != "slot":
            continue
        try:
            index = int(parts[1])
        except ValueError:
            continue
        if parts[2] == "occupied" and occupied is None:
            occupied = index
        elif parts[2] == "blank" and blank is None:
            blank = index
    return occupied if occupied is not None else blank


def seamless_save_extension(seamless_dll: str) -> str | None:
    """The extension Seamless Co-op renames the save container to, or None.

    Read from the other mod's own settings file, which sits next to the DLL wherever the player
    installed it. Its comments are `;` and its values are bare, so this is a two-line parse rather
    than `configparser` -- whose `[SECTION]` handling would also have to guess at the case the
    player typed.

    A value that is not an extension is refused rather than pasted into a path. The same rule, and
    the same test, live in `crates/ds2-seamless`.
    """
    settings = GAME_DIR / Path(seamless_dll.replace("\\", "/")).parent / SEAMLESS_SETTINGS_NAME
    try:
        text = settings.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return None
    for line in text.splitlines():
        line = line.strip()
        if line.startswith((";", "#", "[")) or "=" not in line:
            continue
        key, _, value = line.partition("=")
        if key.strip().lower() != KEY_SEAMLESS_SAVE_EXTENSION:
            continue
        value = value.strip().strip('"')
        if not value or len(value) > 120 or not value.isalnum() or not value.isascii():
            return None
        return value
    return None


def duplicate_save_for_seamless(seamless_dll: str) -> None:
    """Give the co-op session a character to load, by copying the vanilla container to its name.

    Seamless keeps its own save file -- `save_file_extension = co2` -- so a first co-op launch on
    an account that has only ever played vanilla finds no character at all. The copy is what makes
    the existing one show up in the list.

    **It never overwrites.** A `.co2` that already exists is a co-op character with progress in it,
    and replacing it with the vanilla save would silently throw that away. Re-syncing is the
    player's call: delete the `.co2` and launch again.
    """
    extension = seamless_save_extension(seamless_dll)
    if extension is None:
        print(
            f"[seamless] no usable {KEY_SEAMLESS_SAVE_EXTENSION} in "
            f"{GAME_DIR / Path(seamless_dll.replace(chr(92), '/')).parent / SEAMLESS_SETTINGS_NAME}"
            " -- no save was copied"
        )
        return
    if extension == VANILLA_SAVE_EXTENSION:
        print(
            f"[seamless] {KEY_SEAMLESS_SAVE_EXTENSION} = {extension}, which is the name the game "
            "already uses -- nothing to copy"
        )
        return
    if not SAVE_ROOT.is_dir():
        print(f"[seamless] no save folder at {SAVE_ROOT} -- nothing to copy")
        return

    source_name = f"{SAVE_FILE_STEM}.{VANILLA_SAVE_EXTENSION}"
    target_name = f"{SAVE_FILE_STEM}.{extension}"
    copied = 0
    for account in sorted(path for path in SAVE_ROOT.iterdir() if path.is_dir()):
        source = account / source_name
        target = account / target_name
        if target.exists():
            print(f"[seamless] kept {target} ({target.stat().st_size} bytes) -- not overwritten")
            continue
        if not source.is_file():
            continue
        shutil.copy2(source, target)
        copied += 1
        print(f"[seamless] copied {source} -> {target} ({target.stat().st_size} bytes)")
    if copied == 0:
        print(
            f"[seamless] no {source_name} was copied to .{extension}; a co-op session shows the "
            f"characters in {target_name} and nothing else"
        )


#: The Hyprland monitor DARK SOULS II must open on, by DRM connector name.
#:
#: Without a rule the window lands wherever Hyprland's focus happens to be when Steam gets round to
#: mapping it, which is whichever monitor was last clicked -- so the game turns up on a different
#: screen between runs and the one being watched is not always the one it appears on.
#:
#: `DP-1`, not `DP-0`: Hyprland takes its names from DRM connectors, which start at one
#: (`/sys/class/drm` lists `card1-DP-1`, `card1-DP-2`, `card1-DP-3` here). NVIDIA's own tooling
#: numbers the same physical ports from zero, which is where `DP-0` comes from, but this machine is
#: a Radeon RX 6900 XT and has no connector by that name. Requested by the user 2026-09-22.
GAME_MONITOR = "DP-1"


#: Hyprland's window selector for the game, by the class Proton actually gives it. Confirmed live
#: rather than assumed: `hyprctl clients` reports `steam_app_335300` for the running game.
GAME_WINDOW_MATCH = f"class:^steam_app_{APPID}$"


def hypr(lua: str) -> str | None:
    """Run one Lua expression in Hyprland and hand back what it printed, or `None`.

    THIS BUILD HAS A LUA CONFIG PARSER, AND THAT CHANGES EVERY INVOCATION. On Hyprland 0.56 the
    old spellings are all refused, each with a different error, and none of them is the one an
    agent reaches for first:

        hyprctl keyword windowrulev2 "monitor DP-1, class:..."  -> "keyword can't work with
                                                                   non-legacy parsers. Use eval."
        hyprctl keyword windowrule   "monitor DP-1, class:..."  -> same refusal
        hyprctl dispatch focuswindow class:...                  -> parsed as Lua; syntax error
        hyprctl --batch "dispatch a ; dispatch b"               -> parsed as Lua; syntax error

    What works is `hl.dsp.<thing>{...}`, which BUILDS a dispatcher and does not run it -- calling
    `hl.dsp.focus{ monitor = "DP-1" }` on its own returns an `HL.Dispatcher` and the focus does not
    move, which is a silent no-op and exactly the sort of thing that gets reported as done. It has
    to be handed to `hl.dispatch`. `repl` is used rather than `eval` because `eval` answers a bare
    `ok` and swallows the value, so it cannot confirm anything.
    """
    if shutil.which("hyprctl") is None:
        return None
    try:
        result = subprocess.run(
            ["hyprctl", "repl", lua], capture_output=True, text=True, timeout=5
        )
    except (OSError, subprocess.SubprocessError):
        return None
    if result.returncode != 0:
        return None
    answer = result.stdout.strip()
    return None if answer.startswith("error:") else answer


def pin_to_monitor() -> None:
    """Point Hyprland at [`GAME_MONITOR`] so the game's window maps there.

    A new window opens on the focused monitor, so focusing the wanted one immediately before
    `steam -applaunch` is what puts the game where it is meant to go. This is best-effort by
    design: [`settle_on_monitor`] moves the window outright once it exists, which is what covers
    the case where focus wanders while Steam is still starting up.
    """
    if hypr(f'return hl.dispatch(hl.dsp.focus{{ monitor = "{GAME_MONITOR}" }})') is None:
        return
    print(f"[monitor] focused {GAME_MONITOR} so the game maps there")


def settle_on_monitor() -> None:
    """Move the game's window to [`GAME_MONITOR`] and SAY WHERE IT ACTUALLY ENDED UP.

    Called after the DLL's own log line has confirmed the run, by which point the window exists.
    The move is issued by class rather than by focus, so it does not matter what the user clicked
    on while the game was loading.

    The check afterwards is the point. A dispatcher that was built and never run fails silently,
    and so does a move to a monitor that has been unplugged; reading the window's monitor back is
    the difference between reporting a pin and having made one.
    """
    moved = hypr(
        "return hl.dispatch(hl.dsp.window.move{ "
        f'monitor = "{GAME_MONITOR}", window = "{GAME_WINDOW_MATCH}" }})'
    )
    if moved is None:
        return
    where = hypr(
        "local w = hl.get_windows() "
        f'for _, x in ipairs(w) do if x.class == "steam_app_{APPID}" then '
        'return x.monitor and x.monitor.name or "?" end end return "no window"'
    )
    if where == GAME_MONITOR:
        print(f"[monitor] game is on {GAME_MONITOR}")
    else:
        print(f"[monitor] WANTED {GAME_MONITOR}, GAME IS ON {where}")


def launch(
    probe: str,
    observe: float,
    fault_after_ms: int = NO_FAULT_MS,
    site: str = "m1",
    intro_skip: bool = True,
    dialog_skip: bool = True,
    press_any_button: bool = True,
    process_windows: bool = True,
    hide_process_windows: bool = True,
    title_animation: bool = True,
    title_sequence_gate: bool = True,
    title_settle: bool = True,
    substate_floors: bool = True,
    show_unavailable: bool = False,
    boot_timeline: bool = False,
    continue_record: bool = False,
    continue_slot: int = -1,
    continue_silence: bool = True,
    continue_hide_menus: bool = True,
    offline: bool = True,
    block_sockets: bool = True,
    inventory_sort: bool = True,
    inventory_sort_key: str = "F7",
    inventory_sort_pad: str = "lthumb",
    item_warn: bool = False,
    seamless: bool = False,
    seamless_dll: str = SEAMLESS_DEFAULT_DLL,
    invasion_path: bool = False,
    invasion_path_key: str = "semicolon",
    invasion_path_start_enabled: bool = False,
    invasion_path_marker_effect_id: int = 0,
    invasion_path_npc_self_check: bool = False,
    input_harness: bool = False,
    save_directory: str = "",
    menu_rows_all: bool = False,
    menu_rows_no_save: bool = False,
    launcher_dlls: tuple[str, ...] = (),
) -> int:
    report_environment(probe)
    problems = preflight(dry_run=False)
    if problems:
        for problem in problems:
            print(f"REFUSING TO LAUNCH: {problem}", file=sys.stderr)
        return EXIT_ERROR

    staged, digest = stage()
    print(f"[stage] {staged}")
    print(f"[stage] sha256 {digest}")

    # BEFORE LAUNCHING, and after staging: the DLL reads this in `DllMain`, so it has to be on
    # disk before the game starts, and it is rewritten every run so a file left over from the
    # other arm cannot decide this one.
    config_path, config = write_config(
        GAME_DIR,
        probe,
        fault_after_ms,
        site,
        intro_skip,
        dialog_skip,
        press_any_button,
        process_windows,
        hide_process_windows,
        title_animation,
        title_sequence_gate,
        title_settle,
        substate_floors,
        show_unavailable,
        boot_timeline,
        continue_record,
        continue_slot,
        continue_silence,
        continue_hide_menus,
        offline,
        block_sockets,
        inventory_sort,
        inventory_sort_key,
        inventory_sort_pad,
        item_warn,
        seamless,
        seamless_dll,
        invasion_path,
        invasion_path_key,
        invasion_path_start_enabled,
        invasion_path_marker_effect_id,
        invasion_path_npc_self_check,
        input_harness,
        save_directory,
        menu_rows_all,
        menu_rows_no_save,
        launcher_dlls,
    )
    print(f"[config] {config_path}")

    # Before the launch, not after: the game reads its save folder during boot, and a character
    # copied in afterwards would not be in the list this run shows.
    if seamless:
        duplicate_save_for_seamless(seamless_dll)

    log_path = GAME_DIR / LOG_NAME
    # Take the tail's mark BEFORE launching. Everything it hands back afterwards is this run's.
    # Deliberately NOT deleting the log: the DLL rotates it to `.prev` itself on its first write,
    # and deleting here would destroy the previous run's evidence for no gain.
    tail = LogTail(log_path)

    # TEAR DOWN FIRST, ALWAYS. `steam -applaunch` is a request to a client that already believes
    # it knows whether the app is running, and while any process of the previous session survives
    # -- Steam's own `reaper` is the one that does -- the client answers by doing NOTHING: no
    # error, no window, and the testimony wait below times out four minutes later against a game
    # that was never started. Measured 2026-09-22: `pkill -x DarkSoulsII.exe` reported the game
    # gone and left SIXTEEN processes of its session alive, and the relaunch wrote an empty log.
    # This is also what "launch" means as an instruction: remove what is running, then start.
    torn = subprocess.run(
        [sys.executable, str(REPO_ROOT / "scripts" / "ds2-teardown.py")],
        capture_output=True, text=True, timeout=60,
    )
    for line in torn.stdout.splitlines():
        print(line)
    if torn.returncode != 0:
        print("[launch] REFUSING: the previous session did not die; see the survivors above.")
        return EXIT_ERROR

    pin_to_monitor()

    environment = launch_env(probe)
    argv = ["steam", "-applaunch", APPID]
    workdir = None
    inject_from_outside = seamless or bool(launcher_dlls)
    if inject_from_outside:
        # A mod that cannot be loaded from inside the process has to be injected before the game
        # runs, so with anything on this list the run goes through Proton directly rather than
        # through `steam -applaunch`. What that costs is Steam's own bookkeeping -- playtime, the
        # overlay, cloud sync on exit -- because Steam is not the one starting the game. What it
        # buys is a mod that actually initialises.
        chain, problems = proton_chain()
        for problem in problems:
            print(f"[launcher] {problem}")
        if not BUILT_LAUNCHER.is_file():
            problems.append(f"no built launcher at {BUILT_LAUNCHER}")
            print(
                f"[launcher] no built launcher at {BUILT_LAUNCHER}\n"
                "    build it: cargo xwin build --release --target x86_64-pc-windows-msvc "
                "-p ds2-launcher"
            )
        if problems:
            print(
                "[launcher] REFUSING TO LAUNCH -- fix the above, or drop --seamless and "
                "--launcher-dll to run through Steam with this repo's own DLL alone"
            )
            return 1
        # Staged beside the game rather than run out of `target/`: it resolves the game directory
        # from its own location, so that it keeps working when a Proton chain hands it a working
        # directory it did not choose.
        launcher, launcher_hash = stage_launcher()
        print(f"[launcher] staged {launcher}")
        print(f"[launcher] sha256 {launcher_hash}")
        argv = [*chain, str(launcher)]
        workdir = str(GAME_DIR)
        # `SteamAppId` is set for the case Steam did not start the game -- which is this case.
        environment = {
            **environment,
            "STEAM_COMPAT_DATA_PATH": str(PREFIX_DIR.parent),
            "STEAM_COMPAT_CLIENT_INSTALL_PATH": str(PREFIX_DIR.parents[3]),
            "SteamAppId": APPID,
            "SteamGameId": APPID,
        }
    started = datetime.now(timezone.utc).isoformat(timespec="seconds")
    subprocess.Popen(
        argv,
        cwd=workdir,
        env={**os.environ, **environment},
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        stdin=subprocess.DEVNULL,
        start_new_session=True,  # survives this shell, and this agent turn
    )
    print(
        "[launch] "
        + " ".join(argv)
        + "  ("
        + " ".join(f"{k}={v}" for k, v in environment.items())
        + ")"
    )
    if inject_from_outside:
        print(
            f"[launch] started by this repo's own {STAGED_LAUNCHER_NAME}, not by Steam -- no "
            "playtime, no overlay, and no cloud sync for this session"
        )
        print(
            f"[launch] grep the launcher's own output for `{LAUNCHER_LOG_PREFIX}`: it names every "
            "DLL it injected and refuses rather than starting a half-modded game"
        )
    print(f"[launch] waiting up to {TESTIMONY_BUDGET_SECONDS:.0f}s for {log_path}")

    verdict = await_testimony(tail)
    # Once the DLL has testified the window exists, so this is the first moment the move can
    # actually land. Before it there is nothing to move.
    settle_on_monitor()
    if verdict["status"] != "confirmed":
        if verdict["status"] == "attached-silent":
            reason = "the DLL LOADED but dearxan never reported"
            detail = [
                f"saw       {verdict['attach_line']}",
                f"missing   a line starting {ARXAN_LINE_PREFIX!r}",
                "",
                "The proxy and the WINEDLLOVERRIDES both worked -- this is not a loading",
                "failure. neuter_arxan's callback did not reach the log, so look at dearxan,",
                "not at the override.",
            ]
        else:
            reason = "no DLL testimony at all"
            detail = [
                f"waited    {TESTIMONY_BUDGET_SECONDS:.0f}s for any line in {log_path}",
                f"{GAME_COMM} was "
                + (
                    "seen and has since exited"
                    if verdict.get("game_exited")
                    else ("seen and is still running" if verdict.get("game_seen") else "never seen")
                ),
                "",
                "Most likely the DLL was never mapped: check WINEDLLOVERRIDES actually reached",
                "the game (see the [env] steam note above), and that the staged DLL is the one",
                "whose sha256 is printed above.",
            ]
        print(failed_block(reason, detail))
        return EXIT_NO_TESTIMONY

    print(
        running_block(
            {
                "started": started,
                "staged": staged,
                "sha256": digest,
                "log": log_path,
                "testimony": verdict["line"],
                "attach_line": verdict.get("attach_line"),
                "waited": verdict["waited"],
                "game_pids": pgrep_exact(GAME_COMM),
                "config_path": config_path,
                "config": config,
            }
        )
    )
    if fault_after_ms > NO_FAULT_MS:
        return await_crash_evidence(fault_after_ms, started)

    # A dialog is the one failure every log in this repo reports as a success: the DLL loaded,
    # every hook installed, and the boot is standing still behind a message box nobody can see
    # from here. Asked once, after the loader has spoken, because that is when a mod's own
    # refusal has had time to appear. See `game_dialog_lines`.
    for line in game_dialog_lines():
        print(f"[dialog] {line}")

    if probe == "off":
        return EXIT_OK

    # The loader has spoken; now wait for the experiment. The probe's install lines are written
    # from the same Arxan callback that produced the line above, so they are usually ALREADY READ
    # by the time we get here -- sitting in the chunk `await_testimony` returned out of the middle
    # of. Sharing the tail object is not enough to recover them; they are handed over explicitly.
    # See the docstring of `watch_probe` for the run this cost.
    print(f"[probe] observing {observe:.0f}s for {PROBE_LINE_PREFIX!r} lines")
    probe_state = watch_probe(tail, observe, verdict.get("leftover", ()))
    block, code = probe_block(probe, probe_state, site if probe != "off" else None)
    print(block)
    return code


def crash_artifact_report(started_iso: str, now: float) -> tuple[list[str], bool]:
    """Which crash artifacts exist and are FRESH, and whether that is enough to claim success.

    Freshness is the whole point. `ds2-crash-logging-core` rotates each file to `<name>.prev` and
    writes a new one per run, so a stale `ds2-crash-log.txt` from an earlier session sitting in the
    game directory would otherwise read as proof that this run's logger worked. Everything is
    compared against the moment this run launched.

    The minidump is treated as OPTIONAL, and that is a finding rather than a concession:
    `ds2-mods-rs-4tm` asks which minidump tier survives Proton's dbghelp, and in `../er-mods-rs`
    both the rich and normal tiers failed with ERROR_NOACCESS (998). A run whose text artifacts all
    landed and whose dump did not is a SUCCESSFUL crash-logger test that has just answered that
    question in the negative -- so it must not be reported as a failure.
    """
    lines: list[str] = []
    required_ok = True
    for name in CRASH_ARTIFACTS:
        path = GAME_DIR / name
        optional = name.endswith(".dmp")
        if not path.is_file():
            lines.append(f"  MISSING   {name}")
            if not optional:
                required_ok = False
            continue
        age = now - path.stat().st_mtime
        size = path.stat().st_size
        # Written after this run started, not before it. A file older than the launch is the
        # PREVIOUS run's evidence and says nothing about this one.
        fresh = path.stat().st_mtime >= _iso_to_epoch(started_iso)
        mark = "ok" if fresh else "STALE"
        if not fresh and not optional:
            required_ok = False
        lines.append(f"  {mark:<9} {name}  {size} bytes, written {age:.0f}s ago")
    return lines, required_ok


def _iso_to_epoch(iso: str) -> float:
    """The launch timestamp as a POSIX float, for comparing against file mtimes."""
    return datetime.fromisoformat(iso).timestamp()


def await_crash_evidence(fault_after_ms: int, started_iso: str) -> int:
    """Wait for the deliberate fault, then prove the crash LOGGER -- not the crash -- worked.

    THE GAME DYING IS NOT THE RESULT. A game that crashed on its own looks identical from outside,
    and so does a game someone closed. The result is the five files the logger writes, freshly
    written, with the fatal record in them. That is why this reads the artifacts rather than the
    exit of the process.
    """
    # The fault fires `fault_after_ms` after the ENTRY POINT, which is already some way after the
    # testimony line this function is called on the back of. The slack covers the minidump write,
    # which is the slowest thing in the fatal path and the one most likely to be slow under Proton.
    budget = fault_after_ms / 1000.0 + 60.0
    print(f"[crash] armed for {fault_after_ms}ms; waiting up to {budget:.0f}s for the game to die")
    deadline = time.monotonic() + budget
    died = False
    while time.monotonic() < deadline:
        if not pgrep_exact(GAME_COMM):
            died = True
            break
        time.sleep(POLL_SECONDS)

    # Give the fatal path a moment to finish writing after the process leaves the table.
    time.sleep(2.0)
    lines, required_ok = crash_artifact_report(started_iso, time.time())

    latest = GAME_DIR / "ds2-crash-latest.txt"
    record = ""
    if latest.is_file():
        record = latest.read_text(encoding="utf-8", errors="replace").strip()

    body = [
        f"game exited     {died}",
        f"fault armed at  {fault_after_ms}ms after the entry point",
        "",
        "artifacts in the game directory:",
        *lines,
    ]
    if record:
        body += ["", "ds2-crash-latest.txt, verbatim:", *[f"  | {ln}" for ln in record.splitlines()]]

    # DID THE FATAL PATH RUN? This is the distinction the first version of this function missed,
    # and missing it printed a triumphant header over a run that had only proved half of what
    # ds2-mods-rs-4tm asks. The vectored handler sees FIRST-CHANCE exceptions and records
    # `fatal=false`; the top-level filter is what runs when nothing handled the exception, and it
    # is the only thing that writes a fatal record or a minidump. A run with first-chance records
    # and no fatal one has exercised the VEH and NOT the filter -- which is a real answer, but it
    # is not the same answer.
    fatal_seen = "fatal=true" in record
    first_chance_seen = "veh-first-chance-exception" in record

    if died and required_ok and fatal_seen:
        header = "===== CRASH LOGGER CAPTURED THE FAULT, FATAL PATH INCLUDED ====="
        code = EXIT_OK
    elif died and required_ok and first_chance_seen:
        header = "===== VECTORED HANDLER CAUGHT IT; THE FATAL PATH DID NOT RUN ====="
        body += [
            "",
            "This is a RESULT, not a failure -- exit 0, the experiment ran. What it establishes:",
            "the crash logger installs in-game, the vectored handler sees a real 0xc0000005, and",
            "module+RVA resolution works against the live module table.",
            "",
            "What it does NOT establish, and 4tm asks for both: no record with fatal=true was",
            "written, so SetUnhandledExceptionFilter's callback did not run for this fault, and no",
            "minidump was attempted -- so which tier survives Proton's dbghelp is STILL UNKNOWN.",
            "Do not report the minidump question as answered on the strength of this run.",
        ]
        code = EXIT_OK
    elif died and required_ok:
        header = "===== ARTIFACTS WERE WRITTEN BUT NO EXCEPTION RECORD IS IN THEM ====="
        body += [
            "",
            "The files are fresh, so the logger installed and wrote at startup -- but nothing",
            "recorded the fault. Check whether the fault fired at all before blaming the handler.",
        ]
        code = EXIT_NO_CRASH_EVIDENCE
    elif not died:
        header = "===== THE GAME DID NOT DIE -- the fault never fired ====="
        body += [
            "",
            "The loader armed it (see the log line above) or it did not. Check the log for",
            "'deliberate fault ARMED'. No arm line means the config never reached the DLL.",
        ]
        code = EXIT_NO_CRASH_EVIDENCE
    else:
        header = "===== THE GAME DIED BUT THE LOGGER LEFT NO FRESH EVIDENCE ====="
        body += [
            "",
            "This is the interesting failure: the fault fired and the handler did not produce",
            "its files. Look at whether the vectored handler installed at all -- the loader logs",
            "'crash logger installed' with the previous_unhandled_filter it chained.",
        ]
        code = EXIT_NO_CRASH_EVIDENCE

    width = len(header)
    print("\n" + header)
    for line in body:
        print(line)
    print("=" * width)
    return code


def selftest() -> int:
    """Exercise the log tailer against the rotations it exists to survive.

    This is the only part of the script that can silently turn a good run into a reported
    failure, so it is the only part with a test.
    """
    import tempfile

    ok = True

    def check(condition: bool, label: str) -> None:
        nonlocal ok
        if not condition:
            ok = False
        print(("  ok   " if condition else "  FAIL ") + label)

    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / LOG_NAME

        path.write_text("STALE FROM AN EARLIER RUN\n", encoding="utf-8")
        tail = LogTail(path)
        check(tail.new_text() == "", "a pre-existing log contributes nothing to this run")

        # REPLACED: the DLL rotated the old file aside and made a new one.
        path.rename(path.with_suffix(path.suffix + ".prev"))
        path.write_text(f"{ATTACH_LINE_PREFIX} x\n", encoding="utf-8")
        check(
            ATTACH_LINE_PREFIX in tail.new_text(),
            "a replaced (new-inode) log is read from the top",
        )

        # TRUNCATED IN PLACE: same inode, size drops below our offset.
        with path.open("r+b") as handle:
            handle.truncate(0)
        with path.open("ab") as handle:
            handle.write(b"short\n")
        check("short" in tail.new_text(), "a truncated-in-place log is re-read from the top")

        # PARTIAL LINE: a read landing mid-write must yield nothing, then the whole line.
        with path.open("ab") as handle:
            handle.write(f"{ARXAN_LINE_PREFIX} status=ok detected=".encode())
        check(tail.new_text() == "", "an unterminated line is withheld until it is complete")
        with path.open("ab") as handle:
            handle.write(b"true blocking_entrypoint=true\n")
        text = tail.new_text()
        check(
            text.strip() == f"{ARXAN_LINE_PREFIX} status=ok detected=true blocking_entrypoint=true",
            "the completed line arrives whole on the next poll",
        )
        check(tail.new_text() == "", "a line already handed back is not handed back twice")

        # THE HANDOVER. This is the regression test for the bug that cost the first real M1 run:
        # `await_testimony` returns from the MIDDLE of the chunk it is walking, and everything
        # after the Arxan line in that chunk is already consumed from the tail. The probe writes
        # its install lines from the same callback, milliseconds later, so in a real run they are
        # in that chunk essentially always. Without the handover they are read, dropped, and the
        # run reports "the probe never installed" over a log that plainly contains the install.
        with path.open("ab") as handle:
            handle.write(
                (
                    f"{ARXAN_LINE_PREFIX} status=ok detected=true blocking_entrypoint=true\n"
                    f"{PROBE_LINE_PREFIX} install arm=neuter-arxan rva=0x00832e70\n"
                    f"{PROBE_LINE_PREFIX} watching arm=neuter-arxan\n"
                ).encode()
            )
        handover = await_testimony(tail)
        check(handover["status"] == "confirmed", "the arxan line is still what ends the wait")
        check(
            any("install" in line for line in handover.get("leftover", ())),
            "the install line that arrived in the SAME chunk is handed on, not dropped",
        )
        state = watch_probe(tail, 0.0, handover.get("leftover", ()))
        check(
            state.get("install") is not None,
            "and watch_probe starts with that install line already absorbed",
        )

    check(
        f'name = "{BUILT_DLL.stem}"' in (REPO_ROOT / "crates/ds2-loader/Cargo.toml").read_text(),
        f"the crate really is named to emit {BUILT_DLL.name}",
    )
    loader_src = (REPO_ROOT / "crates/ds2-loader/src/lib.rs").read_text()
    check(f'"{LOG_NAME}"' in loader_src, f"the DLL writes the log this polls for ({LOG_NAME})")
    check(f'"{ARXAN_LINE_PREFIX}"' in loader_src, "the DLL writes the line this gates on")
    check(f'"{ATTACH_LINE_PREFIX}"' in loader_src, "the DLL writes the attach line")

    # THE PROBE CONTRACT. Every string below is spelled in two files that cannot check each other
    # at compile time; a rename on one side turns every run into a false "the probe never
    # installed" or, worse, a false "wrong arm". This is the only place that failure is cheap.
    probe_src = (REPO_ROOT / "crates/ds2-loader/src/arxan_probe.rs").read_text()
    check(f'"{PROBE_LINE_PREFIX}"' in probe_src, f"the DLL writes the probe prefix ({PROBE_LINE_PREFIX})")
    check(f'"{CONFIG_LINE_PREFIX}"' in probe_src, f"the DLL echoes what it read ({CONFIG_LINE_PREFIX})")
    check(f'"{CONFIG_NAME}"' in probe_src, f"the DLL reads the config file this writes ({CONFIG_NAME})")
    check(f'"{CONFIG_SECTION}"' in probe_src, f"the DLL reads the section this writes ([{CONFIG_SECTION}])")
    for key in (KEY_ENABLED, KEY_SKIP_NEUTER, KEY_SITE, KEY_POLL_INTERVAL_MS, KEY_HEARTBEAT_INTERVAL_MS):
        check(f'"{key}"' in probe_src, f"the DLL reads {CONFIG_SECTION}.{key}")
    for arm, (_, expected_arm) in PROBE_ARMS.items():
        if expected_arm is not None:
            check(f'"{expected_arm}"' in probe_src, f"the DLL can report arm={expected_arm} (--probe {arm})")
    rva_src = (REPO_ROOT / "crates/ds2-rva/src/lib.rs").read_text()
    check("ARXAN_PROBE_HOOK_SITE" in rva_src, "the hook site RVA is recorded in ds2-rva")

    # THE ENVIRONMENT IS NO LONGER A CHANNEL, and this is the check that keeps it that way. The
    # variables did not merely stop working -- they were measured arriving unset, because
    # `steam -applaunch` starts the game from an already-running client's environment. Anything
    # that reads configuration back out of the environment reintroduces that failure silently.
    check(
        "std::env::var" not in probe_src and "std::env::var" not in loader_src,
        "the DLL reads NO configuration from the environment",
    )
    check(
        set(launch_env("neuter")) == {"WINEDLLOVERRIDES"},
        "the launch environment carries only WINEDLLOVERRIDES",
    )

    # THE CONFIG FILE ITSELF. It is written by this script and parsed by the DLL, and the two
    # halves are checked here against a parser that mirrors the DLL's rules: `[section]` headers,
    # `key = value`, `#` comments, strict `true`/`false`.
    def parse_config(text: str) -> tuple[dict[tuple[str, str], str], list[str]]:
        """A mirror of `ds2_hotkey_config::kv::KeyValues`, to the extent this file uses it."""
        values: dict[tuple[str, str], str] = {}
        unusable: list[str] = []
        section = ""
        for raw in text.splitlines():
            line = raw.strip()
            if not line or line.startswith("#"):
                continue
            if line.startswith("["):
                if not line.endswith("]"):
                    unusable.append(line)
                    continue
                section = line[1:-1].strip()
                continue
            key, sep, value = line.partition("=")
            if not sep or not key.strip():
                unusable.append(line)
                continue
            value = value.strip()
            if not value.startswith('"') and " #" in value:
                value = value.split(" #", 1)[0].strip()
            values[(section, key.strip())] = value.strip('"')
        return values, unusable

    for arm, (settings, expected_arm) in PROBE_ARMS.items():
        text = config_text(arm)
        values, unusable = parse_config(text)
        check(not unusable, f"--probe {arm} writes a file with no unusable lines")
        for key, wanted in settings.items():
            check(
                values.get((CONFIG_SECTION, key)) == str(wanted).lower(),
                f"--probe {arm} writes [{CONFIG_SECTION}] {key} = {str(wanted).lower()}",
            )
        # The DLL accepts `true` and `false` and nothing else. A generator that emitted Python's
        # `True` would read as a rejected value and the probe would silently stay off.
        check(
            all(
                values.get((CONFIG_SECTION, key)) in ("true", "false")
                for key in (KEY_ENABLED, KEY_SKIP_NEUTER)
            ),
            f"--probe {arm} writes booleans the DLL's strict parser accepts",
        )
        # The two live keys stay commented out, so the DLL's own defaults are what run. Writing
        # them would silently pin the cadence to whatever this script happened to believe.
        check(
            (CONFIG_SECTION, KEY_POLL_INTERVAL_MS) not in values
            and (CONFIG_SECTION, KEY_HEARTBEAT_INTERVAL_MS) not in values,
            f"--probe {arm} leaves the live keys at the DLL's defaults",
        )
        check(
            f"# {KEY_POLL_INTERVAL_MS} = {DEFAULT_POLL_INTERVAL_MS}" in text
            and f"# {KEY_HEARTBEAT_INTERVAL_MS} = {DEFAULT_HEARTBEAT_INTERVAL_MS}" in text,
            f"--probe {arm} documents the live keys and their defaults in the file",
        )

    # THE SITE MUST BE WRITTEN AND MUST DEFAULT TO THE CONTROL. A run that asks for the
    # redirected site and silently gets the clean one would survive and mean nothing, which is
    # the precise failure ds2-mods-rs-by0 exists to prevent.
    for want in PROBE_SITES:
        values, _ = parse_config(config_text("neuter", site=want))
        check(
            values.get((CONFIG_SECTION, KEY_SITE)) == want,
            f'--probe-site {want} writes [{CONFIG_SECTION}] {KEY_SITE} = "{want}"',
        )
    values, _ = parse_config(config_text("neuter"))
    check(
        values.get((CONFIG_SECTION, KEY_SITE)) == "m1",
        "the site defaults to the control, not to the interesting one",
    )

    # OFFLINE MODE IS ON BY DEFAULT, AND THE DEFAULT IS THE SAFE ONE. Every other feature's
    # default is a convenience; this one's is an account. A regression that flipped it to false
    # would show up as a soft-ban long after the commit that caused it, so it is asserted here
    # rather than trusted to the Rust `Default` impl -- the two are separate sources of the same
    # decision and this is the check that they agree.
    values, _ = parse_config(config_text("off"))
    for key in (KEY_OFFLINE_ENABLED, KEY_PIN_FLAG, KEY_REPORT_OFFLINE, KEY_BLOCK_SOCKETS):
        check(
            values.get((OFFLINE_SECTION, key)) == "true",
            f"[{OFFLINE_SECTION}] {key} defaults to true",
        )

    # --no-offline HAS TO REACH ALL FOUR KEYS. The master switch alone would leave three true
    # keys in a file whose header says the run is online, and the next person to read that file
    # would have to know which key the DLL actually consults to tell what happened.
    values, _ = parse_config(config_text("off", offline=False))
    for key in (KEY_OFFLINE_ENABLED, KEY_PIN_FLAG, KEY_REPORT_OFFLINE, KEY_BLOCK_SOCKETS):
        check(
            values.get((OFFLINE_SECTION, key)) == "false",
            f"--no-offline writes [{OFFLINE_SECTION}] {key} = false",
        )

    # THE MEASUREMENT ARM DROPS ONLY THE SOCKET GUARD. If this ever turned off a flag patch too
    # it would stop being a measurement of the flag layer and become a measurement of nothing.
    values, _ = parse_config(config_text("off", block_sockets=False))
    check(
        values.get((OFFLINE_SECTION, KEY_BLOCK_SOCKETS)) == "false"
        and values.get((OFFLINE_SECTION, KEY_PIN_FLAG)) == "true"
        and values.get((OFFLINE_SECTION, KEY_REPORT_OFFLINE)) == "true",
        "--offline-no-socket-block drops the socket guard and keeps both flag patches",
    )

    # THE DLL READS THE SECTION AND THE KEYS THIS WRITES. Same contract the probe section has:
    # a rename on one side alone turns every run into a silent "offline off", which is the one
    # failure mode here that costs something real.
    offline_src = (REPO_ROOT / "crates/ds2-loader/src/offline.rs").read_text(encoding="utf-8")
    check(
        f'"{OFFLINE_SECTION}"' in offline_src,
        f"the DLL reads the section this writes ([{OFFLINE_SECTION}])",
    )
    for key in (KEY_OFFLINE_ENABLED, KEY_PIN_FLAG, KEY_REPORT_OFFLINE, KEY_BLOCK_SOCKETS):
        check(f'"{key}"' in offline_src, f"the DLL reads {OFFLINE_SECTION}.{key}")

    # THE INTRO SKIP IS ON BY DEFAULT, AND MUST STILL BE SWITCHABLE. Removing the boot screens is
    # what the mod is for. The off switch is what keeps it diagnosable: if a run ever fails to
    # boot, ruling this feature out has to cost one config line, not a rebuild.
    values, _ = parse_config(config_text("off"))
    check(
        values.get((INTRO_SECTION, KEY_INTRO_ENABLED)) == "true",
        f"[{INTRO_SECTION}] {KEY_INTRO_ENABLED} defaults to true",
    )
    values, _ = parse_config(config_text("off"))
    check(
        values.get((TITLE_SECTION, KEY_SUBSTATE_FLOORS)) == "true",
        f"[{TITLE_SECTION}] {KEY_SUBSTATE_FLOORS} defaults to true",
    )
    values, _ = parse_config(config_text("off", substate_floors=False))
    check(
        values.get((TITLE_SECTION, KEY_SUBSTATE_FLOORS)) == "false",
        f"--no-substate-floors writes [{TITLE_SECTION}] {KEY_SUBSTATE_FLOORS} = false",
    )
    check(
        values.get((TITLE_SECTION, KEY_PROCESS_WINDOWS)) == "true",
        "--no-substate-floors leaves the process-window de-flooring ON -- different mechanism, "
        "different classes, separate switch",
    )
    values, _ = parse_config(config_text("off"))
    check(
        values.get((TIMELINE_SECTION, KEY_TIMELINE_ENABLED)) == "false",
        f"[{TIMELINE_SECTION}] {KEY_TIMELINE_ENABLED} defaults to FALSE -- an instrument that "
        f"turns itself on is one nobody chose to run",
    )
    values, _ = parse_config(config_text("off", boot_timeline=True))
    check(
        values.get((TIMELINE_SECTION, KEY_TIMELINE_ENABLED)) == "true",
        f"--boot-timeline writes [{TIMELINE_SECTION}] {KEY_TIMELINE_ENABLED} = true",
    )
    check(
        values.get((INTRO_SECTION, KEY_INTRO_ENABLED)) == "true",
        "--boot-timeline leaves every other switch alone",
    )
    values, _ = parse_config(config_text("off", intro_skip=False))
    check(
        values.get((INTRO_SECTION, KEY_INTRO_ENABLED)) == "false",
        f"--no-intro-skip writes [{INTRO_SECTION}] {KEY_INTRO_ENABLED} = false",
    )

    # The rows key stays commented unless asked for, and then carries every row. A subset written
    # from here is the failure the key's own comment records: rows the DLL was told to leave out,
    # read as rows the DLL had lost. And a run with `save-game-to-file` among them does not save by
    # itself, so "which rows" is no longer only about what is on the menu.
    default_rows = config_text("off")
    check(
        f"\n# {KEY_MENU_ROW_ROWS} = [" in default_rows,
        f"[{MENU_ROW_SECTION}] {KEY_MENU_ROW_ROWS} stays commented out by default",
    )
    values, _ = parse_config(default_rows)
    check(
        (MENU_ROW_SECTION, KEY_MENU_ROW_ROWS) not in values,
        "the default run leaves the legacy keys deciding, exactly as the DLL does",
    )
    all_rows = config_text("off", menu_rows_all=True)
    values, _ = parse_config(all_rows)
    written = values.get((MENU_ROW_SECTION, KEY_MENU_ROW_ROWS), "")
    check(
        all(f'"{name}"' in written for name in MENU_ROW_ROW_NAMES),
        f"--all-menu-rows writes every one of the {len(MENU_ROW_ROW_NAMES)} rows, not a subset",
    )
    check(
        SAVE_BLOCK_LOG_PREFIX
        in (REPO_ROOT / "crates/ds2-save-block/src/lib.rs").read_text(encoding="utf-8"),
        "the prefix this script tells you to grep is the one that crate writes",
    )

    # The save folder, whose default has to be empty. An arm that wrote a path nobody asked for
    # would move where a player's progress is written, which is the one setting here that can lose
    # a character, so "unset unless asked" is asserted rather than assumed.
    values, _ = parse_config(config_text("off"))
    check(
        values.get((SAVE_REDIRECT_SECTION, KEY_SAVE_REDIRECT_DIRECTORY)) == "",
        f"[{SAVE_REDIRECT_SECTION}] {KEY_SAVE_REDIRECT_DIRECTORY} defaults to empty -- the game's "
        "own directory",
    )
    values, _ = parse_config(config_text("off", save_directory="Z:\\home\\you\\DS2 Saves\\new"))
    check(
        values.get((SAVE_REDIRECT_SECTION, KEY_SAVE_REDIRECT_DIRECTORY))
        == "Z:\\home\\you\\DS2 Saves\\new",
        f"--save-dir writes [{SAVE_REDIRECT_SECTION}] {KEY_SAVE_REDIRECT_DIRECTORY}, backslashes "
        "and spaces intact",
    )
    # The conversion the flag does before it ever reaches the config. A path that is one separator
    # wrong produces a game with no saves and nothing on screen to say why, so both directions are
    # pinned: a Linux path is converted, and a path that is already Windows is left alone.
    check(
        windows_path("/home/you/DS2 Saves/new") == "Z:\\home\\you\\DS2 Saves\\new",
        "windows_path converts a Linux path to the Z: spelling the prefix sees",
    )
    check(
        windows_path("Z:\\home\\you\\DS2 Saves\\new") == "Z:\\home\\you\\DS2 Saves\\new",
        "windows_path leaves a drive-lettered path exactly as typed",
    )
    check(windows_path("") == "", "windows_path leaves an unset value unset")

    # THE DIALOG SKIP IS A SEPARATE SWITCH, and the point of asserting both here is that they are
    # INDEPENDENT. One flag turning both off would make a boot failure attributable to "the mod"
    # instead of to a feature, which is the whole thing these switches exist to prevent.
    values, _ = parse_config(config_text("off"))
    check(
        values.get((DIALOG_SECTION, KEY_DIALOG_ENABLED)) == "true",
        f"[{DIALOG_SECTION}] {KEY_DIALOG_ENABLED} defaults to true",
    )
    values, _ = parse_config(config_text("off", dialog_skip=False))
    check(
        values.get((DIALOG_SECTION, KEY_DIALOG_ENABLED)) == "false",
        f"--no-dialog-skip writes [{DIALOG_SECTION}] {KEY_DIALOG_ENABLED} = false",
    )
    values, _ = parse_config(config_text("off", intro_skip=False))
    check(
        values.get((DIALOG_SECTION, KEY_DIALOG_ENABLED)) == "true",
        "--no-intro-skip leaves the dialog skip ON -- the two switches are independent",
    )

    # THE TITLE SKIPS ARE TWO MORE INDEPENDENT SWITCHES. Four hooks now patch executable memory at
    # startup, and the whole value of separate keys is that a run that fails to boot can be pinned
    # on ONE of them without a rebuild. Asserting the independence is what keeps that true.
    values, _ = parse_config(config_text("off"))
    check(
        values.get((TITLE_SECTION, KEY_PRESS_ANY_BUTTON)) == "true"
        and values.get((TITLE_SECTION, KEY_PROCESS_WINDOWS)) == "true",
        f"[{TITLE_SECTION}] both keys default to true",
    )
    values, _ = parse_config(config_text("off", press_any_button=False))
    check(
        values.get((TITLE_SECTION, KEY_PRESS_ANY_BUTTON)) == "false"
        and values.get((TITLE_SECTION, KEY_PROCESS_WINDOWS)) == "true",
        "--no-press-any-button-skip turns off only its own key",
    )
    values, _ = parse_config(config_text("off", process_windows=False))
    check(
        values.get((TITLE_SECTION, KEY_PROCESS_WINDOWS)) == "false"
        and values.get((TITLE_SECTION, KEY_PRESS_ANY_BUTTON)) == "true",
        "--no-process-window-skip turns off only its own key",
    )
    values, _ = parse_config(config_text("off", hide_process_windows=False))
    check(
        values.get((TITLE_SECTION, KEY_HIDE_PROCESS_WINDOWS)) == "false"
        and values.get((TITLE_SECTION, KEY_PROCESS_WINDOWS)) == "true",
        "--no-hide-process-windows falls back to shortening rather than leaving them alone",
    )
    values, _ = parse_config(config_text("off", title_sequence_gate=False))
    check(
        values.get((TITLE_SECTION, KEY_TITLE_SEQUENCE_GATE)) == "false"
        and values.get((TITLE_SECTION, KEY_PRESS_ANY_BUTTON)) == "true",
        "--no-title-sequence-skip turns off only its own key",
    )
    values, _ = parse_config(config_text("off", title_settle=False))
    check(
        values.get((TITLE_SECTION, KEY_TITLE_SETTLE)) == "false"
        and values.get((TITLE_SECTION, KEY_TITLE_ANIMATION)) == "true",
        "--no-title-settle turns off only its own key, not the detour it shares",
    )
    values, _ = parse_config(config_text("off", title_animation=False))
    check(
        values.get((TITLE_SECTION, KEY_TITLE_ANIMATION)) == "false"
        and values.get((TITLE_SECTION, KEY_PRESS_ANY_BUTTON)) == "true",
        "--no-title-animation-skip turns off only its own key",
    )

    # THE MENU KEY IS IN A DIFFERENT SECTION, and that is the point worth asserting: it is not a
    # skip, it patches different functions, and a boot failure has to be attributable to it alone.
    values, _ = parse_config(config_text("off"))
    check(
        values.get((MENU_SECTION, KEY_SHOW_UNAVAILABLE)) == "false",
        f"[{MENU_SECTION}] {KEY_SHOW_UNAVAILABLE} defaults to FALSE -- the game's own menu already "
        "draws every row, dims what it cannot offer, and leaves no gap",
    )
    values, _ = parse_config(config_text("off", show_unavailable=True))
    check(
        values.get((MENU_SECTION, KEY_SHOW_UNAVAILABLE)) == "true"
        and values.get((TITLE_SECTION, KEY_TITLE_SETTLE)) == "true"
        and values.get((DIALOG_SECTION, KEY_DIALOG_ENABLED)) == "true",
        "--show-unavailable-menu-rows turns on only its own key, in its own section",
    )
    values, _ = parse_config(config_text("off", title_settle=False))
    check(
        values.get((MENU_SECTION, KEY_SHOW_UNAVAILABLE)) == "false",
        "--no-title-settle leaves the menu key OFF -- different section, different hooks",
    )

    # THE LEGACY KEY IS NEVER WRITTEN, in any arm, and that is what makes a launch show every row.
    # Set, it selects the DLL's legacy branch, which registers quit-to-desktop ALONE; every row is
    # registered only when neither legacy key is present, which the DLL reports as `Default`. There
    # was a `--menu-row` flag that wrote it, and on 2026-09-23 it cost a run: launched to exercise a
    # save-file row, it logged `source=LegacyEnabled rows=[quit-to-desktop]` and the row under test
    # was not in the menu. Same reasoning as the `rows` assertion further down, and the flag is gone.
    for arm in PROBE_ARMS:
        values, _ = parse_config(config_text(arm))
        check(
            (MENU_ROW_SECTION, KEY_MENU_ROW_ENABLED) not in values,
            f"[{MENU_ROW_SECTION}] {KEY_MENU_ROW_ENABLED} is COMMENTED OUT in the {arm} arm -- "
            "written, it narrows the menu to one row",
        )
    values, _ = parse_config(config_text("off", show_unavailable=True))
    check(
        (MENU_ROW_SECTION, KEY_MENU_ROW_ENABLED) not in values
        and values.get((MENU_SECTION, KEY_SHOW_UNAVAILABLE)) == "true",
        "--show-unavailable-menu-rows touches the title menu's key and not the pause menu's -- "
        "different menus, different hooks",
    )
    # SPELT IN HALVES for the same reason the `--rows` check below is: a check that searches its own
    # file for a literal finds the literal it is written with.
    legacy_flags = ('dest="menu_' "row\"", 'dest="build_' "import\"")
    self_source = Path(__file__).read_text(encoding="utf-8")
    for spelling in legacy_flags:
        check(
            spelling not in self_source,
            "neither legacy key has a flag of its own again -- a flag named after the rows that "
            f"takes three of them away is how this was lost the first time ({spelling})",
        )

    # THE DLL READS THE SECTION AND THE KEY THIS WRITES, and writes the prefix this file tells the
    # reader to grep for. Same contract as the probe and offline sections.
    menu_row_src = (REPO_ROOT / "crates/ds2-loader/src/menu_row.rs").read_text(encoding="utf-8")
    check(
        f'"{MENU_ROW_SECTION}"' in menu_row_src,
        f"the DLL reads the section this writes ([{MENU_ROW_SECTION}])",
    )
    check(
        f'"{KEY_MENU_ROW_ENABLED}"' in menu_row_src,
        f"the DLL reads {MENU_ROW_SECTION}.{KEY_MENU_ROW_ENABLED}",
    )
    menu_row_lib = (REPO_ROOT / "crates/ds2-menu-row/src/lib.rs").read_text(encoding="utf-8")
    check(
        f'"{MENU_ROW_LOG_PREFIX}"' in menu_row_lib,
        f"the DLL writes the prefix this config tells the reader to look for "
        f"({MENU_ROW_LOG_PREFIX})",
    )

    # THE ROW LIST. Four names against twelve slots, so the count and every spelling are checked
    # here rather than discovered in a log after a launch. The rows key is the only key in this file
    # whose value is a LIST, and the reason it started as one was the game's item vector; see
    # `MENU_ROW_MAX_ADDED` for what replaced that ceiling. This script no longer writes it -- the
    # names are still checked because the commented line it emits offers every one of them.
    for name in MENU_ROW_ROW_NAMES:
        check(
            f'"{name}"' in menu_row_src,
            f"the DLL knows the row name this script offers ({name})",
        )
    check(
        f'"{KEY_MENU_ROW_ROWS}"' in menu_row_src,
        f"the DLL reads the list key this script writes ({KEY_MENU_ROW_ROWS})",
    )
    # The ceiling this script refuses on is the one the DLL refuses on, spelled in the crate that
    # owns it. A script that allowed a third row would produce a config whose third row vanishes.
    menu_row_api = (REPO_ROOT / "crates/ds2-menu-row/src/api.rs").read_text(encoding="utf-8")
    check(
        f"assert_eq!(MAX_ADDED_ROWS, {MENU_ROW_MAX_ADDED});" in menu_row_api,
        f"MAX_ADDED_ROWS is still {MENU_ROW_MAX_ADDED}, which is the ceiling the commented line "
        "above offers",
    )
    values, _ = parse_config(config_text("off"))
    check(
        (MENU_ROW_SECTION, KEY_MENU_ROW_ROWS) not in values,
        f"[{MENU_ROW_SECTION}] {KEY_MENU_ROW_ROWS} is COMMENTED OUT in every config this script "
        "writes -- present, it overrides the enabled keys, and a launcher that narrowed the row "
        "set turned four shipped rows into two and read as a regression in the DLL",
    )
    # SPELT IN TWO HALVES SO THIS LINE IS NOT ITSELF THE MATCH. A check that searches its own file
    # for a literal finds the literal it is written with, and fails forever; the halves are joined
    # at run time and the contiguous bytes never appear in the source.
    readded = 'dest="menu_row_' 'rows"'
    check(
        readded not in Path(__file__).read_text(encoding="utf-8"),
        "there is no --rows flag: the launcher launches the default row set, and a narrower set is "
        "an edit to the config file where the choice sits next to what it changes",
    )

    # THE HANDOFF FILE, named in this config's prose and written by the DLL. A drifted spelling would
    # send a player looking for a file that is never created, and leave the real one uncollected.
    handoff_src = (REPO_ROOT / "crates/ds2-save-file-core/src/handoff.rs").read_text(
        encoding="utf-8"
    )
    check(
        f'"{SAVE_FILE_HANDOFF_NAME}"' in handoff_src,
        f"the DLL writes the handoff file this config names ({SAVE_FILE_HANDOFF_NAME})",
    )
    save_file_lib = (REPO_ROOT / "crates/ds2-save-file/src/lib.rs").read_text(encoding="utf-8")
    check(
        f'"{SAVE_FILE_LOG_PREFIX}"' in save_file_lib,
        f"the DLL writes the prefix this config tells the reader to look for "
        f"({SAVE_FILE_LOG_PREFIX})",
    )

    # THE SAME CONTRACT FOR THE BUILD-IMPORT ROW. It is a separate section on purpose -- one row
    # measures whether a fourth row draws, the other opens a Steam overlay and talks to the network
    # -- so a run that misbehaves is attributable to one of them by editing one line.
    for arm in PROBE_ARMS:
        values, _ = parse_config(config_text(arm))
        check(
            (BUILD_IMPORT_SECTION, KEY_BUILD_IMPORT_ENABLED) not in values,
            f"[{BUILD_IMPORT_SECTION}] {KEY_BUILD_IMPORT_ENABLED} is COMMENTED OUT in the {arm} "
            "arm -- it is the other half of the legacy branch, and writing either half narrows "
            "the menu to the rows those two keys name",
        )
    build_import_src = (REPO_ROOT / "crates/ds2-loader/src/build_import.rs").read_text(
        encoding="utf-8"
    )
    check(
        f'"{BUILD_IMPORT_SECTION}"' in build_import_src,
        f"the DLL reads the section this writes ([{BUILD_IMPORT_SECTION}])",
    )
    check(
        f'"{KEY_BUILD_IMPORT_ENABLED}"' in build_import_src,
        f"the DLL reads {BUILD_IMPORT_SECTION}.{KEY_BUILD_IMPORT_ENABLED}",
    )
    build_import_lib = (REPO_ROOT / "crates/ds2-build-import/src/lib.rs").read_text(
        encoding="utf-8"
    )
    check(
        f'"{BUILD_IMPORT_LOG_PREFIX}"' in build_import_lib,
        f"the DLL writes the prefix this config tells the reader to look for "
        f"({BUILD_IMPORT_LOG_PREFIX})",
    )
    # EVERY ROW IS REGISTERED FROM ONE PLACE, and that is an invariant in the loader rather than
    # anything this file writes. `ds2_menu_row::install` seals the registry as its first act, so a row
    # registered after it is a row that silently never appears -- and a row registered from some other
    # `install_*` would take a slot out of the order a `rows` list asked for. `install_menu_row` loops
    # over the selection and is the only caller of `register_row`; the checks below are what say so.
    loader_src = (REPO_ROOT / "crates/ds2-loader/src/lib.rs").read_text(encoding="utf-8")
    check(
        loader_src.count("install_menu_row();") == 2,
        "both Arxan arms register the rows, or the A/B pair stops being comparable",
    )
    check(
        loader_src.count("register_row(*row);") == 1
        and loader_src.count("fn register_row(") == 1,
        "one registration loop and one `register_row`, so the list's order is the screen's order",
    )
    for callee in (
        "ds2_build_import::register(",
        "ds2_save_file::register_import_row(",
        "ds2_save_file::register_export_row(",
        "ds2_menu_row::add_row(",
    ):
        check(
            loader_src.count(callee) == 1,
            f"{callee}...) is reached from exactly one place, which is `register_row`",
        )

    # THE SORT REBINDING, whose whole risk is that it CALLS a shipped function on the game thread
    # rather than only patching one. Same two questions as every other feature: is it off unless
    # asked for, and does the DLL read the section this writes.
    values, _ = parse_config(config_text("off"))
    check(
        values.get((INVENTORY_SORT_SECTION, KEY_INVENTORY_SORT_ENABLED)) == "true"
        and values.get((INVENTORY_SORT_SECTION, KEY_INVENTORY_SORT_PAD)) == "lthumb",
        f"[{INVENTORY_SORT_SECTION}] {KEY_INVENTORY_SORT_ENABLED} defaults to TRUE, bound to "
        "lthumb -- both menus and the L3 binding have been pressed in game",
    )
    values, _ = parse_config(config_text("off", inventory_sort=False))
    check(
        values.get((INVENTORY_SORT_SECTION, KEY_INVENTORY_SORT_ENABLED)) == "false"
        and values.get((INVENTORY_SORT_SECTION, KEY_INVENTORY_SORT_PAD)) == "lthumb",
        "--no-inventory-sort turns off only its own key, in its own section",
    )
    values, _ = parse_config(
        config_text("off", inventory_sort=True, inventory_sort_key="", inventory_sort_pad="y")
    )
    check(
        values.get((INVENTORY_SORT_SECTION, KEY_INVENTORY_SORT_KEY)) == ""
        and values.get((INVENTORY_SORT_SECTION, KEY_INVENTORY_SORT_PAD)) == "y",
        "a controller-only binding is writable -- an empty key is 'no keyboard binding', not a "
        "missing value",
    )
    inventory_sort_src = (REPO_ROOT / "crates/ds2-loader/src/inventory_sort.rs").read_text(
        encoding="utf-8"
    )
    check(
        f'"{INVENTORY_SORT_SECTION}"' in inventory_sort_src,
        f"the DLL reads the section this writes ([{INVENTORY_SORT_SECTION}])",
    )
    inventory_sort_lib = (REPO_ROOT / "crates/ds2-inventory-sort/src/lib.rs").read_text(
        encoding="utf-8"
    )
    check(
        f'"{KEY_INVENTORY_SORT_KEY}"' in inventory_sort_lib
        and f'"{KEY_INVENTORY_SORT_PAD}"' in inventory_sort_lib,
        "the DLL reads both binding keys this writes",
    )
    # SAME SEALING ORDER AS THE ROWS, for the same reason: the button is read from a per-frame tick
    # that `ds2_menu_row::install` seals the registry for, so a tick registered after it is a button
    # nothing ever reads.
    check(
        loader_src.count("install_inventory_sort();") == 2,
        "both Arxan arms install the sort rebinding, or the A/B pair stops being comparable",
    )
    check(
        loader_src.index("install_inventory_sort();") < loader_src.index("install_menu_row();"),
        "install_inventory_sort runs BEFORE install_menu_row, which seals the tick registry",
    )

    # THE ARMS MUST DIFFER, and in exactly one key. If two arms ever generated the same file the
    # A/B comparison would be two runs of the same experiment, and the arm-readback guard would
    # not catch it because the DLL would be reporting truthfully.
    neuter_values, _ = parse_config(config_text("neuter"))
    skip_values, _ = parse_config(config_text("skip-neuter"))
    differing = {key for key in neuter_values | skip_values if neuter_values.get(key) != skip_values.get(key)}
    check(
        differing == {(CONFIG_SECTION, KEY_SKIP_NEUTER)},
        f"the two arms differ in exactly one key ({KEY_SKIP_NEUTER})",
    )
    check(
        config_text("neuter") == config_text("neuter"),
        "the generated config is deterministic -- same arm, same bytes",
    )

    # WRITING IT, for real, to a temp directory. `write_config` is the one function here that
    # touches the game directory before a launch, and a run whose config never landed looks
    # exactly like a run whose probe never installed.
    with tempfile.TemporaryDirectory() as tmp:
        directory = Path(tmp)
        path, written = write_config(directory, "skip-neuter")
        check(path == directory / CONFIG_NAME, f"the config is written as {CONFIG_NAME}")
        check(path.read_text(encoding="utf-8") == written, "what was written is what was returned")
        check(
            parse_config(path.read_text(encoding="utf-8"))[0][(CONFIG_SECTION, KEY_SKIP_NEUTER)]
            == "true",
            "the file on disk parses back to the arm that was requested",
        )
        # REWRITING OVER THE OTHER ARM. Both arms run back to back with no user action between
        # them, so the second run must fully replace the first run's file rather than merge with
        # it or append to it.
        _, rewritten = write_config(directory, "neuter")
        check(
            path.read_text(encoding="utf-8") == rewritten == config_text("neuter"),
            "a second arm's write REPLACES the first arm's file",
        )
        check(
            parse_config(path.read_text(encoding="utf-8"))[0][(CONFIG_SECTION, KEY_SKIP_NEUTER)]
            == "false",
            "and the replaced file parses back to the second arm",
        )

    # The transcript has to carry the file, or the arm under test is invisible to whoever reads
    # the block later.
    quoted = quoted_config(config_text("skip-neuter"), indent="    | ")
    check(
        f"{KEY_SKIP_NEUTER} = true" in quoted and quoted.startswith("    | "),
        "the config is quoted into the transcript verbatim and indented",
    )

    # THE VERDICT LOGIC. It is the only code here that turns lines into a conclusion, so it is the
    # only code here that can turn a real finding into the wrong headline. Every branch that a run
    # can actually reach gets a synthetic log.
    def verdict_for(lines: list[str], requested: str = "neuter", **overrides) -> tuple[str, int]:
        state = new_probe_state()
        for line in lines:
            absorb_probe_line(line, state)
        state.update(overrides)
        return probe_block(requested, state)

    installed = [
        f"{PROBE_LINE_PREFIX} install arm=neuter-arxan base=0x0000000140000000 rva=0x00832e70 va=0x0000000140832e70",
        f"{PROBE_LINE_PREFIX} install original=[48 89 5c 24 08 57 48 83 ec 20 48 8b d9 48 8b 0d] expected=[48 89 5c 24 08] prologue-match=true",
        f"{PROBE_LINE_PREFIX} install minhook=ok trampoline=0x0000000012340000 patched=[e9 8b 1a 3c ff 57 48 83 ec 20 48 8b d9 48 8b 0d] site-jmp=true",
        f"{PROBE_LINE_PREFIX} watching arm=neuter-arxan poll=1.0s heartbeat=10.0s site-window=16 trampoline-window=64",
    ]
    healthy = f"{PROBE_LINE_PREFIX} heartbeat uptime=180.0s arm=neuter-arxan hits=48213991 site=intact tramp=intact site-diverged=0 tramp-diverged=0"

    block, code = verdict_for(installed + [healthy])
    check(code == EXIT_OK and "SURVIVED and FIRED" in block, "a clean run reads as survived")
    check("hits=48213991" in block, "the hit count is quoted, not summarised")

    silent = f"{PROBE_LINE_PREFIX} heartbeat uptime=180.0s arm=neuter-arxan hits=0 site=intact tramp=intact site-diverged=0 tramp-diverged=0"
    block, code = verdict_for(installed + [silent])
    check(
        code == EXIT_OK and "never called" in block and "SURVIVED" not in block,
        "an intact patch that never fired is NOT reported as a surviving detour",
    )

    reverted = [
        f"{PROBE_LINE_PREFIX} SITE uptime=41.0s arm=neuter-arxan state=DIVERGED prev=intact hits=98765 va=0x0000000140832e70 expected=[e9 8b 1a 3c ff 57 48 83 ec 20 48 8b d9 48 8b 0d] observed=[48 89 5c 24 08 57 48 83 ec 20 48 8b d9 48 8b 0d]",
        f"{PROBE_LINE_PREFIX} heartbeat uptime=50.0s arm=neuter-arxan hits=98765 site=DIVERGED tramp=intact site-diverged=1 tramp-diverged=0",
    ]
    block, code = verdict_for(installed + reverted)
    check(code == EXIT_OK, "a reverted hook still EXITS ZERO -- the experiment ran")
    check("HOOK SITE WAS REVERTED" in block, "a reverted hook says so")
    check("observed=[48 89 5c 24 08" in block, "the observed bytes are reproduced verbatim")

    corrupt = f"{PROBE_LINE_PREFIX} heartbeat uptime=50.0s arm=neuter-arxan hits=0 site=intact tramp=DIVERGED site-diverged=0 tramp-diverged=1"
    block, _ = verdict_for(installed + [corrupt])
    check(
        "TRAMPOLINE WAS CORRUPTED" in block,
        "a corrupt trampoline is reported even though the site looks intact",
    )

    block, code = verdict_for(installed + [healthy], requested="skip-neuter")
    check(
        code == EXIT_NO_PROBE_VERDICT and "WRONG ARM" in block,
        "a log whose arm is not the requested one REFUSES to produce a verdict",
    )

    block, code = verdict_for([], requested="neuter")
    check(code == EXIT_NO_PROBE_VERDICT and "never installed" in block, "no install line, no verdict")

    void = f"{PROBE_LINE_PREFIX} VOID prologue-mismatch va=0x0000000140832e70 -- already patched"
    block, code = verdict_for([void])
    check(code == EXIT_NO_PROBE_VERDICT and "refused the hook site" in block, "a VOID run is not a result")

    block, code = verdict_for(installed)
    check(code == EXIT_NO_PROBE_VERDICT and "never reported a heartbeat" in block, "installed but silent is not a result")

    # A config edited mid-window is not a footnote: the measurement cadence may have changed
    # underneath the numbers above, and a reader comparing two arms has to be told.
    touched = [
        f"{PROBE_LINE_PREFIX} config uptime=30.0s RELOADED poll=1000ms heartbeat=10000ms -> poll=1000ms heartbeat=60000ms",
    ]
    block, code = verdict_for(installed + touched + [healthy])
    check(code == EXIT_OK, "a mid-run config reload is not itself a failure")
    check("CONFIG FILE WAS TOUCHED" in block, "a mid-run config reload is reported, not swallowed")
    check("heartbeat=60000ms" in block, "the reload is quoted verbatim")

    ignored = [
        f"{PROBE_LINE_PREFIX} config uptime=30.0s STARTUP-ONLY-IGNORED enabled=\"true\" skip_neuter=\"true\" -- this run is still arm=neuter-arxan",
    ]
    block, code = verdict_for(installed + ignored + [healthy])
    check(
        code == EXIT_OK and "STARTUP-ONLY-IGNORED" in block,
        "an attempt to switch arms mid-run is reported and does NOT change the arm",
    )
    check(
        "arm            neuter-arxan" in block,
        "and the verdict still reports the arm that actually ran",
    )

    block, _ = verdict_for(installed + [healthy])
    check("CONFIG FILE WAS TOUCHED" not in block, "an untouched config says nothing at all")

    block, _ = verdict_for(installed + [healthy], detach=f"{PROBE_LINE_PREFIX} detach uptime=612.4s arm=neuter-arxan hits=99 site=intact tramp=intact site-diverged=0 tramp-diverged=0")
    check("detach uptime=612.4s" in block, "an orderly exit is reported as one")
    block, _ = verdict_for(installed + [healthy])
    check("did not wind down through" in block, "a missing detach line is reported as a possible crash")

    # ---- the crash test's config and its evidence check -------------------------------------
    # These guard the two ways a crash test can lie: writing a config that does not actually arm
    # the fault, and calling a run successful on artifacts left over from a previous one.
    off_text = config_text("off")
    check(
        f"{KEY_FAULT_AFTER_MS} = {NO_FAULT_MS}" in off_text,
        "an ordinary run writes fault_after_ms = 0 -- a crash is never a default",
    )
    check(
        "ARMED TO CRASH" not in off_text,
        "and does not shout about crashing",
    )
    armed_text = config_text("off", 15000)
    check(
        f"{KEY_FAULT_AFTER_MS} = 15000" in armed_text,
        "--crash-test 15000 writes fault_after_ms = 15000",
    )
    check(
        "THIS RUN IS ARMED TO CRASH ON PURPOSE" in armed_text,
        "and the file says so, in the file the user reads",
    )
    check(
        f"[{CRASH_SECTION}]" in off_text and f"{KEY_CRASH_ENABLED} = true" in off_text,
        "crash logging is written ON for every run, not just crash tests",
    )
    check(
        f"{KEY_REINSTALL_FILTER_AFTER_MS} = {DEFAULT_REINSTALL_FILTER_AFTER_MS}" in off_text,
        "and the filter re-assert is written ON -- without it no fatal record is ever produced",
    )

    with tempfile.TemporaryDirectory() as tmp:
        global GAME_DIR  # noqa -- the check is about GAME_DIR's contents by design
        real_game_dir = GAME_DIR
        try:
            GAME_DIR = Path(tmp)
            launched = datetime.now(timezone.utc)
            launched_iso = launched.isoformat(timespec="seconds")

            _, required_ok = crash_artifact_report(launched_iso, time.time())
            check(not required_ok, "no artifacts at all is not a pass")

            # Stale: written a full hour before this run launched.
            stale = launched.timestamp() - 3600
            for name in CRASH_ARTIFACTS:
                target = GAME_DIR / name
                target.write_text("from an earlier run\n", encoding="utf-8")
                os.utime(target, (stale, stale))
            lines, required_ok = crash_artifact_report(launched_iso, time.time())
            check(not required_ok, "artifacts older than the launch are STALE, not evidence")
            check(
                any("STALE" in line for line in lines),
                "and the report names them as stale rather than passing them off",
            )

            # Fresh: written after the launch, as this run's logger would.
            for name in CRASH_ARTIFACTS:
                (GAME_DIR / name).write_text("this run\n", encoding="utf-8")
            _, required_ok = crash_artifact_report(launched_iso, time.time())
            check(required_ok, "fresh artifacts are a pass")

            # The minidump is optional: er-mods-rs saw every tier rejected by Proton's dbghelp,
            # so a text-complete run with no dump must still pass and report the absence.
            (GAME_DIR / "ds2-crash-minidump.dmp").unlink()
            lines, required_ok = crash_artifact_report(launched_iso, time.time())
            check(
                required_ok,
                "a missing minidump does NOT fail the run -- which tier survives Proton is the question",
            )
            check(
                any("MISSING" in line and "minidump" in line for line in lines),
                "but the absent dump is still reported",
            )

            # A missing TEXT artifact is a genuine failure.
            (GAME_DIR / "ds2-crash-latest.txt").unlink()
            _, required_ok = crash_artifact_report(launched_iso, time.time())
            check(not required_ok, "a missing text artifact IS a failure")
        finally:
            GAME_DIR = real_game_dir

    print("selftest: " + ("OK" if ok else "FAILED"))
    return EXIT_OK if ok else EXIT_ERROR


def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="report what a run would do; stage nothing, launch nothing",
    )
    parser.add_argument(
        "--selftest",
        action="store_true",
        help="test the log tailer, the DLL/script contract and the verdict logic",
    )
    parser.add_argument(
        "--no-substate-floors",
        dest="substate_floors",
        action="store_false",
        help=(
            "leave the one-second floors on SteamLoadSystemData and Information in place. They "
            "are ~1.86s of a 6.7s boot, both spent after the work finished, so this is the switch "
            "that says whether a boot failure is theirs."
        ),
    )
    parser.add_argument(
        "--no-offline",
        dest="offline",
        action="store_false",
        default=True,
        help=(
            "PLAY ONLINE WITH A MODDED CLIENT. Offline mode is on by default and this is the "
            "switch that turns it off. Everything else this DLL does patches game code in memory, "
            "which is what FromSoftware's matchmaking servers watch for, so an online run with "
            "the mod loaded is an account risk you are taking on purpose."
        ),
    )
    parser.add_argument(
        "--offline-no-socket-block",
        dest="block_sockets",
        action="store_false",
        default=True,
        help=(
            "keep offline mode's two flag patches and drop its socket guard. THE MEASUREMENT ARM: "
            "FeSubStateTitleGameServerLogin's work starter does not read the online flag, so this "
            "run is the one that shows how much traffic the flag layer never reaches. It talks to "
            "FromSoftware's servers if the flag layer misses something -- that is the point."
        ),
    )
    parser.add_argument(
        "--continue-slot",
        dest="continue_slot",
        type=int,
        default=None,
        metavar="N",
        help=(
            "open the character list on slot N (0-9) instead of the game's own selection. "
            "Negative leaves it alone. The list still opens; only the cursor is placed. "
            "WITH --save-redirect AND NO VALUE, the slot is read from the redirected save and the "
            "first occupied one is used -- the slot belongs to the save, not to your account."
        ),
    )
    parser.add_argument(
        "--no-continue-hide-menus",
        dest="continue_hide_menus",
        action="store_false",
        help=(
            "let the top menu and the character list draw themselves as the autocontinue passes "
            "through. They are closed by default whenever --continue-slot is set."
        ),
    )
    parser.add_argument(
        "--no-continue-silence",
        dest="continue_silence",
        action="store_false",
        help=(
            "let the title music and the menu confirm sounds play through the autocontinue. "
            "They are muted by default, because nobody pressed the buttons making the noise."
        ),
    )
    parser.add_argument(
        "--continue-record",
        dest="continue_record",
        action="store_true",
        help=(
            "record which save slot a load actually uses: one line per change of slot, action or "
            "phase in the character list. Records only -- it drives nothing."
        ),
    )
    parser.add_argument(
        "--boot-timeline",
        dest="boot_timeline",
        action="store_true",
        help=(
            "instrument the title flow: log every substate entered and left, with milliseconds "
            "from DllMain. OFF unless asked for -- this is the measurement run, not a fix."
        ),
    )
    parser.add_argument(
        "--no-intro-skip",
        dest="intro_skip",
        action="store_false",
        default=True,
        help=(
            "leave the boot logo, no-copy warning and user-policy screens in place. They are "
            "skipped by default, by detouring each substate's `enter` and writing the terminal "
            "phase it already writes itself under the game's own skip conditions. Pass this to "
            "rule the feature out when a run fails to boot."
        ),
    )
    parser.add_argument(
        "--no-dialog-skip",
        dest="dialog_skip",
        action="store_false",
        default=True,
        help=(
            "leave the title-flow message boxes waiting for a button. They are answered by "
            "default, by writing the same result byte a press writes and letting the game's own "
            "dispatch close the box. Pass this to rule the feature out when a run fails to boot, "
            "or to read a dialog this mod would otherwise dismiss."
        ),
    )
    parser.add_argument(
        "--no-press-any-button-skip",
        dest="press_any_button",
        action="store_false",
        default=True,
        help=(
            "leave the PRESS ANY BUTTON gate waiting for a button. It is forced by default, by "
            "detouring the poll behind it -- which has exactly one caller in the whole image, so "
            "the change reaches that gate and not input handling."
        ),
    )
    parser.add_argument(
        "--no-process-window-skip",
        dest="process_windows",
        action="store_false",
        default=True,
        help=(
            "leave the 'please wait' windows their minimum display time. By default that floor is "
            "zeroed so they close as soon as their work is actually done. The work itself is "
            "never skipped -- only the time the window lingers after finishing."
        ),
    )
    parser.add_argument(
        "--no-hide-process-windows",
        dest="hide_process_windows",
        action="store_false",
        default=True,
        help=(
            "draw the 'please wait' windows, shortened rather than hidden. They are hidden by "
            "default by reproducing their `enter` without its one drawing call -- the work is "
            "still started and still waited for. This falls back to the milder behaviour."
        ),
    )
    parser.add_argument(
        "--no-title-animation-skip",
        dest="title_animation",
        action="store_false",
        default=True,
        help=(
            "keep the title screen's activation flourish. By default its terminal phase is "
            "written once the phase-1 body that builds the top menu has run, so only the "
            "animation is skipped."
        ),
    )
    parser.add_argument(
        "--no-title-sequence-skip",
        dest="title_sequence_gate",
        action="store_false",
        default=True,
        help=(
            "wait for the title logo and prompt to finish animating in before the forced press is "
            "accepted. That wait is the animation, so leaving it on means --no-press-any-button "
            "-style behaviour is visible even though the press itself is still forced."
        ),
    )
    parser.add_argument(
        "--no-title-settle",
        dest="title_settle",
        action="store_false",
        default=True,
        help=(
            "let the title scene play its intro sequence out instead of putting it straight into "
            "its settled state. On by default: settling it is what makes the menu usable as soon "
            "as its data is available rather than being paced by an animation."
        ),
    )
    parser.add_argument(
        "--show-unavailable-menu-rows",
        dest="show_unavailable",
        action="store_true",
        default=False,
        help=(
            "override the title menu's own look for rows it cannot offer. OFF BY DEFAULT, and the "
            "default was set by a control run rather than by taste: with this off, the game draws "
            "every row anyway, dims an unavailable Continue instead of removing it, and swaps "
            "INFORMATION and GO ONLINE inside one shared slot with no gap. Turning this on forces "
            "the enable byte and then plays the faded sequence over the result, which is the "
            "segment that leads into the row being removed -- it poses the row invisible."
        ),
    )
    # THERE IS NO --menu-row AND NO --build-import. Both wrote the DLL's legacy `enabled` key, and
    # that key selects a branch which registers quit-to-desktop ALONE -- so the flags named after
    # the pause-menu rows were the ones that took the other three away. The launcher writes neither,
    # which is what the DLL reads as `Default`: every row. `--selftest` pins both spellings out.
    parser.add_argument(
        "--no-inventory-sort",
        dest="inventory_sort",
        action="store_false",
        default=True,
        help=(
            "TURN OFF the sort button. It is ON by default and bound to L3, which is where ELDEN "
            "RING puts Sort. It adds no sorting: DARK SOULS II already sorts, on a button no menu "
            "in the game can rebind -- and on the equip screen, on no button at all. Six detours "
            "(each menu's constructor, destructor and per-frame update) and one direct call into "
            "the shipped dialog entry, which refuses itself while another dialog is up. Move the "
            "button with --sort-key and --sort-pad; both change while the game runs by editing the "
            "config file."
        ),
    )
    parser.add_argument(
        "--sort-key",
        dest="inventory_sort_key",
        default="F7",
        help=(
            "key NAME for --inventory-sort, in `ds2-hotkey-config` spelling: F7, ], KP_Plus, "
            "Insert, Ctrl+S. Empty string for no keyboard binding. THE DEFAULT IS A PLACEHOLDER "
            "-- the button worth having here is the one your fingers already know."
        ),
    )
    parser.add_argument(
        "--sort-pad",
        dest="inventory_sort_pad",
        default="lthumb",
        help=(
            "XInput button for --inventory-sort: a b x y lb rb back start lthumb rthumb dpad_up "
            "dpad_down dpad_left dpad_right. Empty string for no controller binding. DEFAULT: "
            "lthumb -- left stick click, which is where ELDEN RING puts Sort, and mirroring that "
            "is the entire point of the feature. Names are the Xbox ones because XInput is an "
            "Xbox API; on a DualShock, lthumb is L3 and y is Triangle."
        ),
    )
    parser.add_argument(
        "--item-warn",
        dest="item_warn",
        action="store_true",
        help=(
            "put a red badge in the bottom-left of any weapon icon whose stat requirements this "
            "character does not meet. OFF without this flag, matching the DLL, because the feature "
            "patches the frontend's layout builder and its cell bind and no run has yet put it on "
            "screen. It answers with the DETAIL PANE's check, which ignores grip -- two-handing "
            "halves a weapon's Strength requirement and the badge will not know."
        ),
    )
    parser.add_argument(
        "--seamless",
        dest="seamless",
        action="store_true",
        help=(
            "also load Seamless Co-op's DLL into the same process. NOTHING HERE SHIPS IT -- you "
            "install that mod yourself, from its own download, next to DarkSoulsII.exe, and this "
            "flag only writes the path into the config. Implies --no-offline: that feature fronts "
            "the socket imports, so a co-op mod under it would load, report success and never "
            "connect. Grep the log for `ds2-seamless:`."
        ),
    )
    parser.add_argument(
        "--seamless-dll",
        dest="seamless_dll",
        default=SEAMLESS_DEFAULT_DLL,
        metavar="PATH",
        help=(
            "where that DLL is, relative to the game directory "
            f"(default: {SEAMLESS_DEFAULT_DLL}, which is where its own launcher injects from)."
        ),
    )
    parser.add_argument(
        "--no-save-file-row",
        dest="menu_rows_no_save",
        action="store_true",
        help=(
            "put every menu row on EXCEPT `save-game-to-file`, so the game saves itself the way "
            "it normally does. That row registering is what installs `ds2-save-block`, which "
            "refuses every save the game makes for itself -- no autosave, nothing at a bonfire, "
            "nothing on the way out -- so a session played with it on loses progress unless the "
            "row is pressed by hand. Use this to actually play. The DLL's own defaults include "
            "that row, so dropping --all-menu-rows is not enough to get saving back."
        ),
    )
    parser.add_argument(
        "--launcher-dll",
        dest="launcher_dll",
        action="append",
        default=[],
        metavar="PATH",
        help=(
            "another mod's DLL to inject before the game runs an instruction, relative to the "
            "game directory or absolute. Repeatable, and injected in the order given. This is "
            "the only way into the process for a mod that cannot be loaded from inside it, and "
            "it makes the run go through this repo's own "
            f"{STAGED_LAUNCHER_NAME} rather than through `steam -applaunch` -- the same "
            "trade `--seamless` makes, and for the same reason. A named file that is not there "
            "refuses the launch instead of starting a game without it."
        ),
    )
    parser.add_argument(
        "--invasion-path",
        dest="invasion_path",
        action="store_true",
        help=(
            "TURN ON the overlay that draws a direction to every other player in your session. "
            "OFF by default, and the only feature here that detours a RENDERING function -- "
            "`IDXGISwapChain::Present`, in dxgi.dll rather than in the game image. It draws an "
            "arrow per player, not a walkable route: DARK SOULS II's navigation stack can be "
            "READ but not yet asked, see crates/ds2-invasion-path/src/navpath.rs. Grep the log "
            f"for `{INVASION_PATH_LOG_PREFIX}` -- `overlay:` says whether it could draw, "
            "`camera:` which camera it found, `roster:` what it saw."
        ),
    )
    parser.add_argument(
        "--invasion-path-key",
        dest="invasion_path_key",
        default="semicolon",
        help=(
            "key NAME that toggles the overlay, in `ds2-hotkey-config` spelling. DEFAULT: "
            "semicolon, chosen because it is clear of everything else this workspace polls."
        ),
    )
    parser.add_argument(
        "--invasion-path-markers",
        dest="invasion_path_marker_effect_id",
        type=int,
        default=0,
        metavar="EFFECT_ID",
        help=(
            "lay the game's OWN glowing stones along the route, one effect id per marker. 0 is "
            "off and is the default. 833 is the Prism Stone -- the item ELDEN RING renamed to "
            "Rainbow Stone -- and 833..=839 are its seven colours. This is the only setting here "
            "that makes the DLL change the game rather than draw over it: the stones are real "
            "effects, spawned by the engine from the game's own tick."
        ),
    )
    parser.add_argument(
        "--invasion-path-self-check",
        dest="invasion_path_npc_self_check",
        action="store_true",
        help=(
            "route to the nearest NPC instead of waiting for another player, and narrate every "
            "step in the log. THE ONLY WAY A SOLO RUN CAN EXERCISE ANY OF THIS: alone, the "
            "roster reads remotes=0, nothing is ever requested and every line is an install "
            "line. Pairs with --invasion-path-markers to also place the stones and sweep all "
            "seven colours. It narrates the trail; it does not hold it still -- the route is "
            "re-planned as you walk and torn down and re-laid when it moves, exactly as it "
            "would be for a real player. Grep for `self-check:`."
        ),
    )
    parser.add_argument(
        "--invasion-path-on",
        dest="invasion_path_start_enabled",
        action="store_true",
        help=(
            "start with the overlay already switched on. WHAT A TEST RUN WANTS: the roster read, "
            "the camera search and the draw all happen without anyone pressing anything, so a "
            "headless run produces the `roster:` and `camera:` lines on its own."
        ),
    )
    parser.add_argument(
        "--save-dir",
        dest="save_dir",
        default=DEFAULT_SAVE_DIR,
        metavar="DIR",
        help=(
            "play out of this folder instead of the game's own save directory, for the whole "
            "launch. The game opens its own container name inside it and reads and writes that "
            "file, so a character autoloaded from here saves back into here. Nothing is copied "
            "in either direction -- which is the difference from the `[save_redirect] path` key "
            "this replaces, whose copy the next launch overwrote and whose sessions therefore "
            "lost everything done in them. Takes a Linux path and converts it for the prefix. "
            f"DEFAULT: {DEFAULT_SAVE_DIR}, so an ordinary run already plays out of there and "
            "nothing writes the container Steam syncs; pass an empty string to turn the redirect "
            "off and use the prefix's own AppData again. The extension is not this flag's to "
            "choose -- the game opens whatever name is current, which is `.co2` under --seamless "
            "and `.sl2` otherwise. A folder that does not exist is refused by the DLL and named "
            "in the log; an existing empty one is a fresh start."
        ),
    )
    parser.add_argument(
        "--all-menu-rows",
        dest="menu_rows_all",
        action="store_true",
        help=(
            f"write `[{MENU_ROW_SECTION}] {KEY_MENU_ROW_ROWS}` with every row this table knows, "
            "instead of leaving the key commented out for the DLL's legacy defaults. All of them "
            "or none: a subset written from here once read as a DLL that had lost two rows. The "
            "reason to want it is that two features can only be reached through a row -- "
            "`load-character-from-file` and `save-game-to-file` -- and `ds2-save-block` installs "
            "only when the second one registers, which means THIS RUN DOES NOT SAVE BY ITSELF: no "
            "autosave, nothing on quit to menu, nothing at a bonfire. Only that row writes the "
            f"container. Grep the log for `{SAVE_BLOCK_LOG_PREFIX}`."
        ),
    )
    parser.add_argument(
        "--release-config",
        dest="release_config",
        action="store_true",
        help=(
            f"stage {RELEASE_CONFIG.relative_to(REPO_ROOT)} verbatim as the game's config, "
            "instead of writing one from this script's flags -- the game as a player who "
            "unpacked the release gets it. Every other config flag is ignored, and no slot is "
            "autoloaded unless that file asks for one."
        ),
    )
    parser.add_argument(
        "--input-harness",
        dest="input_harness",
        action="store_true",
        help=(
            "TURN ON the agent-driven input harness: it detours DARK SOULS II's three DLUID "
            "device polls (pad, mouse, keyboard) and, after each one runs, writes the fields "
            "the engine reads. OFF by default, because it is the only thing here that can stop "
            "YOUR input reaching the game. Drive it while the game runs by writing a sequence "
            f"number and a command to `{INPUT_HARNESS_COMMAND_FILE}` beside the exe -- "
            "`turn <degrees>` closes a loop on the camera's own yaw, `block <frames>` blanks "
            "every human input, `probe` reports which pad axis actually moves the camera. "
            "`turn` and `probe` measure against the camera --invasion-path draws through and "
            "refuse without it. Every command is frame-bounded; the block caps at ten minutes. "
            f"Grep the log for `{INPUT_HARNESS_LOG_PREFIX}`."
        ),
    )
    parser.add_argument(
        "--probe-site",
        choices=PROBE_SITES,
        default="m1",
        help=(
            "which function the detour goes on. `m1` (default) is the CONTROL -- a clean "
            "function Arxan never touched, where a surviving detour proves only that hooking "
            "works in this game. `redirected` is applySpEffect, whose five entry bytes ARE "
            "Arxan's redirect, and is the only site where survival is evidence about Arxan. "
            "The control is the default so a forgotten flag yields an uninformative run rather "
            "than a mislabelled one."
        ),
    )
    parser.add_argument(
        "--probe",
        choices=sorted(PROBE_ARMS),
        default="off",
        help=(
            "run the M1 Arxan-survival experiment. `neuter` runs it with dearxan having "
            "neutered Arxan first; `skip-neuter` leaves Arxan's 48 stubs live. BOTH ARMS ARE "
            "NEEDED -- neither one alone distinguishes 'dearxan saved the hook' from 'Arxan "
            "never touched it'. Default: off, which is the plain loader run."
        ),
    )
    parser.add_argument(
        "--observe",
        type=float,
        default=OBSERVE_SECONDS,
        metavar="SECONDS",
        help=(
            f"how long to watch the probe after it installs (default {OBSERVE_SECONDS:.0f}). "
            "Ignored when --probe is off. The verdict is explicitly scoped to this window."
        ),
    )
    parser.add_argument(
        "--crash-test",
        type=int,
        default=NO_FAULT_MS,
        metavar="MS",
        help=(
            "DELIBERATELY CRASH THE GAME this many milliseconds after the entry point, to prove "
            "the crash logger works. Writes `[crash_logging] fault_after_ms` and the loader "
            "raises 0xc0000005 on a dedicated thread. The game dying IS the expected result: it "
            "is the only way to exercise the fatal path -- top-level filter and minidump -- that "
            "a first-chance exception cannot reach. Default 0, which never faults."
        ),
    )
    args = parser.parse_args()

    if args.crash_test < 0:
        parser.error("--crash-test takes a non-negative number of milliseconds")

    # THE INTERLOCK, applied here rather than left to the DLL to refuse at runtime. `[offline]`
    # fronts the socket imports, so a co-op mod under it loads, reports success and never connects
    # -- and that is indistinguishable on screen from a co-op mod that is simply broken. Turning
    # the conflicting feature off and SAYING SO is the only outcome that cannot be misread later.
    if args.seamless and args.offline:
        args.offline = False
        print(
            f"[config] --seamless turned [{OFFLINE_SECTION}] off for this run: it fronts the "
            "socket imports a co-op mod needs. Pass --no-offline yourself to make that explicit."
        )

    if args.selftest:
        return selftest()

    if args.release_config:
        global release_config_text
        release_config_text = RELEASE_CONFIG.read_text(encoding="utf-8")
        print(f"[config] --release-config: staging {RELEASE_CONFIG} verbatim")
        # The release file decides the slot; reading one off the save here would be autoloading
        # on the harness's behalf in a run meant to show what a player gets.
        continue_slot = -1
    elif args.continue_slot is not None:
        continue_slot = args.continue_slot
    else:
        # No value given: read the slot off the redirected save, which is what this flag's help
        # has always said it does. The slot belongs to the save rather than to the account, so a
        # folder swapped in under `--save-dir` brings its own answer with it.
        continue_slot = first_loadable_slot(args.save_dir, args.seamless, args.seamless_dll)
        if continue_slot is None:
            continue_slot = -1
            if args.save_dir:
                print(
                    f"[continue] no loadable slot found in {args.save_dir} -- the character list "
                    "opens where the game left it. Name a slot with --continue-slot N to override."
                )
        else:
            print(
                f"[continue] autoloading slot {continue_slot} of the save in {args.save_dir} -- "
                "the first slot holding a named character, or the first fresh one if it holds "
                "none. Override with --continue-slot N."
            )

    if args.dry_run:
        return dry_run(
            args.probe,
            args.observe,
            args.crash_test,
            args.probe_site,
            args.intro_skip,
            args.dialog_skip,
            args.press_any_button,
            args.process_windows,
            args.hide_process_windows,
            args.title_animation,
            args.title_sequence_gate,
            args.title_settle,
            args.substate_floors,
            args.show_unavailable,
            args.boot_timeline,
            args.continue_record,
            continue_slot,
            args.continue_silence,
            args.continue_hide_menus,
            args.offline,
            args.block_sockets,
            args.inventory_sort,
            args.inventory_sort_key,
            args.inventory_sort_pad,
            args.item_warn,
            args.seamless,
            args.seamless_dll,
            args.invasion_path,
            args.invasion_path_key,
            args.invasion_path_start_enabled,
            args.invasion_path_marker_effect_id,
            args.invasion_path_npc_self_check,
            args.input_harness,
            windows_path(args.save_dir),
            args.menu_rows_all,
            args.menu_rows_no_save,
            tuple(args.launcher_dll),
        )
    return launch(
        args.probe,
        args.observe,
        args.crash_test,
        args.probe_site,
        args.intro_skip,
        args.dialog_skip,
        args.press_any_button,
        args.process_windows,
        args.hide_process_windows,
        args.title_animation,
        args.title_sequence_gate,
        args.title_settle,
        args.substate_floors,
        args.show_unavailable,
        args.boot_timeline,
        args.continue_record,
        continue_slot,
        args.continue_silence,
        args.continue_hide_menus,
        args.offline,
        args.block_sockets,
        args.inventory_sort,
        args.inventory_sort_key,
        args.inventory_sort_pad,
        args.item_warn,
        args.seamless,
        args.seamless_dll,
        args.invasion_path,
        args.invasion_path_key,
        args.invasion_path_start_enabled,
        args.invasion_path_marker_effect_id,
        args.invasion_path_npc_self_check,
        args.input_harness,
        windows_path(args.save_dir),
        args.menu_rows_all,
        args.menu_rows_no_save,
        tuple(args.launcher_dll),
    )


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)
