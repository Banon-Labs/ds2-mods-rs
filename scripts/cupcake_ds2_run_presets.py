#!/usr/bin/env python3
"""Whether each `ds2-run.py` a pending Bash command launches keeps the user's F1-saved presets.

Read by `.cupcake/signals/ds2_run_presets.sh` for DS2-MODS-REFUSE-PRESET-WIPING-LAUNCH
(`.cupcake/policies/claude/ds2_run_preset_guard.rego`).

# The incident (2026-09-28, the second one)

The user's DS2 Lighting Engine atmosphere presets, `Game/ds2le_atmosphere_presets/
atmospheres_extended.ini`, saved from the engine's F1 menu, were overwritten with Second Sin's stock
copy (sha256 edaf06bb...). main already kept them: d694947 added `ENGINE_SHIPPED_PRESETS` to
`scripts/ds2-run.py`, so a preset file is reinstalled only while it still holds bytes the engine
shipped. The launches that wiped it ran `scripts/ds2-run.py` from worktrees whose branches predate
d694947, and that copy of the launcher reinstalls Second Sin's presets over whatever is there.

There is one game install and every worktree's launcher writes into it, so the launcher that runs
is whichever copy the command names -- not main's. This reads that copy.

# What it emits

    CHECKED                                  always, first, so the policy can tell "looked and
                                             found nothing" from "the signal never ran"
    launch\t<realpath>\tpresets=<0|1>        one per real launch of an existing ds2-run.py
    unknown\t<token>\t<reason>               a launch whose script cannot be resolved to a file

A real launch is `ds2-run.py` run as a program -- directly, or as the script operand of a python
interpreter -- without `--dry-run`, `--selftest`, `-h` or `--help`, which stage nothing. A path that
names no existing file emits nothing: python fails on it and nothing is written into the game.

The walk follows a leading `cd`/`pushd` (restored at the end of a `( ... )` subshell), `bash -c`
payloads, and heredocs fed to a shell. A heredoc fed to anything else (a commit message, a python
program) is data and is skipped, so prose naming an old worktree's launcher is not a launch.
"""
from __future__ import annotations

import json
import os
import re
import shlex
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import cupcake_push_target_repo as target_repo  # noqa: E402

MARKER = "ENGINE_SHIPPED_PRESETS"
SCRIPT_NAME = "ds2-run.py"
NON_LAUNCH_FLAGS = {"--dry-run", "--selftest", "-h", "--help"}

PYTHON = re.compile(r"^(python|pypy)(3(\.\d+)?)?$")
# Interpreter options that take a separate value, so the value is not read as the script.
PYTHON_VALUE_OPTIONS = {"-W", "-X", "--check-hash-based-pycs"}
EXTRA_PREFIXES = {"setsid", "stdbuf", "nice", "ionice", "unbuffer"}
SHELLS = target_repo.SHELL_WRAPPERS
LAUNCH_SHAPE = re.compile(
    r"(?:^|[;&|(]\s*|(?:python|pypy)[0-9.]*\s+(?:-\S+\s+)*)[^\s;&|()'\"]*ds2-run\.py(?=[\s;&|)]|$)",
    re.MULTILINE,
)
HEREDOC = re.compile(r"<<-?[ \t]*(['\"]?)([A-Za-z_][A-Za-z0-9_]*)\1")


def split_heredocs(command: str) -> tuple[str, list[tuple[str, str]]]:
    """The command with heredoc bodies removed, and each body with the text that opened it.

    Not line-based. The command a signal receives is not the one typed: scripts/cupcake-hook.sh
    rewrites unquoted newlines to `; `, and cupcake's whitespace normalisation then collapses the
    rest, so `cat <<'EOF'<newline>body<newline>EOF<newline>echo` arrives as one line,
    `cat <<'EOF' body EOF; echo` (scripts/test-cupcake-delivered-shape.py measures both). The
    terminator is the tag as a whole word followed by a separator or the end, in either form.
    """
    out: list[str] = []
    bodies: list[tuple[str, str]] = []
    pos = 0
    while True:
        match = HEREDOC.search(command, pos)
        if not match:
            out.append(command[pos:])
            break
        tag = match.group(2)
        opening = "".join(out) + command[pos : match.start()]
        out.append(command[pos : match.start()])
        end = re.compile(r"(?:^|(?<=[\s;]))" + re.escape(tag) + r"(?=[ \t]*(?:[;&|\n]|$))", re.M)
        ended = end.search(command, match.end())
        if not ended:
            bodies.append((opening, command[match.end() :]))
            break
        bodies.append((opening, command[match.end() : ended.start()]))
        out.append("\n")
        pos = ended.end()
    return "".join(out), bodies


