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

Reads the hook event JSON on stdin, or takes `--command`. Prints nothing for a command that pushes
nothing. A command that names both `commit` and `push` and will not lex prints `COMMIT 1` and
`REF HEAD`, because silence there is the exact miss above.
"""
from __future__ import annotations

import argparse
import json
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


def segments(tokens: list[str]) -> list[list[str]]:
    out: list[list[str]] = [[]]
    for token in tokens:
        if token and all(character in ";&|\n()" for character in token):
            out.append([])
        else:
            out[-1].append(token)
    return [segment for segment in out if segment]


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


def scope(command: str, depth: int = 0) -> tuple[bool, list[str], bool, bool]:
    """(commits before a push, refs pushed, pushes everything, pushes at all)."""
    tokens = lex(command)
    if tokens is None:
        lowered = command.lower()
        if "push" in lowered:
            return ("commit" in lowered, ["HEAD"], False, True)
        return (False, [], False, False)
    committed = False
    commit_before_push = False
    refs: list[str] = []
    everything = False
    pushes = False
    for segment in segments(tokens):
        payload = shell_payload(segment)
        if payload is not None and depth < 3:
            inner = scope(payload, depth + 1)
            if inner[3]:
                pushes = True
                refs.extend(inner[1])
                everything = everything or inner[2]
                commit_before_push = commit_before_push or inner[0] or committed
            if "commit" in payload:
                committed = True
            continue
        found = git_subcommand(segment)
        if found is None:
            continue
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
    return commit_before_push, refs, everything, pushes


def render(command: str) -> str:
    commit_before_push, refs, everything, pushes = scope(command)
    if not pushes:
        return ""
    lines = [f"COMMIT {int(commit_before_push)}"]
    lines.extend(f"REF {ref}" for ref in dict.fromkeys(refs))
    if everything:
        lines.append("ALL 1")
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
        ("git -C /tmp/x commit -m y; git -C /tmp/x push", "COMMIT 1\nREF HEAD"),
        ("bash -c 'git commit -am x && git push origin b'", "COMMIT 1\nREF b"),
        # A push BEFORE the commit carries only what was already committed.
        ("git push && git commit -am next", "COMMIT 0\nREF HEAD"),
        ("git commit -m 'mentions git push in the message'", ""),
        ("git log --oneline", ""),
        ("echo 'unterminated && git commit && git push", "COMMIT 1\nREF HEAD"),
    ]
    bad = 0
    for command, want in cases:
        got = render(command)
        ok = got == want
        bad += 0 if ok else 1
        print(f"  {'ok  ' if ok else 'FAIL'} {command[:70]!r} -> {got!r}" + ("" if ok else f" (want {want!r})"))
    print("selftest: PASS" if not bad else f"selftest: {bad} FAILED")
    return 1 if bad else 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--command", help="the command to read, instead of an event on stdin")
    parser.add_argument("--selftest", action="store_true")
    args = parser.parse_args()
    if args.selftest:
        return selftest()
    command = args.command
    if command is None:
        try:
            event = json.load(sys.stdin)
        except (ValueError, OSError):
            return 0
        tool_input = event.get("tool_input") if isinstance(event, dict) else None
        command = tool_input.get("command") if isinstance(tool_input, dict) else None
        if not isinstance(command, str):
            return 0
    out = render(command)
    if out:
        print(out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
