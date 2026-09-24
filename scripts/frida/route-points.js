// The polyline `ds2-invasion-path` is actually laying stones along, point by point.
//
// THE QUESTION. The player reports a trail that jumps over terrain, piles duplicates at its end,
// and never reaches the character it is routing to. The crate's log prints the route's node IDS
// and their attributes; it never prints the POSITIONS it hands to the trail. Those are what the
// stones are placed on, so they are what has to be read.
//
// This re-implements `navpath::decode` against live memory -- segments walked in reverse because
// a route is stored goal-first, points inside each segment reversed too -- and prints every point
// with its distance from the previous one and from the goal. Three things become visible that the
// id log cannot show:
//
//   - a gap between consecutive points, which is a jump over terrain;
//   - repeated points, which is a stone laid where one already is;
//   - the distance from the LAST point to the character, which is how far short the trail stops.
//
// It reads and prints only. No hook, no timer (`frida-js-thread-wedges-in-ds2`), one message.

'use strict';

const GAME_MANAGER_IMP = 0x016148f0;
const PLAYER_CTRL_OFFSET = 0xd0;
const CHARACTER_MANAGER_OFFSET = 0x18;
const ENTITY_BEGIN_OFFSET = 0x10;
const ENTITY_END_OFFSET = 0x18;
const POSITION_OFFSET = 0x90;
const PLAYER_CTRL_VTABLE = 0x010e4bb8;
const NAV_SYSTEM_OFFSET = 0xbc0; // GameManagerImp -> NvNavigationSystem

// NvRoutePlanner / NvRoute, all from ds2-rva.
const PLANNER_ROUTE_OFFSET = 0x48;
const ROUTE_SEGMENTS_OFFSET = 0x10;
const ROUTE_SEGMENT_COUNT_OFFSET = 0x18;
const SEGMENT_STRIDE = 0x60;
const SEGMENT_POINT_OFFSET = 0x20; // the segment's own point, used when it has no polyline
const SEGMENT_POINTS_OFFSET = 0x40; // -> array of 16-byte points
const SEGMENT_POINT_COUNT_OFFSET = 0x50; // i16
const POINT_STRIDE = 16;
const MAX_SEGMENTS = 4096;

// The planners this crate creates are linked onto the navigation system's intrusive update list.
// Walking it is how their addresses are found without the crate having to tell us -- the log
// prints only the first one, and there is one per target now.
//
// Both offsets are `ds2-rva`'s, not invented here: `NV_NAVIGATION_SYSTEM_LIST_HEAD_OFFSET` is
// +0x20 (`0x140bae8d0` pushes onto the front) and `NV_NAVIGATION_NODE_NEXT_OFFSET` is +0x10
// (`NvNavigationSystem::Update` walks `puVar12[2]`, the factory writes `plVar3[2]`, same field).
// A first pass at this guessed 0x10/0x8 and found nothing, which is what guessing buys.
const NAV_LIST_HEAD_OFFSET = 0x20;
const NAV_LIST_NEXT_OFFSET = 0x10;
const MAX_LIST_WALK = 256;

function ptr_(at) {
  try {
    return at.readPointer();
  } catch (_) {
    return null;
  }
}

function point(at) {
  try {
    const raw = new Float32Array(at.readByteArray(12));
    if (!isFinite(raw[0]) || !isFinite(raw[1]) || !isFinite(raw[2])) {
      return null;
    }
    return [raw[0], raw[1], raw[2]];
  } catch (_) {
    return null;
  }
}

function dist(a, b) {
  if (!a || !b) {
    return null;
  }
  const dx = a[0] - b[0];
  const dy = a[1] - b[1];
  const dz = a[2] - b[2];
  return Math.sqrt(dx * dx + dy * dy + dz * dz);
}

