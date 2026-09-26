#!/usr/bin/env python3
"""What a pending Bash command would push: which refs, and whether it commits first.

Read by `.cupcake/signals/runtime_evidence_for_head.sh`, which answers "has the code this push
carries ever run" and used to answer it about `HEAD` as it stood when the hook fired.

# The miss this exists to close, measured 2026-09-25

The main checkout was on a fresh branch cut from `origin/main`, with `scripts/ds2-run.py`,
`README.md` and `docs/DS2-OFFLINE.md` edited and not committed. One Bash call did both halves:

    git commit -qam "$(printf 'chore(scripts): the launcher takes no switch ...')" \
        && git push -u origin launcher-drop-flag 2>&1 | tail -3

and DS2-MODS-REQUIRE-RUNTIME-BEFORE-PUSH let it through, with `ds2-loader.log` last written 330
seconds before the commit. The PreToolUse hook runs before the command does, so at that moment the
commit did not exist: `HEAD` was still `origin/main`, `git diff origin/main...HEAD` was empty,
`game_code` came out 0, and a push of launcher code was judged to be a push of nothing. Earlier in
the same session the identical `git push -u origin <branch>` was refused, because there the commit
had been made by a separate command and was already in `HEAD`.

So the signal needs two facts about the command that it could not see:

  * `COMMIT 1` -- a `git commit` runs before the push in the same command. The commit the push
    will carry is then made of what is in the working tree now, and no run can have tested a
    commit that does not exist yet.
  * `REF <src>` -- the source side of each refspec pushed, or `HEAD` when none is named. A push of
    `git push origin other-branch` from a checkout sitting on a docs branch carries `other-branch`,
    not `HEAD`, and the diff has to be taken over the thing being pushed.
  * `ALL 1` -- `--all` or `--mirror`, which push every branch. Not resolved further: the signal
    treats it as game code, the fail-closed answer.
  * `DIR <path>` -- the directory the push runs in, printed only when the command moves it: a `cd`
    or `pushd` earlier in the same command, or `git -C <dir>` on the push itself. `DIR ?` when the
    text alone cannot say where (`cd "$VAR"`, `cd -`, `popd`, pushes from two different
    directories), and the signal then refuses rather than guess. Without this the signal judged
    the checkout the hook was invoked from: `cd <worktree> && git push` run from the main checkout
    was refused as `dll_match=0` although the worktree's built and staged DLLs were
    byte-identical (ds2-mods-rs-lgor, 2026-09-26).

Reads the hook event JSON on stdin (its `cwd` is where relative moves start), or takes `--command`
and `--cwd`. Prints nothing for a command that pushes
nothing. A command that names both `commit` and `push` and will not lex prints `COMMIT 1` and
`REF HEAD`, because silence there is the exact miss above.
"""
from __future__ import annotations

import argparse
import json
import os
import shlex
import sys
from pathlib import Path

SHELL_WRAPPERS = {"bash", "sh", "zsh", "dash", "ksh", "fish"}

TRANSPARENT_PREFIXES = {"command", "builtin", "exec", "nohup", "time", "sudo", "env", "timeout"}

# Global options that consume the next word, so the subcommand is found after them.
GIT_VALUE_OPTIONS = {"-C", "-c", "--git-dir", "--work-tree", "--namespace", "--config-env", "--exec-path"}

# `git push` options that consume the next word. Everything else starting with `-` stands alone or
# carries its value after `=`.
PUSH_VALUE_OPTIONS = {"--repo", "-o", "--push-option", "--receive-pack", "--exec"}


def lex(command: str) -> list[str] | None:
    lexer = shlex.shlex(command, posix=True, punctuation_chars=True)
    lexer.whitespace_split = True
    try:
        return list(lexer)
    except ValueError:
        return None


def strip_prefixes(segment: list[str]) -> list[str]:
    index = 0
    while index < len(segment):
        word = segment[index]
        if "=" in word and not word.startswith("-") and word.split("=", 1)[0].isidentifier():
            index += 1
            continue
        if Path(word).name in TRANSPARENT_PREFIXES:
            index += 1
            # `timeout 30 git push`: the duration belongs to the wrapper.
            if index < len(segment) and segment[index].replace(".", "").rstrip("smhd").isdigit():
                index += 1
            continue
        break
    return segment[index:]


