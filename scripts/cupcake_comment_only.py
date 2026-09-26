#!/usr/bin/env python3
"""Does a pushed ref change game code, or only the comments in it?

Read by `.cupcake/signals/runtime_evidence_for_head.sh`, which decides `game_code` for
DS2-MODS-REQUIRE-RUNTIME-BEFORE-PUSH.

# Why this exists, 2026-09-26

`item-warn-spell-bind-re` changed doc comments in `crates/ds2-rva/src/lib.rs` and a file under
`docs/`, and the push was refused as "game code that has not been run". The jurisdiction test was a
path glob: anything under `crates/` is game code. A comment is not code, and no launch can tell
you anything about one.

The guard declined a documentation exemption on 2026-09-24 because the shape proposed then was a
file-extension or path carve-out, which is unsound. This is not that. Both sides of every changed
Rust file are lexed and compared with their comments removed; the ref is out of jurisdiction only
when the two token streams are identical. A changed log format string, a changed `#[cfg]`, a
reformatted string literal: all still game code.

# What counts as a comment

  * `//` and `/* */` comments, nested block comments included. The lexer drops these before any
    macro sees the tokens, so they cannot reach the DLL.
  * Doc comments (`///`, `//!`, `/** */`, `/*! */`) are `#[doc = "..."]` attributes, and an
    attribute is visible to macros: a derive such as clap's turns doc text into help strings, and a
    `macro_rules!` capturing `$(#[$meta:meta])*` can stringify it. So a doc comment is ignored only
    when neither side of the file has an attribute outside `BUILTIN_ATTRIBUTES`, a derive outside
    `BUILTIN_DERIVES`, or a meta capture. Otherwise the doc text stays in the comparison.
  * Whitespace outside literals is collapsed to one separator, so a comment removed from between two
    tokens leaves them apart, as the compiler sees them.

# What still changes in the binary

Line numbers. A comment that adds or removes lines moves every later `panic!`/`unwrap` location
and `line!()`, and those are strings in the DLL. That changes what a panic message says, never
what the code does, and it is the whole of the difference.

# Fail closed

A game path that is added, deleted, not a `.rs` file, not valid UTF-8, or does not lex (an
unterminated string or comment) is game code. So is any git error. The CLI prints `GAME 1` or
`GAME 0`; the signal treats anything else, including a crash, as `GAME 1`.
"""
from __future__ import annotations

import argparse
import re
import subprocess
import sys

BUILTIN_ATTRIBUTES = {
    "allow", "warn", "deny", "forbid", "expect", "cfg", "cfg_attr", "test", "ignore", "should_panic",
    "must_use", "repr", "inline", "cold", "link", "link_name", "link_section", "no_mangle", "unsafe",
    "export_name", "used", "default", "non_exhaustive", "derive", "doc", "deprecated", "track_caller",
    "path", "macro_export", "macro_use", "no_std", "no_main", "windows_subsystem", "target_feature",
    "automatically_derived", "rustfmt", "clippy", "naked", "crate_type", "crate_name",
}
BUILTIN_DERIVES = {"Clone", "Copy", "Debug", "Default", "Eq", "Hash", "PartialEq", "Ord", "PartialOrd"}

GIT_TIMEOUT_SECONDS = 10.0


class LexError(Exception):
    pass


def is_game_path(path: str) -> bool:
    # The same test the signal's path glob applies: `*crates/*|*scripts/ds2-run.py*`.
    return "crates/" in path or "scripts/ds2-run.py" in path


