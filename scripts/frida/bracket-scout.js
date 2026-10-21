// Map every soul level to DARK SOULS II Seamless Co-op's invasion bracket, without invading anyone.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/bracket-scout.js`
// in the player's own world, not hosting, standing still, no menu open.
//
// # How
//
// For each probe level: set the nine stats so the game's own level (sum - 53, measured) is that
// level (stats first, then +0xD0, as set-soul-level.js), use the Fragment of Malevolance by writing
// the belt's use request (as use-item.js), and read the bracket value Seamless's search filters on
// (`..._bimm_<N>_6`, the AddRequestLobbyListStringFilter on the bracket key). The probes bisect
// between levels whose brackets differ until every boundary is two adjacent levels.
//
// # Why nobody is invaded
//
// On RequestLobbyList (ISteamMatchmaking v009 slot 4), on the game thread, before Steam sees the
// request, this adds one more string filter that no lobby carries. Steam returns no lobby, so
// Seamless's own search fails ("Failed to find another world to invade") and nothing is joined.
// The owner's rule (2026-10-21): an agent's invasion must never sit in a real player's world. If a
// JoinLobby is ever seen while this runs, the scout stops and uses the Obscuring Stone.
//
// At the end, or on dispose, the original stats and level are written back.

'use strict';

const FRAGMENT_OF_MALEVOLANCE = 100000002; // lead from the locale order; the run proves or refutes it
const OBSCURING_STONE = 100000003;
const LEVEL_MIN = 1;
const LEVEL_MAX = 838; // nine stats at 99, minus 53
const BRACKET_KEY = '8c1e0aea8218b194b8598fc57ec01ccf4c5444e3522d367f1f8ec903f1d12ae5';
const VOID_KEY = 'ds2_mods_bracket_scout_no_lobby_has_this';
const SEARCH_QUIET_MS = 6000; // no new RequestLobbyList for this long: the search has given up
const PROBE_TIMEOUT_MS = 30000;
const REUSE_EVERY_MS = 4000;
const BETWEEN_PROBES_MS = 4000;
const LEVEL_SUM_OFFSET = 53;

const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const GAME_MANAGER_IMP = 0x016148f0;
const PLAYER_PARAM_GET = 0x001ab660;
const STAT_OFFSETS = [0x08, 0x0a, 0x0c, 0x0e, 0x10, 0x12, 0x14, 0x16, 0x18];
const LEVEL_OFFSET = 0xd0;
const PLAYER_CTRL_OFFSET = 0xd0;
const USE_REQUEST_OFFSET = 0xc8;
const REQUEST_HANDLE_OFFSET = 0x40;
const REQUEST_COUNT_OFFSET = 0x42;
const REQUEST_FLAGS_OFFSET = 0xfc;
const REQUEST_USE_BIT = 0x800;
const GAME_DATA_MANAGER_OFFSET = 0xa8;
const ITEM_INVENTORY_OFFSET = 0x10;
const ITEM_BAG_LIST_OFFSET = 0x10;
const ITEM_ENTRY_ARRAY_OFFSET = 0x28;
const ITEM_ENTRY_STRIDE = 0x28;
const ITEM_ENTRY_COUNT = 3840;
const ITEM_ENTRY_ITEM_ID_OFFSET = 0x14;
const ITEM_ENTRY_HANDLE_OFFSET = 0x1c;

const steam = Process.getModuleByName('steam_api64.dll');
const iface = new NativeFunction(steam.getExportByName('SteamMatchmaking'), 'pointer', [])();
const vtable = iface.readPointer();
const slot = (n) => vtable.add(n * Process.pointerSize).readPointer();
const addStringFilter = new NativeFunction(slot(5), 'void', ['pointer', 'pointer', 'pointer', 'int']);
const voidKey = Memory.allocUtf8String(VOID_KEY);
const voidValue = Memory.allocUtf8String('1');
const getParam = new NativeFunction(exe.add(PLAYER_PARAM_GET), 'pointer', []);

let lastBracket = null;
let lastSearchAt = 0;
let searches = 0;
let joins = 0;
let stopped = false;