def feeds_a_shell(opening: str) -> bool:
    """Whether the command a heredoc is attached to is a shell reading its program from stdin."""
    tokens = target_repo.lex(opening) or []
    segs = segments(tokens)
    if not segs:
        return False
    words = strip(segs[-1])
    if not words or Path(words[0]).name not in SHELLS:
        return False
    return target_repo.shell_payload(words) is None


def segments(tokens: list[str]) -> list[list[str]]:
    """Commands split at `;`, `&`, `|` and at subshell parens, the parens kept as their own entry."""
    out: list[list[str]] = [[]]
    for token in tokens:
        if token and all(c in ";&|\n" for c in token):
            out.append([])
        elif token and all(c in "()" for c in token):
            for c in token:
                out.append([c])
            out.append([])
        else:
            out[-1].append(token)
    return [s for s in out if s]


def strip(segment: list[str]) -> list[str]:
    words = target_repo.strip_prefixes(segment)
    while words and Path(words[0]).name in EXTRA_PREFIXES:
        words = words[1:]
        while words and words[0].startswith("-"):
            words = words[1:]
        words = target_repo.strip_prefixes(words)
    return words


def script_and_args(words: list[str]) -> tuple[str, list[str]] | None:
    """The `ds2-run.py` this command runs and its arguments, or None."""
    if not words:
        return None
    if Path(words[0]).name == SCRIPT_NAME:
        return words[0], words[1:]
    if not PYTHON.match(Path(words[0]).name):
        return None
    index = 1
    while index < len(words):
        word = words[index]
        if word in ("-c", "-m", "-"):
            return None
        if word in PYTHON_VALUE_OPTIONS:
            index += 2
            continue
        if word.startswith("-"):
            index += 1
            continue
        break
    if index >= len(words) or Path(words[index]).name != SCRIPT_NAME:
        return None
    return words[index], words[index + 1 :]


def resolve(token: str, cwd: str) -> str | None:
    # The two variables a hook's environment carries the same value for as the Bash tool's shell.
    # `$CLAUDE_PROJECT_DIR/scripts/ds2-run.py` is a spelling bash_no_python_file_write allows.
    for name in ("CLAUDE_PROJECT_DIR", "HOME"):
        value = os.environ.get(name)
        if value:
            token = token.replace("${" + name + "}", value).replace("$" + name + "/", value + "/")
    if "$" in token or "`" in token:
        return None
    path = os.path.expanduser(token)
    if not os.path.isabs(path):
        path = os.path.join(cwd, path)
    return os.path.normpath(path)


def walk(command: str, cwd: str, out: list[str], depth: int = 0) -> None:
    if depth > 4:
        return
    text, bodies = split_heredocs(command)
    for opening, body in bodies:
        if feeds_a_shell(opening):
            walk(body, cwd, out, depth + 1)
    tokens = target_repo.lex(text)
    if tokens is None:
        # Prose in an unbalanced quote (a commit message's "user's") does not lex; refuse only
        # when the text still has a launch-shaped ds2-run.py in it, the same shape the policy
        # falls back on when this signal is silent.
        for m in LAUNCH_SHAPE.finditer(text):
            tail = text[m.end() :].split("\n", 1)[0]
            if not any(re.search(r"(^|\s)" + re.escape(f) + r"(\s|$|[;&|)])", tail)
                       for f in NON_LAUNCH_FLAGS):
                out.append(f"unknown\t{SCRIPT_NAME}\tthe command does not lex")
                return
        return
    current = cwd
    stack: list[str] = []
    for segment in segments(tokens):
        if segment == ["("]:
            stack.append(current)
            continue
        if segment == [")"]:
            current = stack.pop() if stack else current
            continue
        words = strip(segment)
        if not words:
            continue
        verb = Path(words[0]).name
        if verb in ("cd", "pushd"):
            operands = [w for w in words[1:] if w != "--"]
            if not operands:
                current = os.path.expanduser("~")
            elif operands[0].startswith("-") or "$" in operands[0] or "`" in operands[0]:
                current = ""
            else:
                current = os.path.normpath(os.path.join(current, os.path.expanduser(operands[0])))
            continue
        payload = target_repo.shell_payload(words)
        if payload is not None:
            walk(payload, current, out, depth + 1)
            continue
        found = script_and_args(words)
        if found is None:
            continue
        token, args = found
        if any(a in NON_LAUNCH_FLAGS for a in args):
            continue
        if not current and not os.path.isabs(os.path.expanduser(token)):
            out.append(f"unknown\t{token}\ta `cd` the guard cannot follow comes before it")
            continue
        path = resolve(token, current)
        if path is None:
            out.append(f"unknown\t{token}\tthe path is built from a variable or substitution")
            continue
        if not os.path.isfile(path):
            continue
        real = os.path.realpath(path)
        try:
            source = Path(real).read_text(encoding="utf-8", errors="replace")
        except OSError:
            out.append(f"unknown\t{token}\tthe script cannot be read")
            continue
        out.append(f"launch\t{real}\tpresets={int(MARKER in source)}")