// `navpath::decode`, against live memory. Goal-first segments walked in reverse, and the points
// inside each segment reversed too -- both reversals, because applying one produces a polyline
// that is plausible, connected and inside out.
function decode(route) {
  const segments = ptr_(route.add(ROUTE_SEGMENTS_OFFSET));
  let count;
  try {
    count = route.add(ROUTE_SEGMENT_COUNT_OFFSET).readS32();
  } catch (_) {
    return null;
  }
  if (segments === null || segments.isNull() || count <= 0 || count > MAX_SEGMENTS) {
    return null;
  }
  const out = [];
  for (let index = count - 1; index >= 0; index -= 1) {
    const segment = segments.add(index * SEGMENT_STRIDE);
    let points;
    try {
      points = segment.add(SEGMENT_POINT_COUNT_OFFSET).readS16();
    } catch (_) {
      return null;
    }
    if (points <= 0) {
      const only = point(segment.add(SEGMENT_POINT_OFFSET));
      if (only !== null) {
        // A lone portal. `ground: false` -- nothing expanded the span that reaches it, which is
        // the whole distinction `navpath::RoutePoint` carries and this mirrors.
        out.push({ at: only, segment: index, slot: -1, ground: false });
      }
      continue;
    }
    const array = ptr_(segment.add(SEGMENT_POINTS_OFFSET));
    if (array === null || array.isNull()) {
      continue;
    }
    for (let slot = points - 1; slot >= 0; slot -= 1) {
      const found = point(array.add(slot * POINT_STRIDE));
      if (found !== null) {
        // The first point of a segment is reached from the previous segment's last point, and
        // nothing expanded THAT span.
        out.push({ at: found, segment: index, slot: slot, ground: slot !== points - 1 });
      }
    }
  }
  return out;
}

function main() {
  const module = Process.findModuleByName('DarkSoulsII.exe');
  if (module === null) {
    send({ error: 'DarkSoulsII.exe is not loaded' });
    return;
  }
  const base = module.base;
  const manager = ptr_(base.add(GAME_MANAGER_IMP));
  if (manager === null || manager.isNull()) {
    send({ error: 'GameManagerImp is null' });
    return;
  }
  const local = ptr_(manager.add(PLAYER_CTRL_OFFSET));
  const localAt = local === null || local.isNull() ? null : point(local.add(POSITION_OFFSET));

  // Every remote PlayerCtrl and where it is, so a route's end can be measured against the thing
  // it was planned towards rather than against a guess.
  const targets = [];
  const characters = ptr_(manager.add(CHARACTER_MANAGER_OFFSET));
  if (characters !== null && !characters.isNull()) {
    const begin = ptr_(characters.add(ENTITY_BEGIN_OFFSET));
    const end = ptr_(characters.add(ENTITY_END_OFFSET));
    if (begin !== null && end !== null && !begin.isNull()) {
      const span = end.sub(begin).toInt32();
      if (span > 0 && span % 8 === 0 && span / 8 < 8192) {
        for (let i = 0; i < span / 8; i += 1) {
          const chr = ptr_(begin.add(i * 8));
          if (chr === null || chr.isNull()) {
            continue;
          }
          const vtable = ptr_(chr);
          if (vtable === null || !vtable.equals(base.add(PLAYER_CTRL_VTABLE))) {
            continue;
          }
          if (local !== null && chr.equals(local)) {
            continue;
          }
          let name = null;
          try {
            const slot = chr.add(0x118).readPointer();
            name = slot.isNull() ? null : slot.readUtf16String(24);
          } catch (_) {
            name = null;
          }
          targets.push({ ctrl: chr.toString(), name: name, at: point(chr.add(POSITION_OFFSET)) });
        }
      }
    }
  }

  // The planners on the navigation system's list. The crate makes one per target; the engine's
  // own AI agents are on the same list, so each candidate is only reported if it holds a route
  // that decodes -- an object that is not an NvRoutePlanner will not.
  const navSystem = ptr_(manager.add(NAV_SYSTEM_OFFSET));
  const routes = [];
  if (navSystem !== null && !navSystem.isNull()) {
    let node = ptr_(navSystem.add(NAV_LIST_HEAD_OFFSET));
    let walked = 0;
    while (node !== null && !node.isNull() && walked < MAX_LIST_WALK) {
      walked += 1;
      const route = ptr_(node.add(PLANNER_ROUTE_OFFSET));
      if (route !== null && !route.isNull()) {
        const points = decode(node.add(PLANNER_ROUTE_OFFSET));
        if (points !== null && points.length >= 2) {
          routes.push({ planner: node.toString(), points: points });
        }
      }
      node = ptr_(node.add(NAV_LIST_NEXT_OFFSET));
    }
  }

  send({ local: localAt, targets: targets, routes: routes });
}

main();