def lex(source: str) -> tuple[list[str], list[str]]:
    """(code tokens with non-doc comments removed, doc comment texts in order).

    Not a full Rust lexer: it only needs to know where literals and comments start and end, so it
    tracks strings, raw strings, byte/C strings, char literals versus lifetimes, and comments.
    Everything else is copied through, with runs of whitespace collapsed to one space.
    """
    out: list[str] = []
    docs: list[str] = []
    i = 0
    n = len(source)
    pending_space = False

    def emit(text: str) -> None:
        nonlocal pending_space
        if pending_space and out:
            out.append(" ")
        pending_space = False
        out.append(text)

    def string_end(src: str, quote: int, prefix: str) -> int:
        """Emit a `"..."` literal whose opening quote is at `quote`; return the index after it."""
        j = quote + 1
        while j < n and src[j] != '"':
            j += 2 if src[j] == "\\" else 1
        if j >= n:
            raise LexError("unterminated string")
        emit(prefix + src[quote:j + 1])
        return j + 1

    def char_or_lifetime(src: str, quote: int, prefix: str) -> int:
        """Emit a char literal, or the lone quote of a lifetime/label; return where to resume."""
        q = quote + 1
        if q < n and src[q] == "\\":
            end = src.find("'", q + 2)
            if end == -1:
                raise LexError("unterminated char literal")
            emit(prefix + src[quote:end + 1])
            return end + 1
        if q + 1 < n and src[q + 1] == "'":
            emit(prefix + src[quote:q + 2])
            return q + 2
        emit(prefix + "'")  # a lifetime or label: its name follows as an ordinary word
        return q

    while i < n:
        c = source[i]
        if c.isspace():
            pending_space = True
            i += 1
            continue
        if source.startswith("//", i):
            end = source.find("\n", i)
            end = n if end == -1 else end
            text = source[i:end]
            if (text.startswith("///") and not text.startswith("////")) or text.startswith("//!"):
                docs.append(text[3:].strip())
            pending_space = True
            i = end
            continue
        if source.startswith("/*", i):
            depth = 1
            j = i + 2
            while j < n and depth:
                if source.startswith("/*", j):
                    depth += 1
                    j += 2
                elif source.startswith("*/", j):
                    depth -= 1
                    j += 2
                else:
                    j += 1
            if depth:
                raise LexError("unterminated block comment")
            text = source[i:j]
            if (text.startswith("/**") and not text.startswith("/***") and text != "/**/") or text.startswith("/*!"):
                docs.append(text[3:-2].strip())
            pending_space = True
            i = j
            continue
        if c.isalnum() or c == "_":
            j = i
            while j < n and (source[j].isalnum() or source[j] == "_"):
                j += 1
            word = source[i:j]
            raw = re.match(r"(#*)\"", source[j:j + 256]) if word in ("r", "br", "cr") else None
            if raw:
                close = '"' + raw.group(1)
                end = source.find(close, j + raw.end())
                if end == -1:
                    raise LexError("unterminated raw string")
                emit(source[i:end + len(close)])
                i = end + len(close)
                continue
            if word in ("b", "c") and source.startswith('"', j):
                i = string_end(source, j, word)
                continue
            if word == "b" and source.startswith("'", j):
                i = char_or_lifetime(source, j, word)
                continue
            emit(word)
            i = j
            continue
        if c == '"':
            i = string_end(source, i, "")
            continue
        if c == "'":
            i = char_or_lifetime(source, i, "")
            continue
        emit(c)
        i += 1
    return out, docs


def docs_can_matter(source: str) -> bool:
    """Whether a doc comment in this file could reach something that turns it into code or data."""
    if re.search(r"\$\(\s*#\s*\[|#\s*\[\s*\$", source):
        return True
    for name in re.findall(r"#!?\s*\[\s*([A-Za-z_][A-Za-z0-9_:]*)", source):
        if name.split("::")[0] not in BUILTIN_ATTRIBUTES:
            return True
    for body in re.findall(r"#\s*\[\s*derive\s*\(([^)]*)\)", source):
        for derive in body.split(","):
            derive = derive.strip()
            if derive and derive.split("::")[-1] not in BUILTIN_DERIVES:
                return True
    return False


def rust_semantically_equal(old: str, new: str) -> bool:
    try:
        old_code, old_docs = lex(old)
        new_code, new_docs = lex(new)
    except LexError:
        return False
    if old_code != new_code:
        return False
    if old_docs != new_docs and (docs_can_matter(old) or docs_can_matter(new)):
        return False
    return True


def git(*args: str) -> bytes | None:
    try:
        result = subprocess.run(["git", *args], capture_output=True, check=False, timeout=GIT_TIMEOUT_SECONDS)
    except (OSError, subprocess.TimeoutExpired):
        return None
    return result.stdout if result.returncode == 0 else None