Interceptor.attach(slot(5), {
  onEnter(args) {
    try {
      if (args[1].readUtf8String() === BRACKET_KEY) lastBracket = args[2].readUtf8String();
    } catch (e) {
      /* a filter we cannot read is not ours to record */
    }
  },
});
Interceptor.attach(slot(4), {
  onEnter(args) {
    if (stopped) return;
    addStringFilter(args[0], voidKey, voidValue, 0);
    searches += 1;
    lastSearchAt = Date.now();
  },
});
Interceptor.attach(slot(14), {
  onEnter() {
    joins += 1;
  },
});

function rp(p) {
  return p.readPointer();
}

function handleOf(goods) {
  let at = rp(rp(exe.add(GAME_MANAGER_IMP)).add(GAME_DATA_MANAGER_OFFSET));
  at = rp(rp(rp(at.add(ITEM_INVENTORY_OFFSET)).add(ITEM_BAG_LIST_OFFSET)).add(ITEM_BAG_LIST_OFFSET));
  const base = at.add(ITEM_ENTRY_ARRAY_OFFSET);
  for (let i = 0; i < ITEM_ENTRY_COUNT; i++) {
    const e = base.add(i * ITEM_ENTRY_STRIDE);
    if (e.add(ITEM_ENTRY_ITEM_ID_OFFSET).readU32() === goods) return e.add(ITEM_ENTRY_HANDLE_OFFSET).readU16();
  }
  return null;
}

// ChrItemCtrl = [[PlayerCtrl + 0xE0 (PlayerActionCtrl)] + 0x170]; its state at +0x10 is 0 when idle,
// the only state that takes a request (static RE 2026-10-21).
const PLAYER_ACTION_CTRL_OFFSET = 0xe0;
const CHR_ITEM_CTRL_OFFSET = 0x170;
const CHR_ITEM_CTRL_STATE_OFFSET = 0x10;

function useState() {
  const ctrl = rp(rp(exe.add(GAME_MANAGER_IMP)).add(PLAYER_CTRL_OFFSET));
  const req = rp(ctrl.add(USE_REQUEST_OFFSET));
  const item = rp(rp(ctrl.add(PLAYER_ACTION_CTRL_OFFSET)).add(CHR_ITEM_CTRL_OFFSET));
  return {
    pending: (req.add(REQUEST_FLAGS_OFFSET).readU32() & REQUEST_USE_BIT) !== 0,
    handle: req.add(REQUEST_HANDLE_OFFSET).readU16(),
    item_ctrl_state: item.add(CHR_ITEM_CTRL_STATE_OFFSET).readU32(),
  };
}

function use(goods) {
  const handle = handleOf(goods);
  if (handle === null) return false;
  const ctrl = rp(rp(exe.add(GAME_MANAGER_IMP)).add(PLAYER_CTRL_OFFSET));
  const req = rp(ctrl.add(USE_REQUEST_OFFSET));
  req.add(REQUEST_HANDLE_OFFSET).writeU16(handle);
  req.add(REQUEST_COUNT_OFFSET).writeU16(1);
  req.add(REQUEST_FLAGS_OFFSET).writeU32(req.add(REQUEST_FLAGS_OFFSET).readU32() | REQUEST_USE_BIT);
  return true;
}

const param = getParam();
// The character as it was before any write this session (set-soul-level.js's first read,
// 2026-10-21): vgr end vit att str dex int fth adp, level 11.
const original = { stats: [6, 6, 6, 10, 6, 11, 7, 6, 6], level: 11 };
const atAttach = { stats: STAT_OFFSETS.map((o) => param.add(o).readU16()), level: param.add(LEVEL_OFFSET).readU32() };

function setLevel(level) {
  // Fill from vigor upward, each stat 1..99, so the sum is level + 53.
  let left = level + LEVEL_SUM_OFFSET;
  const stats = STAT_OFFSETS.map((_, i) => {
    const rest = STAT_OFFSETS.length - i - 1;
    const v = Math.max(1, Math.min(99, left - rest));
    left -= v;
    return v;
  });
  stats.forEach((v, i) => param.add(STAT_OFFSETS[i]).writeU16(v));
  param.add(LEVEL_OFFSET).writeU32(level);
}

