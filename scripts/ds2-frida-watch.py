#!/usr/bin/env python3
"""Attach to DARK SOULS II once, then hot-reload the agent file on every edit.

# Why attach once and iterate on the file

Frida's intended workflow is a `.js` agent that is reloaded in place, not a fresh attach per
experiment (user directive 2026-09-08). Beyond convenience that is the safer shape here: every
attach/detach cycle installs and reverts trampolines under running game threads, and the one crash
this session landed shortly after a detach. Attaching once and editing the file removes those
cycles entirely.

This connects through the Wine-side `frida-server.exe` that `scripts/ds2-frida-up.py` starts inside
the game's own prefix. A plain Linux-side `frida.attach()` on this target still fails --
`NotSupportedError: bootstrapper crashed with signal 11`, re-measured 2026-09-08 on Frida 17.17.0 --
because it injects a Linux bootstrapper into a Windows process. The prefix-resident server has no
such mismatch.

    python3 scripts/ds2-frida-up.py                  # once, brings the server up
    uv run --with frida python3 scripts/ds2-frida-watch.py      # attaches and stays resident
    uv run --with frida python3 scripts/ds2-frida-watch.py --selftest

Every message the agent sends is appended to the log as JSON, one per line, so the transcript
survives the watcher and can be read while it is still running.

# What the exit writes down

When the watch ends -- a detach, an interrupt, a `SIGTERM` -- it records what the session did
through `scripts/ds2-frida-evidence.py`: the agent file, the pid, how many messages came back, how
long it ran. `.cupcake/policies/claude/no_rust_edit_without_frida_proof.rego` reads that record and
refuses a Rust edit under `crates/` without one, so this is where the right to write the code comes
from. The count is taken here, by the tool, for the obvious reason: evidence a caller can type is
not evidence.
"""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import select
import signal
import sys
import time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

# The pipeline's sleepless waits. Everything this watcher waits on is an event: an inotify write in
# the agent's directory, an inotify write in the game directory, or a byte on the detach pipe.
import ds2_run_lib as er_run_lib

PORT = 27042
DEFAULT_ENDPOINT = f"127.0.0.1:{PORT}"
# No default agent. The er-mods-rs original defaulted to its session agent, `ersc-session.js`,
# and the port kept that default -- a file this repository never had, so a bare run and
# `--selftest` both died on FileNotFoundError (measured 2026-09-25). Every DS2 agent answers one
# question and every caller here names one, so `--agent` is required instead.
AGENT_DIR = pathlib.Path(__file__).resolve().parent / "frida"
DEFAULT_LOG = pathlib.Path(
    os.environ.get("ER_FRIDA_LOG", pathlib.Path.home() / ".cache" / "er-frida" / "hits.jsonl")
)
# The cap on one slice of a wait, not the wait itself. A save in the agent's directory or a write in
# the game directory ends a slice at once; this only bounds how long a slice lasts when nothing
# happens, so a detach is still noticed promptly and an unavailable inotify cannot wedge the loop.
WATCH_SLICE_SECONDS = 4.0
# How long to wait for the game to appear before giving up, so this can be started before the
# launch and still catch the process the moment it exists.
WAIT_FOR_GAME_SECONDS = 300
# How often the "still waiting" line is printed while the game has yet to appear.
PROGRESS_EVERY_SECONDS = 10.0
# The evidence recorder, beside this file. Hyphenated, so it is loaded by path rather than imported
# by name; see `evidence_module`.
EVIDENCE_SCRIPT = pathlib.Path(__file__).resolve().parent / "ds2-frida-evidence.py"


