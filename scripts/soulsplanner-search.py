#!/usr/bin/env python3
"""
soulsplanner.com build search.

The site has no search UI: builds are only reachable by id (/darksouls2/<id>)
or through the paginated public list (/builds/darksouls2/?p=N).  Everything
needed to filter is already on the wire though:

  * the list page gives id, name, author, level, rating, created/modified
  * every build page embeds the whole build as a JS literal:
        var plannerId='darksouls2', savedBuild={class_:'knight', ... };
  * the planner bundle carries the class table, so level can be recomputed
    from the stats (the list's own level column goes stale when a build is
    edited -- id 23005 lists as 12 and its stats say 150 -- so never trust it)

so we mirror the list, fetch the build pages, and query locally.

  index   mirror the public build list          -> index-<game>.json
  fetch   download savedBuild for indexed ids   -> builds-<game>.json
  query   filter the local cache
  export  emit one compact JSON payload for the browsable table page
  page    template + payload -> a self-contained HTML search page

Cache lives in ~/.cache/soulsplanner (override with --cache).
DS2 only: the DS1 and DS3 planners store gear as numeric item ids
('98000000;60501000;...') instead of names, so none of the filters below
would mean anything there without an item-id table.

  ./scripts/soulsplanner-search.py index
  ./scripts/soulsplanner-search.py fetch          # ~6.6k pages, run it once
  ./scripts/soulsplanner-search.py query --class knight --max-level 20
"""

import argparse
import json
import os
import re
import sys
import threading
import time
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from html import unescape

BASE = "https://soulsplanner.com"
UA = "Mozilla/5.0 (X11; Linux x86_64) soulsplanner-search/1.0"

STATS = [
    "vigor", "endurance", "vitality", "attunement", "strength",
    "dexterity", "adaptability", "intelligence", "faith",
]


def get(url, cookies=None, tries=3):
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    if cookies:
        req.add_header("Cookie", cookies)
    last = None
    for n in range(tries):
        try:
            with urllib.request.urlopen(req, timeout=20) as r:
                return r.read().decode("utf-8", "replace")
        except Exception as e:  # transient 5xx / reset
            last = e
            time.sleep(1.5 * (n + 1))
    raise last


def cache_path(a, kind):
    os.makedirs(a.cache, exist_ok=True)
    return os.path.join(a.cache, f"{kind}-{a.game}.json")


# ---------------------------------------------------------------- index

ROW_RE = re.compile(
    r'<td class="name"><a href="/[^/]+/(\d+)">(.*?)</a>.*?'
    r'<td class="author">(?:<a[^>]*>(.*?)</a>)?.*?'
    r'<td class="level">(\d+)</td>.*?'
    r'<td class="rating">(-?\d+)</td>.*?'
    r'<td class="created">(.*?)</td>.*?'
    r'<td class="modified">(.*?)</td>',
    re.S,
)
LAST_PAGE_RE = re.compile(r'page-navigator__last-page"><a href="[^"]*\?p=(\d+)"')


def parse_list(html):
    out = []
    for m in ROW_RE.finditer(html):
        bid, name, author, level, rating, created, modified = m.groups()
        out.append({
            "id": int(bid),
            "name": unescape(name).strip(),
            "author": unescape(author or "").strip(),
            "level": int(level),
            "rating": int(rating),
            "created": created.strip(),
            "modified": modified.strip(),
        })
    return out


def cmd_index(a):
    path = cache_path(a, "index")
    rows = {r["id"]: r for r in
            (json.load(open(path)) if os.path.exists(path) else [])}
    page, last = 1, None
    while True:
        html = get(f"{BASE}/builds/{a.game}/?p={page}")
        if last is None:
            m = LAST_PAGE_RE.search(html)
            last = int(m.group(1)) if m else page
        got = parse_list(html)
        if not got:
            break
        for r in got:
            rows[r["id"]] = r
        print(f"\rpage {page}/{last}  builds {len(rows)}",
              end="", file=sys.stderr)
        if page >= last:
            break
        page += 1
        time.sleep(a.delay)
    print(file=sys.stderr)
    out = sorted(rows.values(), key=lambda r: r["id"])
    json.dump(out, open(path, "w"), indent=1)
    print(f"{len(out)} builds indexed -> {path}")
    classes(a, refresh=True)


