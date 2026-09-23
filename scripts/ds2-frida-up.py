#!/usr/bin/env python3
"""Bring up a Wine-side Frida server in DARK SOULS II's own prefix, and prove it can see the game.

# Why the server, and not `frida.attach()`

A plain `frida.attach()` on this target injects a Linux bootstrapper into a Windows process and it
segfaults: `frida.NotSupportedError: bootstrapper crashed with signal 11 at offset 0x0`. Measured
again on 2026-09-08 with Frida 17.17.0, so that half of the old finding still holds. The other half
did not: the game survived both attempts (`scripts/er-frida-attach-probe.py` reports the pid alive
before and after), which is why the standing rule against ever touching Frida here was retired.

Running `frida-server.exe` -- the Windows build -- inside the game's own prefix removes the
mismatch entirely. The server is a Windows process talking to a Windows process, so no Linux-side
bootstrapper is ever injected, and the game is reached as an ordinary remote device. Proven
2026-09-08: the server enumerated 15 prefix processes, found `DarkSoulsII.exe`, attached, resolved
`ersc.dll` at `0x180000000`, and installed an `Interceptor` on `ersc+0x25850`.

# What this buys

Seamless's session object cannot be found by scanning: the game holds 25,192 objects that match its
static shape and no `ersc.dll` global points at it. Frida answers it in one hook, because
`ersc+0x25850` opens `mov rdi, [rcx + 0x58]` -- so `rcx` at entry is the menu object and `rdi` is
the session, handed over by the game rather than guessed at.

    python3 scripts/ds2-frida-up.py            # start it, or report the one already running
    uv run --with frida python3 scripts/ds2-frida-up.py --status   # needs frida in the interpreter
    python3 scripts/ds2-frida-up.py --stop
    python3 scripts/ds2-frida-up.py --selftest # no game required

Then, from any script:

    dev = frida.get_device_manager().add_remote_device("127.0.0.1:27042")
    session = dev.attach(dev.enumerate_processes.__self__ and PID)
"""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

# The pipeline's sleepless waits: `wait_for_exit` blocks on a pidfd, so every wait below returns on
# the event -- a process dying -- rather than on a clock. See its module docstring.
import ds2_run_lib as er_run_lib

FRIDA_VERSION = "17.17.0"
DEFAULT_PORT = 27042
PORT = DEFAULT_PORT
STEAM_APP_ID = "335300"
# Where the downloaded server is cached between runs. Not in the repo: it is a 65 MB third-party
# binary, the same category as the deobfuscated game images.
CACHE = pathlib.Path(os.environ.get("DS2_FRIDA_CACHE", pathlib.Path.home() / ".cache" / "ds2-frida"))
# Where `scripts/er-run-branch.py` leaves a run's artifacts, one `br-*` directory per launch. Named
# here rather than spelled inline in `world_is_up` so the selftest can point the gate at a planted
# directory instead of writing into the user's own run history to test itself.
LOADER_LOG = pathlib.Path(
    "/home/banon/.local/share/Steam/steamapps/common/"
    "Dark Souls II Scholar of the First Sin/Game/ds2-loader.log"
)
DOWNLOAD = (
    f"https://github.com/frida/frida/releases/download/{FRIDA_VERSION}/"
    f"frida-server-{FRIDA_VERSION}-windows-x86_64.exe.xz"
)
# One attempt's worth of transfer, and the subprocess bound around it.
#
# `scripts/check-no-timeouts.py` caps every non-game subprocess at 30 seconds and a 65 MB download
# does not fit in one. It does not have to: the fetch below resumes (`curl -C -`), so the cap costs
# a reconnect rather than the download, and progress is measured in bytes on disk instead of in
# elapsed time. The previous 600-second bound was the opposite trade -- a stalled mirror held the
# terminal for ten minutes and then reported nothing but a timeout.
DOWNLOAD_ATTEMPT_SECONDS = 25
DOWNLOAD_SUBPROCESS_TIMEOUT_SECONDS = 30
# `-C -` is what makes the cap affordable: an attempt cut short by `--max-time` is resumed from the
# byte it reached, so the bound costs a reconnect rather than the whole transfer.
DOWNLOAD_CURL_FLAGS = ["-sSL", "--max-time", str(DOWNLOAD_ATTEMPT_SECONDS), "-C", "-"]
# Decompression is a local, bounded job: 65 MB of xz unpacks in a couple of seconds, so the cap here
# is a backstop against a corrupt archive, not a budget.
UNPACK_TIMEOUT_SECONDS = 30
# Enough attempts that a genuinely slow link finishes; an attempt that adds no bytes ends the loop
# immediately, so a dead mirror fails on the first one rather than on the fortieth.
MAX_DOWNLOAD_ATTEMPTS = 40
# The whole wait for the server to open its port, inside one agent-shell command.
SERVER_START_BUDGET_SECONDS = 28.0
# How long one slice blocks on the server's pidfd before the port is probed again.
SERVER_PROBE_SLICE_SECONDS = 1.0
# How long a killed server gets to actually go. One that is still there after this is stuck in the
# kernel, which is a different problem from a stale server.
SERVER_EXIT_WAIT_SECONDS = 5.0