function restore() {
  original.stats.forEach((v, i) => param.add(STAT_OFFSETS[i]).writeU16(v));
  param.add(LEVEL_OFFSET).writeU32(original.level);
}

const known = new Map(); // level -> bracket number

function nextProbe() {
  if (!known.has(LEVEL_MIN)) return LEVEL_MIN;
  if (!known.has(LEVEL_MAX)) return LEVEL_MAX;
  const levels = [...known.keys()].sort((a, b) => a - b);
  for (let i = 0; i + 1 < levels.length; i++) {
    const a = levels[i];
    const b = levels[i + 1];
    if (known.get(a) !== known.get(b) && b - a > 1) return Math.floor((a + b) / 2);
  }
  return null;
}

function finish(reason) {
  stopped = true;
  restore();
  const levels = [...known.keys()].sort((a, b) => a - b);
  const boundaries = [];
  for (let i = 0; i + 1 < levels.length; i++) {
    if (known.get(levels[i]) !== known.get(levels[i + 1])) {
      boundaries.push({ last: levels[i], bracket: known.get(levels[i]), first: levels[i + 1], next: known.get(levels[i + 1]) });
    }
  }
  send({ tag: 'scout-done', reason, probes: known.size, searches, joins, boundaries, restored: original });
}

function probe() {
  if (stopped) return;
  if (joins > 0) {
    use(OBSCURING_STONE);
    finish('a JoinLobby happened; used the Obscuring Stone');
    return;
  }
  const level = nextProbe();
  if (level === null) {
    finish('every boundary is two adjacent levels');
    return;
  }
  setLevel(level);
  lastBracket = null;
  const searchesBefore = searches;
  const started = Date.now();
  if (!use(FRAGMENT_OF_MALEVOLANCE)) {
    finish('no Fragment of Malevolance (goods ' + FRAGMENT_OF_MALEVOLANCE + ') in the bag');
    return;
  }
  let lastUse = started;
  let uses = 1;
  const timer = setInterval(() => {
    const now = Date.now();
    const searched = searches > searchesBefore;
    if (!searched && now - lastUse >= REUSE_EVERY_MS) {
      // The game did not take the request (or took it and Seamless did not search): say what the
      // request and the item controller look like, and ask again.
      send({ tag: 'reuse', level, uses, state: useState() });
      use(FRAGMENT_OF_MALEVOLANCE);
      lastUse = now;
      uses += 1;
    }
    const retrying = searched && now - started > PROBE_TIMEOUT_MS;
    if (retrying) use(OBSCURING_STONE); // a search that keeps retrying is cancelled, not waited out
    if ((searched && now - lastSearchAt >= SEARCH_QUIET_MS) || retrying) {
      clearInterval(timer);
      const match = lastBracket === null ? null : lastBracket.match(/_bimm_(\d+)_(\d+)$/);
      if (match === null) {
        finish('level ' + level + ' searched but no bracket filter was read: ' + lastBracket);
        return;
      }
      known.set(level, Number(match[1]));
      send({ tag: 'probe', level, bracket: Number(match[1]), suffix: Number(match[2]), searches: searches - searchesBefore, ms: now - started, cancelled: retrying });
      setTimeout(probe, retrying ? 2 * BETWEEN_PROBES_MS : BETWEEN_PROBES_MS);
    } else if (now - started > PROBE_TIMEOUT_MS) {
      clearInterval(timer);
      finish('level ' + level + ': no search within ' + PROBE_TIMEOUT_MS + ' ms of the use request');
    }
  }, 250);
}

rpc.exports = {
  dispose() {
    if (!stopped) finish('detached');
  },
};

send({ tag: 'scout-start', original, atAttach, fragment_handle: handleOf(FRAGMENT_OF_MALEVOLANCE), stone_handle: handleOf(OBSCURING_STONE) });
setTimeout(probe, 500);