# ------------------------------------------------- class table (for level)

CLASSES_RE = re.compile(r"\w+\.classes=\{")
PLANNER_JS = {"darksouls2": "DarkSouls2/ds2planner.min.js"}


def classes(a, refresh=False):
    """{knight: {level, vigor, ...}} scraped from the planner bundle."""
    path = cache_path(a, "classes")
    if not refresh and os.path.exists(path):
        return json.load(open(path))
    js = get(f"{BASE}/public/scripts/release/{PLANNER_JS[a.game]}")
    m = CLASSES_RE.search(js)
    if not m:
        sys.exit("could not find the class table in the planner bundle")
    start = m.end() - 1
    depth, i = 0, start
    while True:  # brace-match the literal; it has no strings with braces
        depth += {"{": 1, "}": -1}.get(js[i], 0)
        i += 1
        if depth == 0:
            break
    # JS object literal with bare keys -> JSON
    src = re.sub(r"([{,])(\w+):", r'\1"\2":', js[start:i])
    out = json.loads(src)
    json.dump(out, open(path, "w"), indent=1)
    return out


def level_of(b, cls):
    """DS level = base level + every point spent past the class's base stats."""
    c = cls.get(b.get("class_"))
    if not c:
        return None
    return c["level"] + sum(b.get(s, 0) - c[s] for s in STATS)


# ---------------------------------------------------------------- fetch

SAVED_RE = re.compile(r"savedBuild=\{(.*?)\};", re.S)
KV_RE = re.compile(r"(\w+):('(?:[^'\\]|\\.)*'|-?\d+)")


def parse_build(html):
    m = SAVED_RE.search(html)
    if not m:
        return None
    d = {}
    for k, v in KV_RE.findall(m.group(1)):
        d[k] = int(v) if v[0] != "'" else v[1:-1].replace("\\'", "'")
    return d


def cmd_fetch(a):
    idx = json.load(open(cache_path(a, "index")))
    # the listed level is stale for edited builds, so widen the pre-filter and
    # let query decide on the recomputed level
    want = [r for r in idx
            if (a.min_level is None or r["level"] >= a.min_level)
            and (a.max_level is None or r["level"] <= a.max_level)]
    path = cache_path(a, "builds")
    cache = json.load(open(path)) if os.path.exists(path) else {}
    todo = [r for r in want if str(r["id"]) not in cache]
    print(f"{len(want)} in range, {len(todo)} to fetch at "
          f"{a.rate}/s over {a.jobs} connections", file=sys.stderr)
    done = [0]

    # one shared tap, so --jobs controls latency hiding and --rate controls
    # how hard someone else's server gets hit
    gate = threading.Lock()
    slot = [0.0]
    gap = 1.0 / a.rate if a.rate > 0 else 0.0

    def wait_turn():
        with gate:
            now = time.monotonic()
            slot[0] = max(now, slot[0]) + gap
            due = slot[0] - gap
        if due > now:
            time.sleep(due - now)

    def one(r):
        wait_turn()
        b = parse_build(get(f"{BASE}/{a.game}/{r['id']}"))
        done[0] += 1
        print(f"\r{done[0]}/{len(todo)}", end="", file=sys.stderr)
        return r["id"], b

    with ThreadPoolExecutor(max_workers=a.jobs) as ex:
        for n, (bid, b) in enumerate(ex.map(one, todo), 1):
            if b:
                cache[str(bid)] = b
            if n % 250 == 0:  # a killed crawl should not have to start over
                json.dump(cache, open(path, "w"), indent=1)
    print(file=sys.stderr)
    json.dump(cache, open(path, "w"), indent=1)
    print(f"{len(cache)} builds cached -> {path}")


# ---------------------------------------------------------------- query

def norm(s):
    return re.sub(r"[^a-z0-9]", "", s.lower())


def weapon_slots(b):
    """weapons is 'Name;Infusion' repeated over the 6 hand slots."""
    p = (b.get("weapons") or "").split(";")
    return list(zip(p[0::2], p[1::2]))