def render(command: str, cwd: str) -> str:
    out = ["CHECKED"]
    walk(command, cwd, out)
    return "\n".join(out)


def selftest() -> int:
    import tempfile

    failures = 0
    root = Path(tempfile.mkdtemp(prefix="ds2-run-presets-"))
    old = root / "old" / "scripts" / SCRIPT_NAME
    new = root / "new" / "scripts" / SCRIPT_NAME
    for path, text in ((old, "SECOND_SIN_PINS = {}\n"), (new, f"{MARKER} = {{}}\n")):
        path.parent.mkdir(parents=True)
        path.write_text(text, encoding="utf-8")

    def check(name: str, command: str, cwd: str, want: list[str]) -> None:
        nonlocal failures
        got = render(command, cwd).split("\n")[1:]
        ok = got == want
        failures += not ok
        print(f"  {'ok  ' if ok else 'FAIL'}  {name}" + ("" if ok else f": got {got!r}, wanted {want!r}"))

    o, n = f"launch\t{old}\tpresets=0", f"launch\t{new}\tpresets=1"
    check("old script by relative path", "python3 scripts/ds2-run.py", str(root / "old"), [o])
    check("new script by relative path", "python3 scripts/ds2-run.py --x 1", str(root / "new"), [n])
    check("absolute path into a worktree", f"python3 {old}", "/", [o])
    check("run directly", f"{old} --foo", "/", [o])
    check("cd then run", f"cd {root}/old && python3 -u scripts/ds2-run.py", "/", [o])
    check("git -C is not a cd", f"git -C {root}/old status; python3 scripts/ds2-run.py",
          str(root / "new"), [n])
    check("subshell cd is undone", f"(cd {root}/old) && python3 scripts/ds2-run.py",
          str(root / "new"), [n])
    check("dry run is not a launch", f"python3 {old} --dry-run", "/", [])
    check("selftest is not a launch", f"python3 {old} --selftest", "/", [])
    check("help is not a launch", f"python3 {old} -h", "/", [])
    check("bash -c payload", f"bash -c 'cd {root}/old && python3 scripts/ds2-run.py'", "/", [o])
    check("setsid/nohup prefix", f"setsid -f nohup python3 {old} &", "/", [o])
    check("grep is not a launch", f"grep -n foo {old}", "/", [])
    check("missing file emits nothing", "python3 scripts/ds2-run.py", str(root), [])
    check("variable path is unknown", "python3 $W/scripts/ds2-run.py", "/",
          ["unknown\t$W/scripts/ds2-run.py\tthe path is built from a variable or substitution"])
    os.environ["CLAUDE_PROJECT_DIR"] = str(root / "old")
    check("project dir variable is expanded", "python3 $CLAUDE_PROJECT_DIR/scripts/ds2-run.py",
          "/", [o])
    check("commit message heredoc is data",
          f"git commit -F - <<'EOF'\npython3 {old}\nEOF", "/", [])
    check("shell heredoc is a program", f"bash <<'EOF'\npython3 {old}\nEOF", "/", [o])
    check("terminator rewritten to `EOF; ...` by the hook shim",
          f"git commit -F - <<'EOF'\npython3 {old}\nEOF; git log -1", "/", [])
    check("heredoc collapsed to one line by the hook is still data",
          f"cat <<'EOF' the user's python3 {old} note EOF; echo ok", "/", [])
    check("collapsed shell heredoc is still a program",
          f"bash <<'EOF'; python3 {old}; EOF; echo ok", "/", [o])
    check("unlexable prose naming the launcher is not a launch",
          "echo the user's ds2-run.py launch", "/", [])
    check("unlexable launch is unknown", "echo it's; python3 scripts/ds2-run.py", "/",
          ["unknown\tds2-run.py\tthe command does not lex"])
    check("python -c is not the script", f"python3 -c 'print(1)' {old}", "/", [])
    print("selftest: " + ("OK" if not failures else "FAILED"))
    return 1 if failures else 0


def main() -> int:
    if sys.argv[1:] == ["--selftest"]:
        return selftest()
    try:
        event = json.load(sys.stdin)
    except ValueError:
        print("CHECKED")
        return 0
    command = (event.get("tool_input") or {}).get("command") or ""
    cwd = event.get("cwd") or os.getcwd()
    print(render(command if isinstance(command, str) else "", cwd if isinstance(cwd, str) else ""))
    return 0


if __name__ == "__main__":
    sys.exit(main())