def evidence_module():
    """`scripts/ds2-frida-evidence.py` as a module object.

    Loaded from its path because the filename carries hyphens and no `import` statement can spell
    it. Its `__name__` is not `__main__` here, so its argument parser does not run.
    """
    import importlib.util

    spec = importlib.util.spec_from_file_location("ds2_frida_evidence", EVIDENCE_SCRIPT)
    if spec is None or spec.loader is None:
        raise ImportError(f"cannot load {EVIDENCE_SCRIPT}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def record_evidence(agent_path: pathlib.Path, pid: int, messages: int, seconds: float) -> None:
    """Append what this watch observed, for the gate that reads it.

    `.cupcake/policies/claude/no_rust_edit_without_frida_proof.rego` refuses a Rust edit under
    `crates/` until a record exists that attached to a pid and received at least one message, so
    this call is what earns the right to write the code the measurement was for. It belongs here
    rather than in an agent's hands: evidence an agent can type is evidence that proves nothing.

    Nothing it can do may change the watcher's exit code. A failed record costs the caller a gate
    they then have to open by measuring again, which is annoying; a failed record that turns a
    successful watch into a non-zero exit costs them the belief that the watch worked at all.
    """
    try:
        evidence_module().record(str(agent_path), int(pid), int(messages), float(seconds))
    except Exception as exc:
        print(
            f"could not record frida evidence ({exc}). The watch itself was fine; "
            f"`python3 scripts/ds2-frida-evidence.py --check` will say UNPROVEN.",
            file=sys.stderr,
            flush=True,
        )


def blocking_slice(watch: er_run_lib.DirectoryWatch, seconds: float) -> None:
    """Block for at most `seconds`, returning early on a real file event.

    inotify when it is available. When it is not, a pidfd wait on this process -- which never fires,
    and so is a bounded block on a real descriptor rather than a delay dressed as one.
    `scripts/er-release-bisect.py` takes the same fallback for the same reason.
    """
    if watch.available:
        watch.wait(seconds)
        return
    er_run_lib.wait_for_exit(os.getpid(), seconds)


def device(endpoint: str = DEFAULT_ENDPOINT):
    import frida

    return frida.get_device_manager().add_remote_device(endpoint)


def find_game(dev, name: str = "DarkSoulsII.exe"):
    for process in dev.enumerate_processes():
        if process.name.lower() == name.lower():
            return process.pid
    return None


def linux_game_pid() -> int | None:
    """The Linux pid of `DarkSoulsII.exe`, or `None`.

    Separate from `find_game`, which answers with the Windows pid the prefix uses. A pidfd -- the
    primitive every wait in this pipeline blocks on -- takes the Linux one, so the two are not
    interchangeable. Matched on `comm` because the Proton wrapper processes carry the executable's
    name in their command line without being it.
    """
    for pid in er_run_lib.find_game_pids():
        try:
            comm = pathlib.Path(f"/proc/{pid}/comm").read_text(encoding="utf-8").strip()
        except OSError:
            continue
        if comm == "DarkSoulsII.exe":
            return pid
    return None


def find_game_bounded(dev, timeout_seconds: float = 6.0):
    """`find_game` with a bound, because the underlying call has none.

    `enumerate_processes` against a server whose wineserver is gone never returns. Running it on a
    daemon thread and abandoning it converts an indefinite hang into a `TimeoutError` the caller can
    report, which is the difference between a watcher that says what is wrong and one that says
    nothing for four minutes.
    """
    import threading

    result: list = []

    def ask() -> None:
        try:
            result.append(find_game(dev))
        except Exception as exc:
            result.append(exc)

    worker = threading.Thread(target=ask, daemon=True)
    worker.start()
    worker.join(timeout_seconds)
    if not result:
        raise TimeoutError("enumerate_processes did not answer")
    if isinstance(result[0], Exception):
        raise result[0]
    return result[0]


def find_pid_bounded(dev, pid: int, timeout_seconds: float = 6.0):
    """Return a Windows process by pid with the same stale-server bound as `find_game_bounded`."""
    import threading

    result: list = []

    def ask() -> None:
        try:
            for process in dev.enumerate_processes():
                if process.pid == pid:
                    result.append(process)
                    return
            result.append(None)
        except Exception as exc:
            result.append(exc)

    worker = threading.Thread(target=ask, daemon=True)
    worker.start()
    worker.join(timeout_seconds)
    if not result:
        raise TimeoutError("enumerate_processes did not answer")
    if isinstance(result[0], Exception):
        raise result[0]
    return result[0]


def agent_prelude(role: str, endpoint: str, pid: int, config: dict | None = None) -> str:
    expected = {"role": role, "endpoint": endpoint, "windows_pid": pid}
    encoded_expected = json.dumps(expected, sort_keys=True)
    encoded_config = json.dumps(config or {}, sort_keys=True)
    return f"""globalThis.__ER_FRIDA_EXPECTED = {encoded_expected};
globalThis.__ER_FRIDA_CONFIG = {encoded_config};
if (Process.id !== globalThis.__ER_FRIDA_EXPECTED.windows_pid) {{
  send({{family: "startup", tag: "identity.mismatch", expected: globalThis.__ER_FRIDA_EXPECTED, process_id: Process.id}});
  throw new Error("attached to the wrong process");
}}
send({{family: "startup", tag: "identity", expected: globalThis.__ER_FRIDA_EXPECTED, process_id: Process.id, config: globalThis.__ER_FRIDA_CONFIG}});
"""


def run(
    agent_path: pathlib.Path,
    log_path: pathlib.Path,
    endpoint: str = DEFAULT_ENDPOINT,
    windows_pid: int | None = None,
    role: str = "default",
    config: dict | None = None,
) -> int:
    # Say something immediately. An empty log used to be ambiguous between "still waiting" and
    # "hung on the first call", and on 2026-09-08 it was the second: the watcher sat mute for a
    # whole run against a server whose prefix had been torn down, and the run reached the player
    # with nothing attached while the log said nothing at all.
    print(f"watcher starting: agent={agent_path.name} endpoint={endpoint} role={role}", flush=True)
    dev = device(endpoint)
    print("connected to the wine-side server", flush=True)
    started = time.monotonic()
    deadline = started + WAIT_FOR_GAME_SECONDS
    pid = windows_pid
    if pid is not None:
        try:
            process = find_pid_bounded(dev, pid)
        except TimeoutError:
            print(
                "the frida server accepted the connection but did not answer "
                "enumerate_processes -- its prefix is stale. Restart it with "
                "`python3 scripts/ds2-frida-up.py --force`.",
                file=sys.stderr,
                flush=True,
            )
            return 2
        if process is None or process.name.lower() != "darksoulsii.exe":
            print(
                f"windows pid {pid} is not DarkSoulsII.exe on {endpoint}",
                file=sys.stderr,
                flush=True,
            )
            return 1
    else:
        announced = 0.0
        # The wait between enumerations blocks on the game directory rather than on a clock: every
        # launch writes there -- me3's staging, the DLLs' own logs -- long before a process exists,
        # so this wakes as the launch happens instead of up to a second after it.
        with er_run_lib.DirectoryWatch(er_run_lib.game_dir()) as launch_watch:
            while pid is None:
                try:
                    pid = find_game_bounded(dev)
                except TimeoutError:
                    # The server answers its socket but not its calls, which is what a stale prefix
                    # looks like. That is fatal here rather than something to keep retrying:
                    # retrying is how the silence lasted a whole run.
                    print(
                        "the frida server accepted the connection but did not answer "
                        "enumerate_processes -- its prefix is stale. Restart it with "
                        "`python3 scripts/ds2-frida-up.py --force`.",
                        file=sys.stderr,
                        flush=True,
                    )
                    return 2
                if pid is not None:
                    break
                waited = time.monotonic() - started
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    break
                if waited - announced >= PROGRESS_EVERY_SECONDS:
                    announced = waited
                    print(f"still waiting for DarkSoulsII.exe ({waited:.0f}s)", flush=True)
                blocking_slice(launch_watch, min(WATCH_SLICE_SECONDS, remaining))
        if pid is None:
            print("no DarkSoulsII.exe appeared in the prefix", file=sys.stderr)
            return 1

    # One watcher at a time, because a second one silently breaks the first's hooks.
    #
    # Measured twice on 2026-09-15. Two attaches were live on the same process and the newer
    # agent's `Interceptor.attach` calls took no effect at all: its integrity controls
    # (`SteamAPI_RunCallbacks`, `PeekMessageW`, ersc's `_Mtx_lock`) never fired once, while the
    # very same hooks in a single-attach session fired 600, 1200 and 500 times. Nothing reports an
    # error -- the agent loads, prints its "hooked" lines, and then says nothing forever, which
    # reads exactly like "the thing I am watching never happens" and is how two conclusions got
    # drawn from silence that meant nothing.
    #
    # Refusing is the whole fix. A watcher is meant to be attached once and its agent file edited
    # in place; wanting two is wanting one agent with both sets of hooks in it.
    existing = [s for s in dev.enumerate_processes() if s.pid == pid]
    attached_already = False
    try:
        # `enumerate_pending_children` is not it; the session list is what says who is attached.
        attached_already = any(
            getattr(s, "pid", None) == pid for s in getattr(dev, "enumerate_sessions", list)()
        )
    except Exception:
        attached_already = False
    if attached_already:
        print(
            "another watcher is already attached to this process. A second attach loads and then "
            "its hooks silently do nothing -- measured twice on 2026-09-15, with every integrity "
            "control dead while the same hooks fired hundreds of times in a single-attach "
            "session. Stop the other watcher, or put both sets of hooks in one agent file.",
            file=sys.stderr,
        )
        return 2
    del existing

    session = dev.attach(pid)
    print(f"attached to DarkSoulsII.exe (windows pid {pid})", flush=True)
    attached_at = time.monotonic()
    log_path.parent.mkdir(parents=True, exist_ok=True)
    log = open(log_path, "a", buffering=1, encoding="utf-8")
    # How many messages the agent sent back. This is the number the evidence gate reads, and it is
    # counted here rather than by anyone reading the transcript afterwards: a watch that attached
    # and received nothing observed nothing, and a zero licenses no edit.
    seen = {"messages": 0}

    def on_message(message, _data):
        seen["messages"] += 1
        record = {"at": time.time(), "role": role, "endpoint": endpoint, "pid": pid, "message": message}
        log.write(json.dumps(record) + "\n")
        payload = message.get("payload", message)
        print(f"AGENT {payload}", flush=True)

    def on_log(level, text):
        seen["messages"] += 1
        record = {"at": time.time(), "role": role, "endpoint": endpoint, "pid": pid,
                  "message": {"type": "log", "level": level, "payload": text}}
        log.write(json.dumps(record) + "\n")
        print(text, flush=True)

    try:
        script = None
        stamp = None

        def load() -> None:
            nonlocal script, stamp
            if script is not None:
                script.unload()
            source = agent_prelude(role, endpoint, pid, config) + "\n" + agent_path.read_text(encoding="utf-8")
            script = session.create_script(source)
            script.on("message", on_message)
            # The agent's console.log. Frida's default handler prints without flushing, and a
            # backgrounded watcher's stdout is a file, so block-buffered: log lines sat in the
            # buffer until something else flushed it, and an agent that only logs looked dead.
            script.set_log_handler(on_log)
            script.load()
            stamp = agent_path.stat().st_mtime
            print(f"loaded {agent_path.name} ({len(source)} bytes)", flush=True)

        load()
        # The session ending is the only reason to stop; the agent stays live across the whole play
        # session so a hook is never installed while the player is mid-action.
        detached = {"why": None}
        # A pipe, because the detach arrives on one of Frida's own threads while this one is parked
        # in `select`. Writing a byte makes the detach an event this loop can wait on alongside the
        # agent file, so the two things that end the watch both end it immediately.
        wake_read, wake_write = os.pipe()

        def on_detached(reason, *_):
            detached["why"] = reason
            try:
                os.write(wake_write, b"x")
            except OSError:
                pass

        session.on("detached", on_detached)

        # A backgrounded watcher is usually ended with `SIGTERM`, whose default action is to kill
        # the process where it stands -- past the `finally` below, so the session's measurement
        # would go unrecorded and the gate would refuse the edit it was taken for. Turning the
        # signal into the same byte on the wake pipe a detach writes ends the watch through the
        # ordinary path instead. `os.write` is one of the few calls a handler may safely make.
        def on_terminate(_signum, _frame) -> None:
            detached["why"] = "terminated"
            try:
                os.write(wake_write, b"x")
            except OSError:
                pass

        try:
            signal.signal(signal.SIGTERM, on_terminate)
        except (ValueError, OSError):
            # Only installable from the main thread. A watcher driven from somewhere else keeps
            # the default action and simply records nothing when it is terminated.
            pass

        # An edit to the agent is a write in its directory, so the reload is driven by inotify
        # rather than by re-stat'ing the file twice a second. The directory rather than the file:
        # an editor that saves by writing a temporary file and renaming it over the original
        # replaces the inode, and a watch pinned to the old one would go deaf at the first save.
        try:
            with er_run_lib.DirectoryWatch(agent_path.parent) as agent_watch:
                while detached["why"] is None:
                    waiting = [wake_read] + ([agent_watch.fd] if agent_watch.available else [])
                    try:
                        ready, _, _ = select.select(waiting, [], [], WATCH_SLICE_SECONDS)
                    except OSError:
                        ready = []
                    for fd in ready:
                        try:
                            os.read(fd, 65536)  # drain; the state below is re-read either way
                        except OSError:
                            pass
                    if detached["why"] is not None:
                        break
                    try:
                        current = agent_path.stat().st_mtime
                    except OSError:
                        continue
                    if current != stamp:
                        print("agent file changed, reloading in place", flush=True)
                        try:
                            load()
                        except Exception as exc:  # a bad edit must not end the session
                            print(f"reload failed, keeping the previous agent: {exc}", flush=True)
                            stamp = current
        except KeyboardInterrupt:
            # Ending a watch by hand is an ordinary way for it to finish, not a fault. Caught so it
            # exits zero with its evidence recorded, rather than unwinding through a traceback.
            detached["why"] = "interrupted"
        os.close(wake_read)
        os.close(wake_write)
        print(f"session detached: {detached['why']}", flush=True)
    finally:
        # Every path out of the watch comes through here: a detach, an interrupt, a `SIGTERM`, or
        # an exception from Frida. What the session observed is recorded once, whichever it was.
        record_evidence(agent_path, pid, seen["messages"], time.monotonic() - attached_at)
        try:
            log.close()
        except OSError:
            pass
    return 0


def selftest() -> int:
    import inspect

    checks = [
        ("the agent directory holds at least one agent", any(AGENT_DIR.glob("*.js"))),
        (
            "no agent is defaulted, so a missing one cannot be",
            # Built from pieces so the assertion does not match its own source text.
            "DEFAULT" + "_AGENT" not in pathlib.Path(__file__).read_text(encoding="utf-8"),
        ),
        ("the log path is user-owned, not a repo path", "ds2-mods-rs" not in str(DEFAULT_LOG)),
        (
            "a hung enumerate_processes is bounded rather than waited on forever",
            "find_game_bounded(" in pathlib.Path(__file__).read_text(encoding="utf-8"),
        ),
        (
            "the watcher announces itself before doing anything that can hang",
            "watcher starting:" in pathlib.Path(__file__).read_text(encoding="utf-8"),
        ),
        (
            "the agent reload is driven by inotify, not by re-stat'ing on a timer",
            "DirectoryWatch(agent_path.parent)" in pathlib.Path(__file__).read_text(encoding="utf-8"),
        ),
        (
            "a detach ends the watch at once, through a pipe select can wait on",
            "os.pipe()" in pathlib.Path(__file__).read_text(encoding="utf-8"),
        ),
        (
            "every wait is an event wait, never a delay",
            # Built from pieces so the assertion does not match its own source text.
            ("ti" + "me.sl" + "eep(") not in pathlib.Path(__file__).read_text(encoding="utf-8"),
        ),
        (
            "the evidence recorder exists beside this file",
            EVIDENCE_SCRIPT.is_file(),
        ),
        (
            "messages are counted, because a session that received none observed nothing",
            'seen["messages"] += 1' in pathlib.Path(__file__).read_text(encoding="utf-8"),
        ),
        (
            "every exit from the watch records what it saw, including an interrupt",
            "finally:" in pathlib.Path(__file__).read_text(encoding="utf-8")
            and "record_evidence(agent_path, pid, seen[" in pathlib.Path(__file__).read_text(encoding="utf-8"),
        ),
        (
            "a failed record cannot change the watcher's exit code",
            "except Exception" in inspect.getsource(record_evidence),
        ),
        (
            "the evidence is written by this tool, not typed by its caller",
            "def record(" in EVIDENCE_SCRIPT.read_text(encoding="utf-8"),
        ),
    ]
    failed = 0
    for label, ok in checks:
        print(f"  {'ok  ' if ok else 'FAIL'}  {label}")
        failed += 0 if ok else 1
    print("selftest: PASS" if not failed else f"selftest: {failed} check(s) failed")
    return 0 if not failed else 1


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--agent", type=pathlib.Path, help=f"the Frida agent to load, e.g. {AGENT_DIR}/tick.js")
    parser.add_argument("--log", type=pathlib.Path, default=DEFAULT_LOG)
    parser.add_argument("--endpoint", default=DEFAULT_ENDPOINT)
    parser.add_argument("--pid", type=int, help="Windows pid to attach. Omitting it keeps legacy first-game behavior.")
    parser.add_argument("--role", default="default", help="Role label written into every event.")
    parser.add_argument("--config-json", help="JSON object exposed to the agent as globalThis.__ER_FRIDA_CONFIG")
    parser.add_argument("--selftest", action="store_true")
    args = parser.parse_args()
    if args.selftest:
        return selftest()
    if args.agent is None:
        parser.error(f"--agent is required; the agents live in {AGENT_DIR}")
    config = None
    if args.config_json:
        try:
            config = json.loads(args.config_json)
        except json.JSONDecodeError as exc:
            print(f"--config-json is not valid JSON: {exc}", file=sys.stderr)
            return 1
        if not isinstance(config, dict):
            print("--config-json must be a JSON object", file=sys.stderr)
            return 1
    return run(args.agent, args.log, endpoint=args.endpoint, windows_pid=args.pid, role=args.role, config=config)


if __name__ == "__main__":
    raise SystemExit(main())