def cmd_query(a):
    idx = {r["id"]: r for r in json.load(open(cache_path(a, "index")))}
    builds = json.load(open(cache_path(a, "builds")))
    cls = classes(a)

    mins = {}
    for s in a.stat or []:
        k, _, v = s.partition(">=")
        k = k.strip().lower()
        if k not in STATS or not v.strip().isdigit():
            sys.exit(f"bad --stat {s!r}; use NAME>=N over {', '.join(STATS)}")
        mins[k] = int(v)

    hits = []
    for sid, b in builds.items():
        r = dict(idx.get(int(sid)) or {"id": int(sid), "name": "", "author": "",
                                       "rating": 0})
        lvl = level_of(b, cls)
        r["level"] = lvl if lvl is not None else r.get("level", 0)
        if a.name and norm(a.name) not in norm(r["name"]):
            continue
        if a.author and norm(a.author) not in norm(r["author"]):
            continue
        if a.class_ and norm(b.get("class_", "")) != norm(a.class_):
            continue
        if a.min_level is not None and r["level"] < a.min_level:
            continue
        if a.max_level is not None and r["level"] > a.max_level:
            continue
        if any(b.get(k, 0) < v for k, v in mins.items()):
            continue
        if a.covenant and norm(a.covenant) not in norm(b.get("covenant", "")):
            continue
        if a.armor and not any(norm(a.armor) in norm(p)
                               for p in (b.get("armor") or "").split(";")):
            continue
        ws = weapon_slots(b)
        if a.weapon and not any(norm(a.weapon) in norm(w) for w, _ in ws):
            continue
        # with --weapon, the infusion must be on that weapon, not just present
        if a.infusion and not any(
                norm(a.infusion) == norm(i)
                and (not a.weapon or norm(a.weapon) in norm(w))
                for w, i in ws):
            continue
        if a.ring and not any(norm(a.ring) in norm(p)
                              for p in (b.get("rings") or "").split(";")):
            continue
        if a.spell and not any(norm(a.spell) in norm(p)
                               for p in (b.get("spells") or "").split(";")):
            continue
        hits.append((r, b))

    hits.sort(key=lambda t: (t[0]["level"], -t[0]["rating"]))
    if a.json:
        print(json.dumps([dict(r, build=b) for r, b in hits], indent=1))
        return
    for r, b in hits:
        gear = ", ".join(f"{w}{'' if i in ('No_Infusion', '') else '/' + i}"
                         for w, i in weapon_slots(b)
                         if w and w != "Bare_Fists")
        print(f"{BASE}/{a.game}/{r['id']:<6} lvl{r['level']:>4} "
              f"{b.get('class_', '?'):<12} {r['name'][:32]:<32} "
              f"@{r['author'][:14]:<14} "
              + " ".join(f"{k[:3].upper()}{b[k]}" for k in STATS if k in b))
        if gear:
            print(f"    {gear}")
        if a.verbose:
            print(f"    armor {b.get('armor', '')}")
            print(f"    rings {b.get('rings', '')}")
    print(f"\n{len(hits)} match", file=sys.stderr)


# ---------------------------------------------------------------- export

ARMOR_SLOTS = ["head", "chest", "hands", "legs"]
HAND_SLOTS = ["lh1", "rh1", "lh2", "rh2", "lh3", "rh3"]