def git_subcommand(segment: list[str]) -> tuple[str, list[str]] | None:
    """(`subcommand`, its arguments) when this segment runs git, else None."""
    words = strip_prefixes(segment)
    if not words or Path(words[0]).name != "git":
        return None
    index = 1
    while index < len(words):
        word = words[index]
        if word in GIT_VALUE_OPTIONS:
            index += 2
            continue
        if word.startswith("-"):
            index += 1
            continue
        return word, words[index + 1:]
    return None


def shell_payload(segment: list[str]) -> str | None:
    """The `-c` string of `bash -c '...'`, which is a command of its own."""
    words = strip_prefixes(segment)
    if not words or Path(words[0]).name not in SHELL_WRAPPERS:
        return None
    for index, word in enumerate(words[1:], start=1):
        if word.startswith("-") and not word.startswith("--") and "c" in word[1:]:
            if index + 1 < len(words):
                return words[index + 1]
    return None


def push_refs(args: list[str]) -> tuple[list[str], bool]:
    """The source refs a `git push <args>` would send, and whether it pushes every branch."""
    positional: list[str] = []
    everything = False
    index = 0
    while index < len(args):
        word = args[index]
        nxt = args[index + 1] if index + 1 < len(args) else ""
        if word and all(character in "<>&" for character in word):
            index += 2  # a redirect operator and its target, as `2>&1` lexes: `2`, `>&`, `1`
            continue
        if word.isdigit() and nxt and all(character in "<>&" for character in nxt):
            index += 1  # the file descriptor in front of a redirect
            continue
        if word in ("--all", "--mirror", "--branches"):
            everything = True
        elif word in PUSH_VALUE_OPTIONS:
            index += 1
        elif word.startswith("-"):
            pass
        else:
            positional.append(word)
        index += 1
    refs: list[str] = []
    for spec in positional[1:]:
        source = spec.lstrip("+").split(":", 1)[0]
        if source:  # `:branch` deletes a remote branch and sends no commits
            refs.append(source)
    if not refs and not everything and not any(spec.startswith(":") for spec in positional[1:]):
        refs.append("HEAD")
    return refs, everything


UNRESOLVED = "?"


def resolve_dir(current: str, target: str) -> str:
    """`current` moved by `cd target`, or UNRESOLVED when the text alone cannot say where."""
    if current == UNRESOLVED or not target or target == "-" or any(c in target for c in "$`*?[{"):
        return UNRESOLVED
    target = os.path.expanduser(target)
    if target.startswith("~"):
        return UNRESOLVED  # `~nosuchuser`
    return os.path.normpath(os.path.join(current, target))


def cd_target(words: list[str]) -> str | None:
    """The argument of `cd`/`pushd`; "" when it cannot be picked out or is `popd`; None if no cd."""
    if not words or words[0] not in ("cd", "pushd", "popd"):
        return None
    if words[0] == "popd":
        return ""
    rest = words[1:]
    if "--" in rest:
        args = rest[rest.index("--") + 1:]
    else:
        args = [word for word in rest if not (word.startswith("-") and word != "-")]
    if not args:
        return "~" if words[0] == "cd" else ""
    return args[0] if len(args) == 1 else ""


def git_dir_options(segment: list[str]) -> list[str]:
    """Every `-C <dir>` given to git before its subcommand, in order; git composes them."""
    words = strip_prefixes(segment)
    out: list[str] = []
    index = 1
    while index < len(words):
        word = words[index]
        if word == "-C" and index + 1 < len(words):
            out.append(words[index + 1])
            index += 2
        elif word in GIT_VALUE_OPTIONS:
            index += 2
        elif word.startswith("-"):
            index += 1
        else:
            break
    return out


def split_keeping_operators(tokens: list[str]) -> list[tuple[list[str], str]]:
    """The command split at `;&|()` and newlines, each segment paired with the operator after it
    so a subshell's `(` ... `)` can be followed."""
    out: list[tuple[list[str], str]] = []
    current: list[str] = []
    for token in tokens:
        if token and all(character in ";&|\n()" for character in token):
            out.append((current, token))
            current = []
        else:
            current.append(token)
    out.append((current, ""))
    return out


