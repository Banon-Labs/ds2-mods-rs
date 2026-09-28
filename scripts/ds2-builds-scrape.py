#!/usr/bin/env python3
"""Mirror MugenMonkey's public DARK SOULS II builds to ~/.cache/mugenmonkey.

    python3 scripts/ds2-builds-scrape.py            # list, then fetch every listed build
    python3 scripts/ds2-builds-scrape.py --list-only

Two JSON endpoints, both found in the site's own `ds2application-*.js` / by probing:

    /api/v0/ds2_builds?per_page=200&page=N   public build index: {count, ds2_builds: {id: meta}}
                                             (per_page is capped at 200 server-side)
    /darksouls2/<id>.json                    one build: the same object the page inlines as
                                             gon.savedStats (slots, stats, soulMemory, two_hand)

Output, both append-only JSON lines so an interrupted run resumes where it stopped:

    index.jsonl    one listed build's metadata per line (id, title, level, votes, dates)
    builds.jsonl   {"id": ..., "build": <savedStats>} per line

Rate: `--workers` concurrent requests (default 4) with `--delay` seconds between each worker's
requests. A small hobby site; keep it gentle.
"""

from __future__ import annotations

import argparse
import json
import sys
import threading
import time
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

BASE = "https://mugenmonkey.com"
UA = {"User-Agent": "ds2-mods-rs build research (single mirror, rate-limited)"}


def get_json(url: str, tries: int = 4):
    for n in range(tries):
        try:
            req = urllib.request.Request(url, headers=UA)
            with urllib.request.urlopen(req, timeout=30) as r:
                return json.loads(r.read())
        except urllib.error.HTTPError as e:
            if e.code == 404:
                return None
            err = e
            if e.code == 429:  # the site rate-limits; back off hard, as long as it asks
                time.sleep(float(e.headers.get("Retry-After") or 60))
                continue
        except Exception as e:  # noqa: BLE001 -- network hiccups retry, the last one is raised
            err = e
        time.sleep(2 ** n)
    raise RuntimeError(f"{url}: {err}")


def list_builds(out: Path, delay: float) -> list[str]:
    path = out / "index.jsonl"
    seen = {}
    if path.exists():
        for line in path.read_text().splitlines():
            m = json.loads(line)
            seen[m["id"]] = m
    first = get_json(f"{BASE}/api/v0/ds2_builds?per_page=200&page=1")
    pages = -(-first["count"] // 200)
    print(f"index: {first['count']} public builds, {pages} pages; {len(seen)} already listed", flush=True)
    with path.open("a") as f:
        for p in range(1, pages + 1):
            d = first if p == 1 else get_json(f"{BASE}/api/v0/ds2_builds?per_page=200&page={p}")
            new = 0
            for bid, m in (d or {}).get("ds2_builds", {}).items():
                if bid not in seen:
                    seen[bid] = m
                    f.write(json.dumps(m) + "\n")
                    new += 1
            if p % 25 == 0 or p == pages:
                print(f"  page {p}/{pages}  listed {len(seen)}", flush=True)
            if p > 1 and new == 0 and p > 5:
                pass  # pages shift as builds are added; keep going rather than stop early
            time.sleep(delay)
    return list(seen)


def fetch_builds(out: Path, ids: list[str], workers: int, delay: float) -> None:
    path = out / "builds.jsonl"
    done = set()
    if path.exists():
        for line in path.read_text().splitlines():
            done.add(json.loads(line)["id"])
    todo = [i for i in ids if i not in done]
    print(f"builds: {len(done)} cached, {len(todo)} to fetch", flush=True)
    lock = threading.Lock()
    counter = {"ok": 0, "missing": 0, "failed": 0}
    t0 = time.time()

    with path.open("a") as f:
        def one(bid: str):
            try:
                b = get_json(f"{BASE}/darksouls2/{bid}.json")
            except RuntimeError as e:
                with lock:
                    counter["failed"] += 1
                    print(f"  FAILED {e}", file=sys.stderr, flush=True)
                return
            with lock:
                if b is None:
                    counter["missing"] += 1
                else:
                    f.write(json.dumps({"id": bid, "build": b}) + "\n")
                    counter["ok"] += 1
                n = sum(counter.values())
                if n % 1000 == 0:
                    f.flush()
                    rate = n / (time.time() - t0)
                    print(f"  {n}/{len(todo)}  {counter}  {rate:.1f}/s  eta {(len(todo) - n) / rate / 60:.0f} min",
                          flush=True)
            time.sleep(delay)

        with ThreadPoolExecutor(workers) as ex:
            list(ex.map(one, todo))
    print(f"done: {counter}", flush=True)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--out", type=Path, default=Path.home() / ".cache/mugenmonkey")
    ap.add_argument("--workers", type=int, default=1)
    ap.add_argument("--delay", type=float, default=0.5)
    ap.add_argument("--list-only", action="store_true")
    a = ap.parse_args()
    a.out.mkdir(parents=True, exist_ok=True)
    ids = list_builds(a.out, a.delay)
    if not a.list_only:
        fetch_builds(a.out, ids, a.workers, a.delay)
    return 0


if __name__ == "__main__":
    sys.exit(main())