def ref_changes_game_code(base: str, ref: str) -> bool:
    merge_base = git("merge-base", base, ref)
    if merge_base is None:
        return True
    mb = merge_base.decode().strip()
    listing = git("diff", "--name-status", "--no-renames", "-z", mb, ref)
    if listing is None:
        return True
    fields = listing.decode("utf-8", "surrogateescape").split("\0")
    index = 0
    while index + 1 < len(fields):
        status, path = fields[index], fields[index + 1]
        index += 2
        if not is_game_path(path):
            continue
        if status != "M" or not path.endswith(".rs"):
            return True
        old = git("show", f"{mb}:{path}")
        new = git("show", f"{ref}:{path}")
        if old is None or new is None:
            return True
        try:
            if not rust_semantically_equal(old.decode("utf-8"), new.decode("utf-8")):
                return True
        except UnicodeDecodeError:
            return True
    return False


def selftest() -> int:
    base = (
        "/// Resolves the slot.\n"
        "#[must_use]\n"
        "pub fn slot(x: u32) -> u32 {\n"
        "    // the offset\n"
        "    let s = \"a // not a comment\";\n"
        "    let r = r#\"raw /* x */\"#;\n"
        "    let c = '/'; let l: &'static str = s;\n"
        "    x + 0x20 /* inline */\n"
        "}\n"
    )
    cases = [
        ("a changed line comment", base.replace("// the offset", "// the offset, measured"), True),
        ("an added comment line", base.replace("    // the offset\n", "    // the offset\n    // more\n"), True),
        ("a changed doc comment with builtin attributes only", base.replace("Resolves", "Finds"), True),
        ("a changed block comment", base.replace("/* inline */", "/* inline, too */"), True),
        ("a removed comment leaves the tokens apart", base.replace(" /* inline */", ""), True),
        ("blank lines added", base.replace("{\n", "{\n\n\n", 1), True),
        ("a comment between two words keeps them apart", base.replace("pub fn", "pub/* */fn"), True),
        ("words joined where a space was are not the same code", base.replace("pub fn", "pubfn"), False),
        ("a changed number", base.replace("0x20", "0x28"), False),
        ("`//` inside a string is data", base.replace("not a comment", "not a comment!"), False),
        ("`/* */` inside a raw string is data", base.replace("raw /* x */", "raw /* y */"), False),
        ("a char literal of `/` is data", base.replace("'/'", "'*'"), False),
        ("comment text moved into code", base.replace("// the offset", "the_offset();"), False),
        ("a changed attribute", base.replace("#[must_use]", "#[inline]"), False),
        ("an unterminated block comment does not lex", base.replace("/* inline */", "/* inline"), False),
        (
            "a doc comment under a non-builtin derive can reach a macro",
            base.replace("#[must_use]", "#[derive(clap::Parser)]"),
            None,
        ),
        (
            "a doc comment beside a meta-capturing macro can reach it",
            base + "macro_rules! m { ($(#[$m:meta])* $i:item) => {} }\n",
            None,
        ),
        ("a lifetime is not a char literal", base.replace("&'static str", "&'static  str"), True),
    ]
    bad = 0
    for name, new, want in cases:
        if want is None:
            # The doc-sensitive file on both sides, with only the doc text changed.
            got = rust_semantically_equal(new, new.replace("Resolves", "Finds"))
            want = False
        else:
            got = rust_semantically_equal(base, new)
        ok = got == want
        bad += 0 if ok else 1
        print(f"  {'ok  ' if ok else 'FAIL'} {name}: equal={got}" + ("" if ok else f" (want {want})"))
    print("cupcake_comment_only selftest: PASS" if not bad else f"cupcake_comment_only selftest: {bad} FAILED")
    return 1 if bad else 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--base", help="the upstream the ref is diffed against, e.g. origin/main")
    parser.add_argument("--ref", help="the pushed ref")
    parser.add_argument("--selftest", action="store_true")
    args = parser.parse_args()
    if args.selftest:
        return selftest()
    if not args.base or not args.ref:
        parser.error("--base and --ref are required")
    print(f"GAME {int(ref_changes_game_code(args.base, args.ref))}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