# The `WorldChrMan` walk this gate's second witness is -- the address constants, the
# `/proc/<pid>/mem` read, the three-valued verdict and the planted-chain coverage for all of it --
# lives in `er_run_lib`. `scripts/er-frida-when-world.py` gates on the same walk, and an rva of that
# provenance written out in two files is an rva that goes stale in one of them without anyone
# noticing: the failure it produces reads as an empty world forever rather than as a wrong address.


def steam_root() -> pathlib.Path:
    return pathlib.Path(
        os.environ.get("ER_STEAM_DIR", pathlib.Path.home() / ".local/share/Steam")
    )


def prefix() -> pathlib.Path:
    return steam_root() / "steamapps/compatdata" / STEAM_APP_ID / "pfx"


def game_pid() -> int | None:
    """The Linux pid of `DarkSoulsII.exe`, or `None`.

    A name of its own rather than a bare call, because the selftest replaces it to drive the gate
    without a game. The lookup itself is `er_run_lib`'s, shared with everything else that has to
    find the game process by `comm`.
    """
    return er_run_lib.game_pid()


def linux_pid_is_game(pid: int) -> bool:
    try:
        return pathlib.Path(f"/proc/{pid}/comm").read_text(encoding="utf-8").strip() == "DarkSoulsII.exe"
    except OSError:
        return False


def container_prefix(pid: int) -> list[str]:
    """The `nsenter` prefix that puts a command inside the game's container, or `[]`.

    # Why this is required rather than optional

    Proton runs the game inside pressure-vessel, so `DarkSoulsII.exe` lives in its own mount and user
    namespaces -- measured 2026-09-08: game `mnt:[4026533261]` / `user:[4026533260]` against the
    host's `mnt:[4026531832]` / `user:[4026531837]`. A wineserver is per-namespace, so a
    `frida-server.exe` started beside the container is talking to a different wineserver and sees
    none of the game's processes. It does not fail: it accepts the connection and then never
    answers `enumerate_processes`, which is how a watcher sat silent through an entire run.

    The pid and network namespaces are shared (`pid:[4026531836]`, `net:[4026531833]` on both
    sides), which is what makes this workable: the server can be reached on host localhost once it
    is running in the right mount namespace, and no privilege is needed to enter a namespace this
    user already owns.
    """
    return [
        "nsenter",
        "-t",
        str(pid),
        "-m",
        "-U",
        "--preserve-credentials",
        "--",
    ]


def wine_binary() -> pathlib.Path | None:
    """Proton's own `wine`, so the server runs against the same build the game does."""
    for proton in sorted((steam_root() / "steamapps/common").glob("Proton*")):
        candidate = proton / "files/bin/wine"
        if candidate.is_file():
            return candidate
    return None


def listening(port: int = PORT) -> bool:
    with socket.socket() as probe:
        probe.settimeout(2)
        try:
            probe.connect(("127.0.0.1", port))
            return True
        except OSError:
            return False