def cmd_export(a):
    """Columnar payload for the web table: gear names are interned, because
    6.5k builds repeat the same few hundred item names over and over."""
    idx = {r["id"]: r for r in json.load(open(cache_path(a, "index")))}
    builds = json.load(open(cache_path(a, "builds")))
    cls = classes(a)

    pool, seen = [], {}

    def intern(s):
        if s not in seen:
            seen[s] = len(pool)
            pool.append(s)
        return seen[s]

    def split(key, n, b):
        parts = (b.get(key) or "").split(";")
        parts += [""] * (n - len(parts))
        return [intern(p) for p in parts[:n]]

    rows = []
    for sid, b in builds.items():
        r = idx.get(int(sid))
        lvl = level_of(b, cls)
        if not r or lvl is None:
            continue
        rows.append([
            r["id"], r["name"], r["author"], lvl, r["rating"], r["created"],
            intern(b.get("class_", "")), intern(b.get("covenant", "")),
            split("armor", 4, b), split("weapons", 12, b),
            split("rings", 4, b), split("spells", 14, b),
            [b.get(s, 0) for s in STATS],
        ])
    rows.sort(key=lambda r: -r[3])

    levels = [r[3] for r in rows] or [0]
    out = {
        "meta": {
            "listed": len(idx),
            "mirrored": len(rows),
            "partial": len(rows) < len(idx),
            "minLevel": min(levels),
            "maxLevel": max(levels),
        },
        "pool": pool,
        "rows": rows,
        "stats": STATS,
        "armorSlots": ARMOR_SLOTS,
        "handSlots": HAND_SLOTS,
        "classes": {k: v["name"] for k, v in cls.items()},
    }
    dest = a.out or os.path.join(a.cache, f"payload-{a.game}.json")
    json.dump(out, open(dest, "w"), separators=(",", ":"))
    print(f"{len(rows)} builds, {len(pool)} interned names, "
          f"{os.path.getsize(dest) / 1e6:.2f} MB -> {dest}")
    return out


def cmd_page(a):
    """Template + payload -> one self-contained page, no runtime fetches."""
    a.out = None
    payload = json.dumps(cmd_export(a), separators=(",", ":"))
    tpl = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                       "soulsplanner-page.template.html")
    html = open(tpl).read()
    if "__PAYLOAD__" not in html:
        sys.exit(f"{tpl} has no __PAYLOAD__ placeholder")
    # the payload sits in a <script type=application/json>, so the only
    # sequence that could break out of it is a literal </script
    html = html.replace("__PAYLOAD__", payload.replace("</", "<\\/"))
    open(a.page_out, "w").write(html)
    print(f"{os.path.getsize(a.page_out) / 1e6:.2f} MB -> {a.page_out}")


# ---------------------------------------------------------------- cli

p = argparse.ArgumentParser(
    description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
p.add_argument("--game", default="darksouls2", choices=["darksouls2"])
p.add_argument("--cache", default=os.path.expanduser(
    os.environ.get("XDG_CACHE_HOME", "~/.cache") + "/soulsplanner"))
sub = p.add_subparsers(dest="cmd", required=True)

pi = sub.add_parser("index", help="mirror the public build list")
pi.add_argument("--delay", type=float, default=0.2)
pi.set_defaults(fn=cmd_index)

pf = sub.add_parser("fetch", help="download build details for indexed ids")
pf.add_argument("--min-level", type=int)
pf.add_argument("--max-level", type=int)
pf.add_argument("--jobs", type=int, default=4, help="concurrent connections")
pf.add_argument("--rate", type=float, default=2.0,
                help="requests per second across all jobs (0 = uncapped)")
pf.set_defaults(fn=cmd_fetch)

pq = sub.add_parser("query", help="filter the local cache")
pq.add_argument("--name", help="substring of the build name")
pq.add_argument("--author", help="substring of the author name")
pq.add_argument("--class", dest="class_", help="starting class, e.g. knight")
pq.add_argument("--min-level", type=int)
pq.add_argument("--max-level", type=int)
pq.add_argument("--stat", action="append", metavar="NAME>=N",
                help="repeatable, e.g. --stat strength>=40")
pq.add_argument("--armor", help="substring of any armor piece")
pq.add_argument("--weapon", help="substring of any weapon")
pq.add_argument("--infusion", help="exact infusion, e.g. Lightning")
pq.add_argument("--ring")
pq.add_argument("--spell")
pq.add_argument("--covenant")
pq.add_argument("-v", "--verbose", action="store_true")
pq.add_argument("--json", action="store_true")
pq.set_defaults(fn=cmd_query)

pe = sub.add_parser("export", help="compact JSON payload for the table page")
pe.add_argument("--out", help="destination file (default: in the cache dir)")
pe.set_defaults(fn=cmd_export)

pp = sub.add_parser("page", help="build the self-contained search page")
pp.add_argument("page_out", metavar="OUT.html")
pp.set_defaults(fn=cmd_page)

a = p.parse_args()
a.fn(a)
