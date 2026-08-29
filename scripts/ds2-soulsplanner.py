#!/usr/bin/env python3
"""Fetch a soulsplanner.com Dark Souls 2 build.

The build id must be in the URL *path* (/darksouls2/253), not the fragment
(/darksouls2/#253). The fragment is never sent to the server and the planner's
JS never reads location.hash, so the hash form always renders the empty
bootstrap script `;var plannerId='darksouls2';` no matter how long you wait.
With the path form the server inlines `savedBuild={...}` into the first <body>
<script>, so a plain HTTP GET is enough -- no headless browser, no waiting.
"""

import json
import re
import sys
import urllib.request

BUILD_ID = 253
BASE = "https://soulsplanner.com/darksouls2"

BODY_SCRIPT = re.compile(r"<body>\s*<script>(.*?)</script>", re.S)
SAVED_BUILD = re.compile(r"savedBuild\s*=\s*\{(.*?)\}\s*;", re.S)
# JS object literal here is flat: bareword key, then a single-quoted string or an int.
FIELD = re.compile(r"(\w+)\s*:\s*(?:'([^']*)'|(-?\d+))")

# Fields that hold ';'-delimited lists.
LIST_FIELDS = ("armor", "weapons", "rings", "spells", "items")


def fetch(build_id, timeout=30):
    url = f"{BASE}/{build_id}"
    req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return resp.read().decode("utf-8", "replace")


def body_script(html):
    m = BODY_SCRIPT.search(html)
    if not m:
        raise ValueError("no <body><script> found")
    return m.group(1)


def parse_build(script):
    m = SAVED_BUILD.search(script)
    if not m:
        raise ValueError(
            "savedBuild missing -- the response only has the bootstrap script. "
            "Use the path form /darksouls2/<id>, not /darksouls2/#<id>, and "
            "check the build id exists and is public."
        )
    build = {}
    for key, text, num in FIELD.findall(m.group(1)):
        build[key] = int(num) if num else text
    for key in LIST_FIELDS:
        if key in build:
            build[key] = build[key].split(";")
    return build


def main(argv):
    build_id = int(argv[1]) if len(argv) > 1 else BUILD_ID
    build = parse_build(body_script(fetch(build_id)))
    json.dump({"id": build_id, **build}, sys.stdout, indent=2)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main(sys.argv)