def declared_size(url: str) -> int | None:
    """The `Content-Length` the server reports after redirects, or `None`.

    This is the completion predicate for [`fetch_resumable`]. Curl's exit code cannot serve as one:
    a transfer cut short by `--max-time` leaves a partial file behind, and a resumed request that
    the server answers with `416 Range Not Satisfiable` succeeds while appending an error page.
    Bytes on disk against a declared length is the only reading that says the file is whole.
    """
    result = subprocess.run(
        ["curl", "-sSLI", "--max-time", str(DOWNLOAD_ATTEMPT_SECONDS), url],
        capture_output=True,
        check=False,
        timeout=DOWNLOAD_SUBPROCESS_TIMEOUT_SECONDS,
    )
    size = None
    for line in result.stdout.decode("utf-8", errors="replace").splitlines():
        name, _, value = line.partition(":")
        # Last one wins: a redirect chain prints one header block per hop, and the file's length is
        # the one the final hop declares.
        if name.strip().lower() == "content-length" and value.strip().isdigit():
            size = int(value.strip())
    return size


def fetch_resumable(url: str, destination: pathlib.Path) -> None:
    """Fetch `url` into `destination` in bounded attempts, resuming rather than restarting.

    Each attempt is its own subprocess under the 30-second cap, and the loop advances on an
    observation -- the file grew -- rather than on a timer. An attempt that adds nothing stops it at
    once with what was actually seen, so an unreachable mirror is a fast, legible failure instead of
    a ten-minute hang.
    """
    expected = declared_size(url)
    for attempt in range(1, MAX_DOWNLOAD_ATTEMPTS + 1):
        have = destination.stat().st_size if destination.exists() else 0
        if expected is not None and have >= expected:
            return
        result = subprocess.run(
            ["curl", *DOWNLOAD_CURL_FLAGS, "-o", str(destination), url],
            check=False,
            timeout=DOWNLOAD_SUBPROCESS_TIMEOUT_SECONDS,
        )
        now = destination.stat().st_size if destination.exists() else 0
        if expected is None and result.returncode == 0 and now > 0:
            return
        if now <= have:
            raise SystemExit(
                f"download made no progress on attempt {attempt}: {now} bytes of "
                f"{expected if expected is not None else 'an undeclared total'} from {url} "
                f"(curl exit {result.returncode}). Fetch it by hand into {destination.parent} if "
                "the link is the problem."
            )
        print(f"  {now}/{expected if expected is not None else '?'} bytes", flush=True)
    raise SystemExit(
        f"download did not finish in {MAX_DOWNLOAD_ATTEMPTS} attempts; {destination} holds "
        f"{destination.stat().st_size if destination.exists() else 0} bytes and will be resumed by "
        "the next run"
    )


def ensure_binary() -> pathlib.Path:
    CACHE.mkdir(parents=True, exist_ok=True)
    server = CACHE / f"frida-server-{FRIDA_VERSION}.exe"
    if server.is_file():
        return server
    packed = CACHE / "frida-server.exe.xz"
    print(f"fetching {DOWNLOAD}", flush=True)
    fetch_resumable(DOWNLOAD, packed)
    subprocess.run(["unxz", "-f", str(packed)], check=True, timeout=UNPACK_TIMEOUT_SECONDS)
    shutil.move(str(CACHE / "frida-server.exe"), str(server))
    return server


def clear_staged_agent() -> bool:
    """Delete the agent DLL a dead server left staged in the prefix, so the next attach can stage.

    Measured 2026-09-16, and it cost three attaches to recognise. `frida-server.exe` unpacks its
    injector payload to `%TEMP%/re.frida.server/x86_64/frida-agent.dll` and, on this Wine target,
    refuses to overwrite one a previous server left behind:

        frida.PermissionDeniedError: error opening file
        "C:\\users\\steamuser\\AppData\\Local\\Temp\\re.frida.server\\x86_64\\frida-agent.dll":
        File exists

    Deleting the tree by hand is not enough either -- a server already running does not re-stage,
    and the next attach fails the other way round with `unable to find DLL at ...`. Both halves
    have to happen together, which is why this lives inside the restart rather than beside it.
    """
    staged = (
        prefix()
        / "drive_c/users/steamuser/AppData/Local/Temp/re.frida.server"
    )
    if not staged.exists():
        return False
    shutil.rmtree(staged, ignore_errors=True)
    return not staged.exists()


