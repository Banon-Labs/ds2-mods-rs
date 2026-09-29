#!/usr/bin/env python3
"""MugenMonkey's Dark Souls 2 gear ids, read out of the planner's own JavaScript.

    python3 scripts/ds2-mugenmonkey-tables.py                   # TSV on stdout
    python3 scripts/ds2-mugenmonkey-tables.py --out table.tsv   # TSV to a file, tallies on stderr
    python3 scripts/ds2-mugenmonkey-tables.py --js saved.js     # from a saved bundle, no network
    python3 scripts/ds2-mugenmonkey-tables.py --selftest

Output: `kind <TAB> id <TAB> name` for kind in weapon, head, chest, hands, legs, ring, spell, item.

Why this exists
---------------

A MugenMonkey build page (`https://mugenmonkey.com/darksouls2/<id>`) inlines the build as JSON in
`gon.savedStats={...};`, but its gear is numbers: `"rh1":281,"head":398,"ring1":25`. The numbers
are the planner's own, not the game's item ids, and the only place they are joined to names is the
planner's bundle, `/assets/ds2application-<hash>.js`, which assigns five tables:

    window.darkSouls2WeaponDetails={schema:[..., "id", ...], "Dagger":[...], ...}
    window.darkSouls2Armor={schema:[..., "type", "id"], ...}
    window.darkSouls2Rings={schema:["id", ...], ...}
    window.darkSouls2Spells={schema:[..., "id"], ...}
    window.darkSouls2Items={schema:["id"], ...}

Each table names its own `id` column in its `schema`, and the column is in a different place in
each, so the column is read from the schema rather than assumed. The names are joined to the
game's item ids later, by `ds2_build_import_core::id_for`, the same way a soulsplanner name is.

How the literals are read
-------------------------

Each table is a JavaScript object literal (bareword keys, nested objects), not JSON. Its text is cut
out by bracket matching that respects string quotes, and handed to `node` to evaluate inside
`vm.runInNewContext` with an empty sandbox. Only the literal is evaluated, never the bundle.
"""

import argparse
import json
import re
import subprocess
import sys
import urllib.request

SITE = "https://mugenmonkey.com"
# Any public build page names the current bundle; this is one that was public on 2026-09-29.
PAGE = SITE + "/darksouls2/181062"
USER_AGENT = "ds2-mods-rs"

# window.<table> -> the kind written to the TSV.
TABLES = {
    "darkSouls2WeaponDetails": "weapon",
    "darkSouls2Armor": "armor",
    "darkSouls2Rings": "ring",
    "darkSouls2Spells": "spell",
    "darkSouls2Items": "item",
}

NODE_EVAL = r"""
const vm = require("vm");
let text = "";
process.stdin.on("data", (chunk) => (text += chunk));
process.stdin.on("end", () => {
  const value = vm.runInNewContext("(" + text + ")", Object.create(null), { timeout: 2000 });
  process.stdout.write(JSON.stringify(value));
});
"""


def fetch(url):
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(request, timeout=30) as response:
        return response.read().decode("utf-8")


def bundle_url(page):
    match = re.search(r'src="(/assets/ds2application-[0-9a-f]+\.js)"', page)
    if not match:
        sys.exit("no ds2application bundle named on the page -- the site changed shape")
    return SITE + match.group(1)


def literal_at(js, start):
    """The `{...}` literal opening at `start`, bracket-matched with string quotes respected."""
    if js[start] != "{":
        raise ValueError(f"no literal at {start}")
    depth = 0
    quote = None
    index = start
    while index < len(js):
        char = js[index]
        if quote:
            if char == "\\":
                index += 2
                continue
            if char == quote:
                quote = None
        elif char in "\"'":
            quote = char
        elif char in "{[":
            depth += 1
        elif char in "}]":
            depth -= 1
            if depth == 0:
                return js[start : index + 1]
        index += 1
    raise ValueError(f"unterminated literal at {start}")


def evaluate(literal):
    result = subprocess.run(
        ["node", "-e", NODE_EVAL],
        input=literal,
        capture_output=True,
        text=True,
        check=True,
        timeout=30,
    )
    return json.loads(result.stdout)


def tables(js):
    """Yield (kind, id, name) for every row of every table, in table order."""
    for table, kind in TABLES.items():
        marker = f"window.{table}="
        at = js.find(marker)
        if at < 0:
            sys.exit(f"{marker} not in the bundle -- the site changed shape")
        value = evaluate(literal_at(js, at + len(marker)))
        schema = value.pop("schema")
        column = schema.index("id")
        # Armor ids are looked up per slot -- the planner's `setArmors` splits the table into
        # `headPieces`, `chestPieces`, ... by this column, and `setSavedArmor` searches only the
        # slot's own list -- so an armor row's kind is its slot.
        slot = schema.index("type") if kind == "armor" else None
        for name, row in value.items():
            yield (row[slot] if slot is not None else kind), int(row[column]), name.strip()


def selftest():
    js = (
        'x=1;window.darkSouls2Rings={schema:["id","weight","effects"],'
        '" No Ring":[0,0,""],"Ring of Blades":[44,2,[{alters:"AR",effect:"a } b",value:50}]]},'
        'window.darkSouls2Spells={schema:["type","slots","id"],Hush:["Sorcery",1,63]};'
    )
    assert literal_at(js, js.index("{schema")).endswith('value:50}]]}')
    rows = []
    for table, kind in (("darkSouls2Rings", "ring"), ("darkSouls2Spells", "spell")):
        at = js.index(f"window.{table}=") + len(f"window.{table}=")
        value = evaluate(literal_at(js, at))
        column = value.pop("schema").index("id")
        rows += [(kind, row[column], name.strip()) for name, row in value.items()]
    assert rows == [("ring", 0, "No Ring"), ("ring", 44, "Ring of Blades"), ("spell", 63, "Hush")], rows
    print("selftest: ok")


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--out", help="write the TSV here instead of stdout")
    parser.add_argument("--js", help="read a saved ds2application bundle instead of fetching")
    parser.add_argument("--selftest", action="store_true")
    args = parser.parse_args()
    if args.selftest:
        return selftest()

    if args.js:
        with open(args.js, encoding="utf-8") as handle:
            js = handle.read()
        source = args.js
    else:
        source = bundle_url(fetch(PAGE))
        js = fetch(source)

    rows = list(tables(js))
    lines = [
        "# MugenMonkey Dark Souls 2 planner ids. kind\tid\tname, tab-separated, in table order.",
        "# Armor rows carry their slot (head, chest, hands, legs) as the kind; id 0 is that slot empty.",
        f"# Extracted by scripts/ds2-mugenmonkey-tables.py from {source.rsplit('/', 1)[-1]}",
        "# The ids are the planner's own, not the game's. Names join to game ids through",
        "# ds2_build_import_core::id_for, like a soulsplanner name.",
    ]
    lines += [f"{kind}\t{ident}\t{name}" for kind, ident, name in rows]
    text = "\n".join(lines) + "\n"
    if args.out:
        with open(args.out, "w", encoding="utf-8") as handle:
            handle.write(text)
        for kind in dict.fromkeys(row[0] for row in rows):
            print(f"{kind}: {sum(1 for row in rows if row[0] == kind)}", file=sys.stderr)
    else:
        sys.stdout.write(text)


if __name__ == "__main__":
    main()