def scope(command: str, depth: int = 0, cwd: str = "") -> tuple[bool, list[str], bool, bool, list[str]]:
    """(commits before a push, refs pushed, pushes everything, pushes at all, dir of each push)."""
    tokens = lex(command)
    if tokens is None:
        lowered = command.lower()
        if "push" in lowered:
            # Unlexable: where it pushes from is as unknown as what it pushes, once it moves.
            moved = "cd " in lowered or "pushd" in lowered or "-c " in lowered
            return ("commit" in lowered, ["HEAD"], False, True, [UNRESOLVED if moved else cwd])
        return (False, [], False, False, [])
    committed = False
    commit_before_push = False
    refs: list[str] = []
    everything = False
    pushes = False
    dirs: list[str] = []
    here = cwd
    # A `(` opens a subshell, and a `cd` inside it does not survive the `)`.
    saved: list[str] = []
    for segment, operator in split_keeping_operators(tokens):
        payload = shell_payload(segment) if segment else None
        if payload is not None and depth < 3:
            inner = scope(payload, depth + 1, here)
            if inner[3]:
                pushes = True
                refs.extend(inner[1])
                everything = everything or inner[2]
                commit_before_push = commit_before_push or inner[0] or committed
                dirs.extend(inner[4])
            if "commit" in payload:
                committed = True
        elif segment:
            target = cd_target(strip_prefixes(segment))
            if target is not None:
                here = resolve_dir(here, target) if target else UNRESOLVED
            found = git_subcommand(segment)
            if found is not None:
                verb, args = found
                if verb == "commit":
                    committed = True
                elif verb == "push":
                    pushes = True
                    if committed:
                        commit_before_push = True
                    more, every = push_refs(args)
                    refs.extend(more)
                    everything = everything or every
                    where = here
                    for option in git_dir_options(segment):
                        where = resolve_dir(where, option)
                    dirs.append(where)
        for character in operator:
            if character == "(":
                saved.append(here)
            elif character == ")":
                here = saved.pop() if saved else UNRESOLVED
    return commit_before_push, refs, everything, pushes, dirs


def render(command: str, cwd: str = "") -> str:
    """`cwd` is where the command starts; relative `cd`s and `-C`s resolve against it."""
    base = cwd if os.path.isabs(cwd) else ""
    commit_before_push, refs, everything, pushes, dirs = scope(command, cwd=base)
    if not pushes:
        return ""
    lines = [f"COMMIT {int(commit_before_push)}"]
    lines.extend(f"REF {ref}" for ref in dict.fromkeys(refs))
    if everything:
        lines.append("ALL 1")
    # Printed only when some push runs somewhere other than where the command started, so a plain
    # `git push` reads exactly as it did before this existed. With no known start a relative move
    # resolves to a relative path, which is as good as unknown.
    if any(where != base for where in dirs):
        distinct = set(dirs)
        where = distinct.pop() if len(distinct) == 1 else UNRESOLVED
        if not os.path.isabs(where):
            where = UNRESOLVED
        lines.append(f"DIR {where}")
    return "\n".join(lines)