def world_is_up() -> tuple[bool, str]:
    """Whether the game has a player in a world yet.

    The launcher returns as soon as the DLL logs that it loaded, which is a minute or more before
    a world exists, and a frida-server started into that window puts an injector into a process
    still building its own address space.

    THE WITNESS IS THE ROSTER LINE, and it is the right one because of what it costs the game to
    produce. `ds2-invasion-path` writes `roster: characters=N ...` only after it has walked
    `CharacterManager` from a live `GameManagerImp` AND resolved the local player -- so the line
    cannot appear at the title screen, during a load, or while the overlay is off. A file read,
    no hook of this script's own, and it needs no server to answer.

    `--allow-early` is still there for the one case this gate is wrong for: measuring the boot
    itself, where the whole point is to be attached before a world exists.
    """
    # No game is the clearest not-ready there is, and it has to be checked before the log: a
    # previous run's log keeps its roster lines long after that process died, so reading it first
    # would let the gate pass with nothing running at all.
    pid = game_pid()
    if pid is None:
        return False, "no DarkSoulsII.exe is running"
    if not LOADER_LOG.is_file():
        return False, f"no loader log at {LOADER_LOG}"
    try:
        text = LOADER_LOG.read_text(encoding="utf-8", errors="replace")
    except OSError as error:
        return False, f"could not read {LOADER_LOG}: {error}"
    for line in reversed(text.splitlines()):
        if "invasion-path: roster:" in line:
            return True, line.strip()
    return False, (
        "the loader log has no `roster:` line yet, so the overlay has not seen a live "
        "CharacterManager -- the game is at the title screen or still loading. Launch with "
        "--invasion-path --invasion-path-on, or pass --allow-early if the boot IS the measurement."
    )