def selftest() -> int:
    cases = [
        # The 2026-09-25 miss, verbatim.
        (
            "git commit -qam \"$(printf 'chore(scripts): the launcher takes no switch to play online\\n')\" "
            "&& git push -u origin launcher-drop-flag 2>&1 | tail -3",
            "COMMIT 1\nREF launcher-drop-flag",
        ),
        ("git push -u origin menu-rows-locked-in-multiplayer 2>&1 | tail -5",
         "COMMIT 0\nREF menu-rows-locked-in-multiplayer"),
        ("git push", "COMMIT 0\nREF HEAD"),
        ("git pull --rebase && git push", "COMMIT 0\nREF HEAD"),
        ("git push origin HEAD:refs/heads/x", "COMMIT 0\nREF HEAD"),
        ("git push --force-with-lease origin +feature:feature", "COMMIT 0\nREF feature"),
        ("git push origin :stale-branch", "COMMIT 0"),
        ("git push --all origin", "COMMIT 0\nALL 1"),
        ("git -C /tmp/x commit -m y; git -C /tmp/x push", "COMMIT 1\nREF HEAD\nDIR /tmp/x"),
        ("bash -c 'git commit -am x && git push origin b'", "COMMIT 1\nREF b"),
        # A push BEFORE the commit carries only what was already committed.
        ("git push && git commit -am next", "COMMIT 0\nREF HEAD"),
        ("git commit -m 'mentions git push in the message'", ""),
        ("git log --oneline", ""),
        ("echo 'unterminated && git commit && git push", "COMMIT 1\nREF HEAD"),
        # ds2-mods-rs-lgor, verbatim but for the worktree path: the push runs in the worktree the
        # command cds into, not in the checkout the hook was invoked from (`/r` here).
        (
            "H=/home/banon/projects/ds2-mods-rs-wt-hk/scripts/ds2-harness.sh; timeout 8 $H 'block 30' "
            "'buttons 0x2000 4' >/dev/null 2>&1; sleep 0.5; timeout 8 $H 'block 30' 'buttons 0x2000 4' "
            ">/dev/null 2>&1; cd /r/.claude/worktrees/agent-a && git push -u origin build-url-dialog "
            "2>&1 | tail -2",
            "COMMIT 0\nREF build-url-dialog\nDIR /r/.claude/worktrees/agent-a",
        ),
        ("cd .claude/worktrees/a && git push", "COMMIT 0\nREF HEAD\nDIR /r/.claude/worktrees/a"),
        ("cd /x; cd ../y && git push", "COMMIT 0\nREF HEAD\nDIR /y"),
        ("cd -- /x && git push", "COMMIT 0\nREF HEAD\nDIR /x"),
        ("git -C /r/.claude/worktrees/a push origin b", "COMMIT 0\nREF b\nDIR /r/.claude/worktrees/a"),
        ("cd /x && git -C sub -C deeper push", "COMMIT 0\nREF HEAD\nDIR /x/sub/deeper"),
        # Moves that end where they started are not moves.
        ("git -C /r push", "COMMIT 0\nREF HEAD"),
        ("cd /r && git push", "COMMIT 0\nREF HEAD"),
        # A subshell's cd dies at its `)`; a `bash -c` payload's cd applies inside it.
        ("(cd /x && git fetch) && git push", "COMMIT 0\nREF HEAD"),
        ("(cd /x && git push)", "COMMIT 0\nREF HEAD\nDIR /x"),
        ("bash -c 'cd /x && git push'", "COMMIT 0\nREF HEAD\nDIR /x"),
        ("cd /x && bash -c 'git push'", "COMMIT 0\nREF HEAD\nDIR /x"),
        # The text cannot say where these push from.
        ('cd "$WT" && git push', "COMMIT 0\nREF HEAD\nDIR ?"),
        ("cd - && git push", "COMMIT 0\nREF HEAD\nDIR ?"),
        ("pushd /x && popd && git push", "COMMIT 0\nREF HEAD\nDIR ?"),
        ("cd /x && git push; cd /y && git push", "COMMIT 0\nREF HEAD\nDIR ?"),
        ("git -C $D push", "COMMIT 0\nREF HEAD\nDIR ?"),
    ]
    bad = 0
    for command, want in cases:
        got = render(command, "/r")
        ok = got == want
        bad += 0 if ok else 1
        print(f"  {'ok  ' if ok else 'FAIL'} {command[:70]!r} -> {got!r}" + ("" if ok else f" (want {want!r})"))
    print("selftest: PASS" if not bad else f"selftest: {bad} FAILED")
    return 1 if bad else 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--command", help="the command to read, instead of an event on stdin")
    parser.add_argument("--cwd", help="where the command starts (default: the event's cwd, else ours)")
    parser.add_argument("--selftest", action="store_true")
    args = parser.parse_args()
    if args.selftest:
        return selftest()
    command = args.command
    cwd = args.cwd
    if command is None:
        try:
            event = json.load(sys.stdin)
        except (ValueError, OSError):
            return 0
        tool_input = event.get("tool_input") if isinstance(event, dict) else None
        command = tool_input.get("command") if isinstance(tool_input, dict) else None
        if not isinstance(command, str):
            return 0
        if cwd is None and isinstance(event, dict) and isinstance(event.get("cwd"), str):
            cwd = event["cwd"]
    out = render(command, cwd or os.getcwd())
    if out:
        print(out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