def start(
    force: bool = False,
    allow_early: bool = False,
    role: str | None = None,
    linux_pid: int | None = None,
    port: int = PORT,
    log_path: pathlib.Path | None = None,
    pidfile: pathlib.Path | None = None,
) -> int:
    # Refuse to start while the game is still booting.
    #
    # 2026-09-16: a server was started seconds after a launch, the drive that followed reported
    # "input is not reaching the game", and the process was gone by the next check -- user's
    # reading, "you definitely crashed the game by attaching frida too early". Injecting into a
    # process that is still mapping its own modules is the one attach this repo never had a reason
    # to make: every question these agents ask is about a world that does not exist yet.
    #
    # `--allow-early` exists for the boot itself being the subject.
    if not allow_early:
        ready, detail = world_is_up()
        if not ready:
            print(
                f"REFUSING to start frida-server: the game is not in a world yet ({detail}). "
                "Attaching during boot has killed the process. Wait for the world, or pass "
                "--allow-early when the boot itself is what you are measuring.",
                file=sys.stderr,
            )
            return 4
    # An open port is not proof of a usable server. Measured 2026-09-08: a server started before
    # `scripts/er-teardown.py` killed the prefix keeps its listening socket, accepts the connection,
    # and then hangs forever inside `enumerate_processes` -- so a watcher started against it sits
    # mute rather than failing, which is exactly how a run reached the player with nothing attached.
    # The server's view is per-wineserver, so a teardown invalidates it and it has to be replaced.
    if linux_pid is not None and not linux_pid_is_game(linux_pid):
        print(f"linux pid {linux_pid} is not DarkSoulsII.exe", file=sys.stderr)
        return 1
    if force:
        # `--force` has to stop the old server, not merely skip the reuse check. Without this the
        # replacement starts, fails with `Unable to start: Error binding to address 127.0.0.1:27042`
        # because the previous one still holds the port, and dies -- leaving the stale server
        # answering. Measured 2026-09-08: after a relaunch the old server was still alive in the
        # previous container's namespace (mnt:[4026533261]) while the game had moved to a new one
        # (mnt:[4026533335]), so every enumerate hung and `--force` appeared to do nothing.
        stop(pidfile=pidfile, port=port)
        if clear_staged_agent():
            print("cleared the agent DLL a previous server left staged in the prefix")
    elif listening(port):
        if server_sees_the_prefix(port):
            print(f"frida-server already listening on 127.0.0.1:{port} and answering")
            return 0
        print("frida-server is listening but not answering; replacing it", flush=True)
        stop(pidfile=pidfile, port=port)
        if clear_staged_agent():
            print("cleared the agent DLL a previous server left staged in the prefix")
    wine = wine_binary()
    if wine is None:
        print("no Proton wine binary found under Steam", file=sys.stderr)
        return 1
    if not prefix().is_dir():
        print(f"no wine prefix at {prefix()}", file=sys.stderr)
        return 1
    server = ensure_binary()
    staged = prefix() / "drive_c" / server.name
    shutil.copyfile(server, staged)
    env = dict(os.environ)
    env["WINEPREFIX"] = str(prefix())
    # The server is a console app that needs neither; suppressing them keeps the log readable.
    env["WINEDLLOVERRIDES"] = "mscoree=d;mshtml=d"
    if log_path is None:
        suffix = "frida-server.log" if role is None else f"frida-server-{role}.log"
        log_path = CACHE / suffix
    pid = linux_pid if linux_pid is not None else game_pid()
    command = [str(wine), str(staged)]
    if port != DEFAULT_PORT:
        command.extend(["-l", f"127.0.0.1:{port}"])
    if pid is not None:
        command = container_prefix(pid) + command
        print(f"starting frida-server inside the game's container (pid {pid})", flush=True)
    else:
        print("no DarkSoulsII.exe yet; starting frida-server on the host namespace", flush=True)
    log_path.parent.mkdir(parents=True, exist_ok=True)
    with open(log_path, "wb") as handle:
        server = subprocess.Popen(
            command,
            env=env,
            stdout=handle,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
    if pidfile is not None:
        pidfile.parent.mkdir(parents=True, exist_ok=True)
        pidfile.write_text(
            json.dumps(
                {
                    "role": role,
                    "linux_pid": pid,
                    "starter_pid": server.pid,
                    "port": port,
                    "log": str(log_path),
                },
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )
    if wait_for_port(server, port):
        print(f"frida-server up on 127.0.0.1:{port} (log {log_path})")
        return 0
    print(f"frida-server did not open {port}; see {log_path}", file=sys.stderr)
    return 1


def wait_for_port(server: subprocess.Popen, port: int = PORT) -> bool:
    """Wait for the server to open `PORT`, or to die trying.

    Readiness is the connect probe, because a listening socket is the only thing that proves the
    server is up. What this does not do is pace the probes with a delay: between them it blocks on
    the server's own pidfd, so the documented failure -- an immediate exit with `Error binding to
    address 127.0.0.1:27042` because a previous server still holds the port -- ends the wait the
    instant it happens instead of thirty seconds later.
    """
    deadline = time.monotonic() + SERVER_START_BUDGET_SECONDS
    while True:
        if listening(port):
            return True
        if server.poll() is not None:
            print(
                f"frida-server exited with {server.returncode} before opening {port}",
                file=sys.stderr,
            )
            return False
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            return False
        er_run_lib.wait_for_exit(server.pid, min(SERVER_PROBE_SLICE_SECONDS, remaining))


def server_sees_the_prefix(port: int = PORT, timeout_seconds: float = 6.0) -> bool:
    """Whether the server can still enumerate its prefix, within a bound.

    `enumerate_processes` has no timeout of its own and blocks indefinitely against a server whose
    wineserver is gone, so the call is made on a daemon thread and abandoned if it does not answer.
    Abandoning it is safe: the process is about to be replaced either way.
    """
    import threading

    answered: list[bool] = []

    def ask() -> None:
        try:
            frida = import_frida()
            if frida is None:
                answered.append(False)
                return
            dev = frida.get_device_manager().add_remote_device(f"127.0.0.1:{port}")
            dev.enumerate_processes()
            answered.append(True)
        except Exception:
            answered.append(False)

    worker = threading.Thread(target=ask, daemon=True)
    worker.start()
    worker.join(timeout_seconds)
    return bool(answered) and answered[0]



def import_frida():
    """Import the real frida package, or say plainly that it is not installed here.

    `scripts/frida/` holds the agent `.js` files and has no `__init__.py`, so under a bare
    `python3` it is picked up as a namespace package for the name `frida` and the import
    succeeds with an empty module. Every later attribute access then fails as
    `module 'frida' has no attribute 'get_device_manager'`, which reads as an api change
    rather than as a missing dependency. Returns `None` when frida is unavailable.
    """
    try:
        import frida
    except ImportError:
        return None
    if not hasattr(frida, "get_device_manager"):
        return None
    return frida


def status(port: int = PORT) -> int:
    up = listening(port)
    print(f"127.0.0.1:{port} {'OPEN' if up else 'closed'}")
    if not up:
        return 1
    frida = import_frida()
    if frida is None:
        print("frida python not importable here; run under `uv run --with frida`")
        return 0
    device = frida.get_device_manager().add_remote_device(f"127.0.0.1:{port}")
    processes = device.enumerate_processes()
    print(f"{len(processes)} process(es) visible in the prefix")
    for process in processes:
        if "darksoulsii" in process.name.lower():
            print(f"  DarkSoulsII.exe is windows pid {process.pid}")
    return 0


def frida_server_pids() -> list[int]:
    """Every frida-server, matched on `comm` rather than the full command line.

    `pkill -f frida-server` matches any process whose argv mentions the name -- including the shell
    that invoked this script, which killed a session on 2026-09-08. Reading `/proc/*/comm` and
    signalling by pid cannot make that mistake.
    """
    pids = []
    for entry in pathlib.Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        pid = int(entry.name)
        if pid == os.getpid():
            continue
        try:
            if (entry / "comm").read_text(encoding="utf-8").strip().startswith("frida-server"):
                pids.append(pid)
        except OSError:
            continue
    return pids


def stop(pidfile: pathlib.Path | None = None, port: int = PORT) -> int:
    """Stop either one recorded server starter or the legacy global server set.

    The wait is a pidfd, not a delay. What a caller needs from this function is that the port has
    been released, and the event that releases it is the holder dying -- so `--force` starting its
    replacement is now gated on the previous server being gone rather than on a second having
    passed. That distinction was measured on 2026-09-08: a replacement started too early fails with
    `Error binding to address 127.0.0.1:27042` and dies, leaving the stale server answering, which
    is how `--force` came to look like it did nothing at all.
    """
    killed = []
    if pidfile is not None and pidfile.exists():
        try:
            record = json.loads(pidfile.read_text(encoding="utf-8"))
            starter_pid = int(record["starter_pid"])
            os.killpg(starter_pid, signal.SIGKILL)
            killed.append(starter_pid)
        except (KeyError, ValueError, OSError, json.JSONDecodeError):
            killed = []
    else:
        for pid in frida_server_pids():
            try:
                os.kill(pid, signal.SIGKILL)
                killed.append(pid)
            except OSError:
                continue
    for pid in killed:
        er_run_lib.wait_for_exit(pid, SERVER_EXIT_WAIT_SECONDS)
    print(f"stopped {killed or 'nothing'}; 127.0.0.1:{port} {'still OPEN' if listening(port) else 'closed'}")
    return 0


def world_gate_selftest() -> list[tuple[str, bool]]:
    """Prove the gate believes the roster line, and only when a game is running.

    Ported from `er-mods-rs`, where the two witnesses were a telemetry file and a `/proc` read.
    DARK SOULS II has one witness -- the overlay's own `roster:` line -- so what is left to prove
    is the ORDER: no process beats any log content, because a previous run's log outlives the
    process that wrote it and reading it first let the gate pass with nothing running at all.
    """
    results: list[tuple[str, bool]] = []
    globals_ = globals()
    saved = {name: globals_[name] for name in ("LOADER_LOG", "game_pid")}
    try:
        with tempfile.TemporaryDirectory() as tmp:
            log = pathlib.Path(tmp) / "ds2-loader.log"
            globals_["LOADER_LOG"] = log

            globals_["game_pid"] = lambda: None
            log.write_text(
                "ds2-invasion-path: roster: characters=6 players=1 remotes=0\n", encoding="utf-8"
            )
            ready, detail = world_is_up()
            results.append(
                ("a dead game beats a log full of roster lines", not ready and "running" in detail)
            )

            globals_["game_pid"] = lambda: 1
            ready, detail = world_is_up()
            results.append(("a roster line with a live game passes", ready and "roster" in detail))

            log.write_text("ds2-loader: attach awaiting-arxan-callback\n", encoding="utf-8")
            ready, detail = world_is_up()
            results.append(
                ("a log with no roster line refuses", not ready and "roster" in detail)
            )

            log.unlink()
            ready, detail = world_is_up()
            results.append(("a missing log refuses", not ready and "no loader log" in detail))
    finally:
        for name, value in saved.items():
            globals_[name] = value
    return results
def selftest() -> int:
    source = pathlib.Path(__file__).read_text(encoding="utf-8")
    # The world walk moved into `er_run_lib`, so the structural checks on it have to follow it
    # there. A check that only ever reads this file would go green on the move itself while the
    # thing it guards sat unguarded in the other one.
    shared = pathlib.Path(er_run_lib.__file__).read_text(encoding="utf-8")
    # The `--force` branch, read structurally rather than by quoting the comment inside it. The
    # quoted form asserted on prose and broke the day the prose was reworded, which is the one thing
    # a check on a file that is already correct must not do.
    force_branch = source.split("\n    if force:", 1)[-1].split("elif listening():", 1)[0]
    # What `world_is_up` does once the telemetry witness has nothing to say, read the same
    # structural way rather than by quoting prose that a rewording would break.
    world_gate_tail = source.split("\ndef world_is_up(", 1)[-1].split("\ndef ", 1)[0]
    checks = [
        ("a Proton wine binary is resolvable", wine_binary() is not None),
        ("the game's wine prefix exists", prefix().is_dir()),
        ("the download url names the pinned version", FRIDA_VERSION in DOWNLOAD),
        ("the cache directory is user-owned, not a repo path", "ds2-mods-rs" not in str(CACHE)),
        ("--force stops the old server before starting a new one", "stop(" in force_branch),
        (
            "--force clears the agent DLL a dead server left staged",
            "clear_staged_agent()" in force_branch,
        ),
        (
            "the server is stopped by comm, never a broad pkill -f pattern",
            # Built from pieces so the check does not match its own source text -- a literal here
            # made the assertion fail against a file that was already correct.
            ("sub" + "process.run([\"pk" + "ill\"")
            not in pathlib.Path(__file__).read_text(encoding="utf-8"),
        ),
        (
            "the server is started inside the game's container when one exists",
            "container_prefix(" in pathlib.Path(__file__).read_text(encoding="utf-8"),
        ),
        (
            "an open port alone is not treated as a working server",
            "server_sees_the_prefix()" in pathlib.Path(__file__).read_text(encoding="utf-8"),
        ),
        (
            "every wait is an event wait, never a delay",
            # Built from pieces so the assertion does not match its own source text.
            ("ti" + "me.sl" + "eep(")
            not in pathlib.Path(__file__).read_text(encoding="utf-8"),
        ),
        (
            "a killed server is waited out on its pidfd",
            "er_run_lib.wait_for_exit(pid, SERVER_EXIT_WAIT_SECONDS)"
            in pathlib.Path(__file__).read_text(encoding="utf-8"),
        ),
        (
            "the download is resumable, so each attempt fits the 30-second subprocess cap",
            "-C" in DOWNLOAD_CURL_FLAGS
            and DOWNLOAD_SUBPROCESS_TIMEOUT_SECONDS <= 30
            and DOWNLOAD_ATTEMPT_SECONDS < DOWNLOAD_SUBPROCESS_TIMEOUT_SECONDS,
        ),
        (
            "the fetch completes on a size, not on curl's exit code",
            "declared_size(" in pathlib.Path(__file__).read_text(encoding="utf-8"),
        ),
        (
            "the world gate never passes on how long the process has been alive",
            # Both files, because the walk lives in one and the gate in the other. Built from
            # pieces so the assertion does not match its own source text. The age fallback that
            # used to sit in `world_is_up` passed a process that had been up five minutes, which a
            # crashed modal satisfies as readily as a world does.
            ("/proc/" + "uptime") not in source + shared
            and ("BOOT_" + "SETTLE_SECONDS") not in source + shared,
        ),
        (
            "the gate reads the overlay's own roster line, which no title screen can produce",
            "invasion-path: roster:" in world_gate_tail,
        ),
        (
            "the live read opens the game's memory read-only, and injects nothing",
            'open(f"/proc/{pid}/mem", "rb", 0)' in shared,
        ),
        (
            "the walk and its address constants have one owner, not a copy per script",
            # The needle is an assignment, not the name: this file still reads the constant
            # through `er_run_lib`, and what must not come back is a second definition of it.
            # Assembled from pieces, or the needle would be in its own haystack.
            ("WORLD_CHR_MAN" + "_GLOBAL_RVA = ") not in source
            and ("WORLD_CHR_MAN" + "_GLOBAL_RVA = 0x3D69FF8") in shared,
        ),
    ]
    failed = 0
    for label, ok in checks:
        print(f"  {'ok  ' if ok else 'FAIL'}  {label}")
        failed += 0 if ok else 1

    print("\n  the WorldChrMan witness, address and read path:")
    hop, hop_detail = er_run_lib.recorded_1162_to_1170_hop()
    if hop is None:
        print(f"  ....  the 1.16.2 to 1.17.0 hop went unchecked -- {hop_detail}")
    else:
        print(f"  {'ok  ' if hop else 'FAIL'}  the map still carries the first hop -- {hop_detail}")
        failed += 0 if hop else 1
    print(
        f"  ....  the second hop is `.data`, which 1.17.1 did not move: "
        f"0x{er_run_lib.GAME_PREFERRED_IMAGE_BASE + er_run_lib.WORLD_CHR_MAN_GLOBAL_RVA:x} "
        f"+0x{er_run_lib.WORLD_CHR_MAN_MAIN_PLAYER_OFFSET:x}"
    )
    for label, ok in er_run_lib.world_read_selftest():
        print(f"  {'ok  ' if ok else 'FAIL'}  {label}")
        failed += 0 if ok else 1
    for label, ok in world_gate_selftest():
        print(f"  {'ok  ' if ok else 'FAIL'}  {label}")
        failed += 0 if ok else 1

    pid = game_pid()
    if pid is None:
        print(
            "  ....  no DarkSoulsII.exe is running, so the live path went unexercised: the walk "
            "above ran against a planted chain in a child process, which proves the arithmetic "
            "and the read but not that these offsets still name a live WorldChrMan"
        )
    else:
        verdict, detail = er_run_lib.player_in_a_world(pid)
        reading = {True: "a player is in a world", False: "no player", None: "cannot tell"}[verdict]
        print(f"  ....  live read of pid {pid}: {reading} -- {detail}")

    print("selftest: PASS" if not failed else f"selftest: {failed} check(s) failed")
    return 0 if not failed else 1


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--status", action="store_true")
    parser.add_argument("--stop", action="store_true")
    parser.add_argument("--selftest", action="store_true")
    parser.add_argument("--force", action="store_true", help="replace a running server outright")
    parser.add_argument("--role", help="Role label for logs and pidfile records, e.g. host or peer")
    parser.add_argument("--linux-pid", type=int, help="Linux pid of the target DarkSoulsII.exe")
    parser.add_argument("--port", type=int, default=PORT, help="TCP port for frida-server")
    parser.add_argument("--log", type=pathlib.Path, help="Role-specific frida-server log path")
    parser.add_argument("--pidfile", type=pathlib.Path, help="Role-specific starter pid record")
    parser.add_argument(
        "--allow-early",
        action="store_true",
        help="start even though no world exists yet. Only for measuring the boot itself: "
        "attaching during boot has killed the game.",
    )
    args = parser.parse_args()
    if args.selftest:
        return selftest()
    if args.status:
        return status(port=args.port)
    if args.stop:
        return stop(pidfile=args.pidfile, port=args.port)
    return start(
        force=args.force,
        allow_early=args.allow_early,
        role=args.role,
        linux_pid=args.linux_pid,
        port=args.port,
        log_path=args.log,
        pidfile=args.pidfile,
    )


if __name__ == "__main__":
    raise SystemExit(main())
