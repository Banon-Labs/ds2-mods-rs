// Putting a Prism Stone on the ground, live, by calling the game's own spawn.
//
//     PYTHONUNBUFFERED=1 uv run --with frida python3 scripts/ds2-frida-watch.py \
//       --agent scripts/frida/stone.js --log /tmp/stone.log --role stone-drop
//
// WHY THIS EXISTS WHEN `crates/ds2-invasion-path/src/sfx.rs` ALREADY DOES IT
// ------------------------------------------------------------------------
// That crate's version is entirely static. Every address in it was read out of Ghidra and no
// live session has ever called one of them -- bd `ds2-mods-rs-zbo` is the bead that says so. The
// arrow in `arrow.js` went the same way and it was worth it: the projection was structurally
// perfect and pointed fifteen degrees wrong, and no amount of reading found that. A call into
// the engine has more ways to be wrong than a matrix does, and all of them crash the game rather
// than drawing a crooked line.
//
// So this calls the same three functions the crate calls, with the same arguments, from the same
// seam, and prints what came back. What it proves transfers to the crate directly because it is
// not a different implementation -- it is the same one, in a language that can be reloaded in
// two seconds instead of rebuilt and relaunched.
//
// THE STAGE IS THE CONTROL CHANNEL
// --------------------------------
// The watcher reloads this file whenever it is saved, so `STAGE` below is edited and saved to
// advance. There is no rpc interface on `ds2-frida-watch.py` and this wants one bit of input.
//
//   'observe'  reads and prints, CALLS NOTHING. The prologue bytes, the SFX system's three
//              fields, the two thread ids. Everything that can be checked without touching the
//              game runs here, and `feet` refuses to run until it has.
//   'feet'     one stone, at the character's feet, once.
//   'follow'   a trail to whoever the arrow points at, laid three stones per pass from your feet
//              outwards, pruned behind you as you walk past it, and torn down whole only when the
//              route it was laid along has actually changed. This is the real thing.
//
// GAME THREAD ONLY, AND THE SEAM IS `NvNavigationSystem::Update`
// --------------------------------------------------------------
// The spawn walks two live vectors, an xorshift RNG and a red-black tree with no lock anywhere,
// and all 46 of the engine's own call sites are game logic. `Present` is the SAME THREAD -- and
// this agent measures that rather than repeating it -- but it runs with the KatanaMainApp sync
// object and the DrawSystem lock both held, after EndDraw has cleared the frame's in-progress
// flag. Spawning from in there touches the FX manager's lists inside two locks the draw-command
// thread also takes, in a frame the engine considers over. The nav tick holds no lock and is
// mid-simulation, which is where every engine spawn happens.

'use strict';

/// 'observe' | 'feet' | 'follow'. Edit and save; the watcher reloads in place.
const STAGE = 'follow';

// --------------------------------------------------------------------------------------------
// Addresses. Every one of these is a `ds2_rva` constant, quoted with its name so that a drift
// between this file and the crate is visible instead of silent.
// --------------------------------------------------------------------------------------------

/// `ds2_rva::GAME_MANAGER_IMP`
const GAME_MANAGER_IMP_RVA = 0x016148f0;
/// `ds2_rva::PLAYER_CTRL_OFFSET`
const PLAYER_CTRL_OFFSET = 0xd0;
/// `ds2_rva::CHARACTER_CTRL_POSITION_OFFSET`
const POSITION_OFFSET = 0x90;
/// `ds2_rva::GAME_MANAGER_CHARACTER_MANAGER_OFFSET`
const CHARACTER_MANAGER_OFFSET = 0x18;
/// `ds2_rva::CHARACTER_MANAGER_ENTITY_BEGIN_OFFSET` / `..._END_OFFSET`, a begin/end pointer pair.
const ROSTER_BEGIN_OFFSET = 0x10;
const ROSTER_END_OFFSET = 0x18;
/// `ds2_rva::CHARACTER_CTRL_VTABLE`
const CHARACTER_CTRL_VTABLE_RVA = 0x010df218;
const MAX_ROSTER = 8192;

/// `ds2_rva::GAME_MANAGER_SFX_SYSTEM_OFFSET` -- `KatanaSfxSystem` in `GameManagerImp`.
const SFX_SYSTEM_OFFSET = 0xbc8;
/// `ds2_rva::KATANA_SFX_SYSTEM_READY_OFFSET`. One byte; zero means the spawner returns early.
const SFX_READY_OFFSET = 0x46;
/// `ds2_rva::KATANA_SFX_SYSTEM_QUALITY_OFFSET`. `u32`.
const SFX_QUALITY_OFFSET = 0x300;
/// `ds2_rva::KATANA_SFX_QUALITY_DROP_THRESHOLD`. AT OR ABOVE THIS, SPAWNS ARE SILENTLY DROPPED
/// and the caller is handed a control block indistinguishable from a successful one.
const SFX_QUALITY_DROP_THRESHOLD = 3;
/// `ds2_rva::KATANA_SFX_SYSTEM_HIGH_QUALITY_OFFSET`. Non-zero makes the spawner try `id + 20000`
/// and `id + 10000` before `id`, which is why the BASE id is what gets passed.
const SFX_HIGH_QUALITY_OFFSET = 0x305;

/// `ds2_rva::KATANA_SFX_SPAWN`.
/// `(KatanaSfxSystem*, ctrl* out, u32 id, const f32[8]* pose, u32, u8, u8) -> ctrl*`
const SFX_SPAWN_RVA = 0x00beb670;
/// `ds2_rva::KATANA_SFX_STOP`. `(ctrl* whole_block)`.
const SFX_STOP_RVA = 0x00141530;
/// `ds2_rva::KATANA_SFX_CTRL_DESTROY`. `(ctrl* half)` -- called on EACH 0x30 half, descending.
const SFX_CTRL_DESTROY_RVA = 0x00a060f0;
/// `ds2_rva::KATANA_SFX_CTRL_BYTES` and `..._HALF_BYTES`.
const SFX_CTRL_BYTES = 0x68;
const SFX_CTRL_HALF_BYTES = 0x30;
/// `ds2_rva::KATANA_SFX_CTRL_NODE_OFFSET`. BOTH HALVES ZERO HERE MEANS THE BLOCK CONTROLS
/// NOTHING, which is what a quality-throttled spawn produces and is the only way to tell it from
/// a success.
const SFX_CTRL_NODE_OFFSET = 0x10;

/// The trailing three arguments, identical at all 46 engine call sites.
const SPAWN_TAIL = [0, 0xff, 0];

/// `ds2_rva::PRISM_STONE_SFX_IDS` -- `833..=839`, the seven colours the game picks between when
/// you throw one. `ItemParam` row `60450000` is the item; DARK SOULS II calls it a Prism Stone
/// and ELDEN RING renamed it to Rainbow Stone.
const PRISM_STONE_SFX_IDS = [833, 834, 835, 836, 837, 838, 839];

/// `ds2_rva::NV_NAVIGATION_SYSTEM_UPDATE`, the game-thread seam.
const NAV_UPDATE_RVA = 0x00baeb20;
/// `ds2_rva::NV_NAVIGATION_SYSTEM_UPDATE_PROLOGUE`.
/// `mov [rsp+8],rcx ; push rbp ; push r12 ; push r13 ; mov rbp,rsp`
const NAV_UPDATE_PROLOGUE = [
  0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x41, 0x54, 0x41, 0x55, 0x48, 0x8b, 0xec,
];

// --------------------------------------------------------------------------------------------
// THE CADENCE, taken from `er-mods-rs/crates/er-invasion-path` rather than invented here.
// --------------------------------------------------------------------------------------------
//
// The sibling learned this the hard way in its first live run and wrote down what went wrong:
// spawning a whole route's worth of markers in one pass and repeating it every couple of seconds
// made them PILE UP -- each pass laid a fresh set on top of the last until the trail was a wall --
// and the whole thing appeared at once for a route the player had already started walking away
// from. The first version here had exactly the first of those bugs and would have grown the
// second the moment it ran twice.
//
// Every value below is `er-invasion-path`'s, and the first four are also live settings in
// `crates/ds2-invasion-path/src/config.rs` under the same names, with the same defaults, read
// from `<Game>/ds2-mods.toml`. They are duplicated here rather than derived because this agent
// does not parse the toml; a drift between the two is a drift this comment is meant to expose.

/// `marker_spacing_meters`. `DEFAULT_MARKER_SPACING_METERS`.
const MARKER_SPACING_METRES = 2.7;
/// `max_markers`. `DEFAULT_MAX_MARKERS`.
const MAX_MARKERS = 144;
/// `marker_keep_behind_meters`. `DEFAULT_MARKER_KEEP_BEHIND_METERS`.
///
/// NOT ZERO: a trail that ends exactly at your feet reads as broken rather than as followed.
const MARKER_KEEP_BEHIND_METRES = 12.0;
/// `markers_per_pass`. `DEFAULT_MARKERS_PER_PASS`.
///
/// This is the "small delay as it goes". Three per pass lays a full 48-marker trail over about
/// two and a half seconds, from your feet outwards -- fast enough to read as a trail appearing,
/// slow enough that a route to somebody who is moving stops being laid long before it reaches
/// where they used to be.
const MARKERS_PER_PASS = 3;

/// `er_invasion_path::ROSTER_EVERY_TICKS`. One pass every ten game ticks, about a sixth of a
/// second, which is what "per pass" above is counted in.
const ROSTER_EVERY_TICKS = 10;
/// `er_invasion_path::ROUTE_REFRESH_TICKS`. The route is recomputed every two seconds at 60 Hz,
/// not every pass: a pass is for LAYING, and re-asking where the target is sixty times a second
/// would replace the trail faster than it could be walked.
const ROUTE_REFRESH_TICKS = 120;

/// `er_invasion_path::trail::same_route::MOVED_METERS`. How far a marker may move before the
/// trail counts as a different one and the old stones come down.
///
/// The sibling's note: it was 1.0 m, which counted an ordinary re-plan as a new trail and tore
/// down a perfectly good set of stones to lay a near-identical one. A target walking about
/// produces routes that wobble by several metres without going anywhere different, and those
/// must not restart the trail.
const ROUTE_MOVED_METRES = 5.0;

// --------------------------------------------------------------------------------------------
// Reading
// --------------------------------------------------------------------------------------------

function findGameImage() {
  const named = Process.findModuleByName('DarkSoulsII.exe');
  if (named !== null) return named;
  const all = Process.enumerateModules();
  for (const candidate of all) {
    if (candidate.name.toLowerCase() === 'darksoulsii.exe') return candidate;
  }
  return all.length > 0 && all[0].name.toLowerCase().endsWith('.exe') ? all[0] : null;
}

const gameModule = findGameImage();
const base = gameModule === null ? null : gameModule.base;

function readPointer(at) {
  try { return at.readPointer(); } catch (error) { return null; }
}

function gameManager() {
  if (base === null) return null;
  const manager = readPointer(base.add(GAME_MANAGER_IMP_RVA));
  return manager === null || manager.isNull() ? null : manager;
}

function readFloats(at, count) {
  const out = new Array(count);
  try {
    for (let index = 0; index < count; index += 1) out[index] = at.add(index * 4).readFloat();
  } catch (error) {
    return null;
  }
  for (let index = 0; index < count; index += 1) {
    if (!Number.isFinite(out[index])) return null;
  }
  return out;
}

function playerCtrl() {
  const manager = gameManager();
  if (manager === null) return null;
  const ctrl = readPointer(manager.add(PLAYER_CTRL_OFFSET));
  return ctrl === null || ctrl.isNull() ? null : ctrl;
}

function positionOf(ctrl) {
  return readFloats(ctrl.add(POSITION_OFFSET), 3);
}

/// Three-component distance, SPELT OUT. `Math.hypot` with three arguments does not work in
/// Frida 17's QuickJS -- two do, which is what made it look fine -- and it cost an hour in
/// `arrow.js` where the roster found six characters and returned nobody.
function apart(a, b) {
  const dx = a[0] - b[0];
  const dy = a[1] - b[1];
  const dz = a[2] - b[2];
  return Math.sqrt(dx * dx + dy * dy + dz * dz);
}

/// The nearest character that is not the player: the same roster walk `arrow.js` aims with, so
/// the trail ends where the arrow points rather than somewhere merely similar.
function nearestCharacter(playerAt, player) {
  const manager = gameManager();
  if (manager === null || base === null) return null;
  const characters = readPointer(manager.add(CHARACTER_MANAGER_OFFSET));
  if (characters === null || characters.isNull()) return null;
  const begin = readPointer(characters.add(ROSTER_BEGIN_OFFSET));
  const end = readPointer(characters.add(ROSTER_END_OFFSET));
  if (begin === null || end === null || begin.isNull()) return null;
  const span = end.sub(begin).toInt32();
  if (span <= 0 || span % 8 !== 0 || span / 8 > MAX_ROSTER) return null;
  const wanted = base.add(CHARACTER_CTRL_VTABLE_RVA);
  let best = null;
  for (let index = 0; index < span / 8; index += 1) {
    const ctrl = readPointer(begin.add(index * 8));
    if (ctrl === null || ctrl.isNull() || ctrl.equals(player)) continue;
    const vtable = readPointer(ctrl);
    if (vtable === null || !vtable.equals(wanted)) continue;
    const at = positionOf(ctrl);
    if (at === null) continue;
    const distance = apart(at, playerAt);
    if (best === null || distance < best.distance) best = { ctrl, position: at, distance };
  }
  return best;
}

/// The `KatanaSfxSystem`, and everything about it that decides whether a spawn will be visible.
///
/// Read as one object rather than as three calls because the three fields only mean anything
/// together: ready-but-throttled and not-ready-at-all both produce no stone on the ground and
/// want opposite responses.
function sfxSystem() {
  const manager = gameManager();
  if (manager === null) return { why: 'no GameManagerImp yet -- the game is still booting' };
  const system = readPointer(manager.add(SFX_SYSTEM_OFFSET));
  if (system === null || system.isNull()) {
    return { why: 'GameManagerImp+0xbc8 is null -- no KatanaSfxSystem' };
  }
  let ready;
  let quality;
  let highQuality;
  try {
    ready = system.add(SFX_READY_OFFSET).readU8();
    quality = system.add(SFX_QUALITY_OFFSET).readU32();
    highQuality = system.add(SFX_HIGH_QUALITY_OFFSET).readU8();
  } catch (error) {
    return { why: 'KatanaSfxSystem ' + system + ' is not readable: ' + error.message };
  }
  return { system, ready, quality, highQuality };
}

// --------------------------------------------------------------------------------------------
// Calling
// --------------------------------------------------------------------------------------------

let spawnSfx = null;
let stopSfx = null;
let destroySfxCtrl = null;

function bindCalls() {
  if (base === null) return false;
  spawnSfx = new NativeFunction(base.add(SFX_SPAWN_RVA), 'pointer', [
    'pointer', 'pointer', 'uint32', 'pointer', 'uint32', 'uint8', 'uint8',
  ], 'win64');
  stopSfx = new NativeFunction(base.add(SFX_STOP_RVA), 'void', ['pointer'], 'win64');
  destroySfxCtrl = new NativeFunction(base.add(SFX_CTRL_DESTROY_RVA), 'void', ['pointer'], 'win64');
  return true;
}

// --------------------------------------------------------------------------------------------
// Storage for the control blocks, WHICH MAY NOT BE FRIDA'S
// --------------------------------------------------------------------------------------------
//
// `Memory.alloc` belongs to the script, and `ds2-frida-watch.py` calls `script.unload()` on every
// edit to this file. The engine links a control block into the effect node's controller list --
// the head at `node + 0xf8` points AT these bytes -- so a reload while any stone is alight would
// hand those pages back with the engine still holding a pointer into them, and the crash would
// land on some later frame with nothing in the log connecting it to a save in an editor. Every
// reload during this work would have been a dice roll.
//
// `VirtualAlloc` is the process's, not the script's. It survives the unload, which means an
// orphaned stone from a previous reload is a leaked page and a pebble nobody can put out --
// annoying, and not a use-after-free in the engine's list walk.
//
// The alignment comes free with it and is load-bearing either way: the two `FXCGSfxCtrl`
// constructors store vtable pointers with aligned moves, `Memory.alloc` promises eight bytes, and
// a `movaps` to an address ending in 8 is the whole game gone rather than a bad log line.

const MEM_COMMIT_RESERVE = 0x1000 | 0x2000;
const PAGE_READWRITE = 0x04;
/// `KATANA_SFX_CTRL_BYTES` rounded up to a multiple of sixteen, so every block in a page is
/// aligned without any arithmetic per block.
const BLOCK_STRIDE = 0x70;
const ARENA_BYTES = 4096;

let virtualAlloc = null;
let arena = null;
let arenaUsed = ARENA_BYTES;
let arenaPages = 0;

function bindAllocator() {
  const kernel32 = Process.findModuleByName('kernel32.dll');
  const found = kernel32 === null ? null : kernel32.findExportByName('VirtualAlloc');
  if (found === null) return false;
  virtualAlloc = new NativeFunction(found, 'pointer',
    ['pointer', 'ulong', 'uint32', 'uint32'], 'win64');
  return true;
}

/// Zeroed, sixteen-byte-aligned storage for one control block, NEVER FREED.
///
/// Deliberately never freed. A page holds thirty-six of these and a session lays a few hundred
/// stones at the outside, so the leak is measured in kilobytes; handing a page back that the
/// engine might still be walking is measured in crashes.
function blockStorage() {
  if (virtualAlloc === null) return null;
  if (arenaUsed + BLOCK_STRIDE > ARENA_BYTES) {
    const page = virtualAlloc(NULL, ARENA_BYTES, MEM_COMMIT_RESERVE, PAGE_READWRITE);
    if (page.isNull()) return null;
    arena = page;
    arenaUsed = 0;
    arenaPages += 1;
  }
  const at = arena.add(arenaUsed);
  arenaUsed += BLOCK_STRIDE;
  for (let offset = 0; offset < SFX_CTRL_BYTES; offset += 8) at.add(offset).writeU64(0);
  return at;
}

/// Stop an effect and unlink its block, in the order `0x140446ee0` uses on its own.
///
/// STOP FIRST, DESTRUCT SECOND, and the destructor runs on each `0x30`-byte half DESCENDING. The
/// destructor unlinks; it does not stop. Calling it once on the whole block leaves the other half
/// on the node's controller list, which is memory the engine writes through later.
function despawn(at) {
  stopSfx(at);
  destroySfxCtrl(at.add(SFX_CTRL_HALF_BYTES));
  destroySfxCtrl(at);
}

/// Is anything actually playing out of this block?
///
/// Both node pointers zero means the spawn produced nothing: `0x140beb670` answers a
/// quality-throttled request with a block built by `0x140127240` whose halves are both empty,
/// and it is returned exactly like a success. There is no other way to tell them apart.
function live(at) {
  for (const half of [0, SFX_CTRL_HALF_BYTES]) {
    const node = readPointer(at.add(half + SFX_CTRL_NODE_OFFSET));
    if (node !== null && !node.isNull()) return true;
  }
  return false;
}

/// Spawn one effect at `position`, facing `direction`, and say which of the two outcomes it was.
///
/// `direction` must be a UNIT vector: `0x140beb590`, the matrix-taking wrapper, normalises before
/// calling this one, so the core is entitled to assume it. `[0, 1, 0]` is the honest answer for a
/// marker with no heading -- the one axis a ground effect can use without claiming a direction of
/// travel it does not have.
function spawnOne(system, id, position, direction) {
  const pose = Memory.alloc(32);
  pose.writeFloat(position[0]);
  pose.add(4).writeFloat(position[1]);
  pose.add(8).writeFloat(position[2]);
  pose.add(12).writeFloat(0);
  pose.add(16).writeFloat(direction[0]);
  pose.add(20).writeFloat(direction[1]);
  pose.add(24).writeFloat(direction[2]);
  pose.add(28).writeFloat(0);

  const at = blockStorage();
  if (at === null) return { at: null, live: false };
  spawnSfx(system, at, id, pose, SPAWN_TAIL[0], SPAWN_TAIL[1], SPAWN_TAIL[2]);
  return { at, live: live(at) };
}

// --------------------------------------------------------------------------------------------
// The seam
// --------------------------------------------------------------------------------------------

let navTicks = 0;
let navThread = null;
/// The `NvNavigationSystem` the engine handed this tick, from `rcx`.
let navSystemAt = null;
let threadsCompared = false;
/// What the stage asked for and has not been given yet. Cleared the instant it is served, so a
/// stage runs ONCE per reload rather than sixty times a second.
let pending = STAGE === 'observe' ? null : STAGE;
let reported = false;

function describeSystem(found) {
  if (found.why !== undefined) return 'KatanaSfxSystem: ' + found.why;
  const verdict = found.ready === 0
    ? 'NOT READY -- the spawner returns before touching anything'
    : found.quality >= SFX_QUALITY_DROP_THRESHOLD
      ? 'READY BUT THROTTLED -- quality ' + found.quality + ' >= ' + SFX_QUALITY_DROP_THRESHOLD +
        ', so spawns are dropped and handed back looking like successes'
      : 'READY, and quality ' + found.quality + ' is below the drop threshold';
  return 'KatanaSfxSystem ' + found.system + ': ready=' + found.ready + ' quality=' +
    found.quality + ' high-quality-variants=' + found.highQuality + '\n[stone] ' + verdict;
}

/// Check the thirteen prologue bytes BEFORE anything is called or patched.
///
/// This is the one test that separates "the RVA is right" from "the RVA drifted and the call
/// lands in the middle of some other function", and it costs a read. Getting it wrong the other
/// way round means calling an arbitrary address with seven arguments on the game thread.
function prologueMatches() {
  if (base === null) return { ok: false, why: 'no game image' };
  let got;
  try {
    got = base.add(NAV_UPDATE_RVA).readByteArray(NAV_UPDATE_PROLOGUE.length);
  } catch (error) {
    return { ok: false, why: 'NvNavigationSystem::Update is not readable: ' + error.message };
  }
  const bytes = new Uint8Array(got);
  const hex = (values) => Array.from(values).map((v) => v.toString(16).padStart(2, '0')).join(' ');
  for (let index = 0; index < NAV_UPDATE_PROLOGUE.length; index += 1) {
    if (bytes[index] !== NAV_UPDATE_PROLOGUE[index]) {
      return {
        ok: false,
        why: 'prologue mismatch at byte ' + index + '\n[stone]   wanted ' +
          hex(NAV_UPDATE_PROLOGUE) + '\n[stone]   found  ' + hex(bytes) +
          '\n[stone] Either the RVA drifted or Arxan has redirected this entry. Either way ' +
          'calling through it is calling an unknown address with seven arguments.',
      };
    }
  }
  return { ok: true, hex: hex(bytes) };
}

function serve(asked) {
  const found = sfxSystem();
  if (found.why !== undefined) { console.log('[stone] cannot spawn: ' + found.why); return; }
  if (found.ready === 0) { console.log('[stone] cannot spawn: the system is not ready'); return; }
  if (found.quality >= SFX_QUALITY_DROP_THRESHOLD) {
    console.log('[stone] REFUSING: quality ' + found.quality + ' means the engine will drop the ' +
      'spawn and hand back a block that looks like a success. Nothing would appear and the log ' +
      'would say it worked.');
    return;
  }
  const ctrl = playerCtrl();
  if (ctrl === null) { console.log('[stone] cannot spawn: no character'); return; }
  const at = positionOf(ctrl);
  if (at === null) { console.log('[stone] cannot spawn: the character has no readable position'); return; }

  if (asked === 'feet') {
    const result = spawnOne(found.system, PRISM_STONE_SFX_IDS[0], at, [0, 1, 0]);
    console.log('[stone] ===== ONE STONE, AT YOUR FEET =====\n' +
      '[stone] effect ' + PRISM_STONE_SFX_IDS[0] + ' at ' +
      at.map((v) => v.toFixed(2)).join(', ') + '\n' +
      '[stone] control block ' + result.at + ': ' + (result.live
        ? 'LIVE -- both halves of the block point at an effect node, so the engine built one'
        : 'EMPTY -- both node pointers are null, so nothing was created') + '\n' +
      '[stone] LOOK AT THE GROUND WHERE YOU ARE STANDING. A live block and no pebble means the ' +
      'effect was built and is invisible, which is a different problem from this one.');
    return;
  }

}

// --------------------------------------------------------------------------------------------
// The trail, laid and taken up again at `er-invasion-path`'s cadence
// --------------------------------------------------------------------------------------------
//
// The first version of this spawned a whole line of stones in one go, which is the exact bug the
// sibling's `trail.rs` has a paragraph about: run it twice and the stones pile up, because every
// pass lays a fresh set on top of the last. A trail is instead laid a FEW PER PASS from your feet
// outwards, pruned behind you as you walk past it, and torn down whole only when the route it was
// laid along has actually changed.

const trail = {
  /// Every marker position for the current route, in order from the player outwards.
  spots: [],
  /// How many have been spawned. Laying stops when this reaches `spots.length`.
  laid: 0,
  /// The stones in the world, in the order they were laid: `{ at, block }`.
  placed: [],
  /// The tick the route was accepted, for `ROUTE_REFRESH_TICKS`.
  computedAt: -ROUTE_REFRESH_TICKS,
  /// The way the trail runs. A direction this code knows, rather than one reconstructed.
  heading: [0, 1, 0],
};
let trailTarget = null;
let laidTotal = 0;
let removedTotal = 0;
let forgottenTotal = 0;
/// Set to the error that killed a pass, which then stops running. A throw sixty times a second
/// fills the log faster than it can be read and hides its own first occurrence.
let passStopped = null;
let lastPlayerAt = null;

/// Are these two marker runs the same trail?
///
/// Position by position, not by route identity: the navmesh returns a fresh answer every refresh
/// and two answers to the same question differ in the last decimal place without differing
/// anywhere a player could see. Treating those as a new trail is what re-laid stones on top of
/// themselves.
function sameRoute(previous, next) {
  if (previous.length !== next.length) return false;
  for (let index = 0; index < previous.length; index += 1) {
    if (apart(previous[index], next[index]) > ROUTE_MOVED_METRES) return false;
  }
  return true;
}

/// Point the trail at a new route, if it is actually a different one. Returns whether it restarted.
///
/// An unchanged route MUST NOT restart: the stones for it are already in the world, and re-laying
/// them is precisely what made them accumulate.
function retarget(spots) {
  if (sameRoute(trail.spots, spots)) return false;
  trail.spots = spots;
  trail.laid = 0;
  return true;
}

/// Distance between consecutive markers, measured from the trail itself rather than assumed.
function spacing() {
  for (let index = 1; index < trail.spots.length; index += 1) {
    const step = apart(trail.spots[index], trail.spots[index - 1]);
    if (step > 1e-6) return step;
  }
  return 1.0;
}

/// Which placed stones are far enough behind `player` to be clutter.
///
/// "Behind" is BY TRAIL ORDER, NOT BY ANGLE. The stones were laid from the player outwards, so
/// anything before the nearest one is ground already covered. Using distance alone would tear
/// down the far end of a route that doubles back past you -- which a spiral staircase does
/// constantly, and those are the stones you most need to see.
function stalePrefix(player, keepBehind) {
  if (trail.placed.length === 0) return 0;
  let nearest = 0;
  let best = Infinity;
  for (let index = 0; index < trail.placed.length; index += 1) {
    const distance = apart(trail.placed[index].at, player);
    if (distance < best) { best = distance; nearest = index; }
  }
  const keep = keepBehind > 0 ? Math.ceil(keepBehind / Math.max(spacing(), 1e-6)) : 0;
  return Math.max(0, nearest - keep);
}

function clearPlaced() {
  let removed = 0;
  for (const placed of trail.placed) { despawn(placed.block); removed += 1; }
  trail.placed = [];
  return removed;
}

/// Let go of every stone WITHOUT touching the engine.
///
/// For the one case where extinguishing is the wrong move: the area changed underneath us. The
/// effect nodes those blocks point at were torn down with the area, so `stop` and `destroy` would
/// be writes through pointers into freed engine memory -- the crate says the same thing and calls
/// `mem::forget` for it. The blocks leak; leaking is what `blockStorage` is built to survive.
function forgetPlaced() {
  const count = trail.placed.length;
  trail.placed = [];
  trail.spots = [];
  trail.laid = 0;
  return count;
}

function pruneBehind(player, keepBehind) {
  const stale = stalePrefix(player, keepBehind);
  if (stale === 0) return 0;
  const gone = trail.placed.splice(0, stale);
  for (const placed of gone) despawn(placed.block);
  return gone.length;
}

/// Spawn the next few stones, keeping each block so it can be put out again.
function layNext(system, batch) {
  const from = trail.laid;
  const to = Math.min(from + batch, trail.spots.length);
  let placed = 0;
  for (let index = from; index < to; index += 1) {
    const at = trail.spots[index];
    // ONE COLOUR PER STONE. The seven ids are the Prism Stone's own, and sweeping them means a
    // run also says which of them resolve -- an id the engine cannot find goes into the
    // failed-lookup tree and produces exactly the same nothing as a working id that is invisible
    // for some other reason.
    const id = PRISM_STONE_SFX_IDS[index % PRISM_STONE_SFX_IDS.length];
    const result = spawnOne(system, id, at, trail.heading);
    if (!result.live) {
      // Nothing was created. LEAVE THE CURSOR WHERE IT IS and try again next pass, rather than
      // marking these positions laid when no stone stands on them.
      break;
    }
    trail.placed.push({ at, block: result.at });
    placed += 1;
  }
  trail.laid = from + placed;
  return placed;
}

/// The one search this agent ever has in flight.
///
/// NO STRAIGHT-LINE FALLBACK, deliberately. There used to be one, and a straight line is exactly
/// the thing being fixed -- it runs through walls and hangs over gaps. Worse, laying it when the
/// planner refuses would HIDE the refusal, which is the failure that most needs to be visible.
/// The sibling does the same: no route, no trail, and the arrow stands in.
const route = {
  /// The `NvRoutePlanner` this agent created and linked onto the navigation system's list.
  planner: null,
  /// Is a search outstanding, and for how many passes.
  pending: false,
  waited: 0,
  /// Where the trail's points came from, for the heartbeat: `none` | `navmesh`.
  source: 'none',
  saidPlanner: false,
  saidSnap: false,
  saidRoute: false,
  saidNoRoute: false,
  saidAudit: false,
};

/// How far the character may move in one pass before it counts as a load rather than a walk.
///
/// A sixth of a second of running is a couple of metres; a death, a bonfire warp or an area
/// transition is hundreds. The stones from before such a jump belong to an area that has been
/// torn down, and the pointers in them are the engine's freed memory -- so they are FORGOTTEN
/// rather than put out. Chosen well above anything a player can cover in a pass and well below
/// any real transition.
const LOAD_JUMP_METRES = 100.0;

/// Snap both ends and ask the planner for a walkable route between them.
///
/// The snap line is printed ONCE rather than twice a second, because a line that repeats sixty
/// times is a line nobody reads -- and it carries the guard beside the ids, so a refusal names
/// the false condition instead of handing the reader three suspects.
function askRoute(player, goal) {
  if (route.planner === null) return;
  const from = snapReporting(player);
  const to = snapReporting(goal);
  if (!route.saidSnap) {
    route.saidSnap = true;
    const guard = from.id === null || to.id === null ? null : readGuard(from.id, to.id);
    console.log('[stone] snap start=' + describeSnap(from) + ' goal=' + describeSnap(to) +
      ' | world holds ' + from.graphs + ' graph(s); map index ' +
      (from.mapIndex === null ? 'UNREADABLE' :
        from.mapIndex === MAP_INDEX_NONE ? 'is the NO-PLAYER-ENTITY sentinel (0xffffffff)' :
        from.mapIndex + ' gave key 0x' + from.key.toString(16) +
          (from.keyedHit ? ' which matched a graph' : ' which matched NO graph') +
          (from.chosenKey === null ? '' : ', and the sweep chose one carrying 0x' +
            from.chosenKey.toString(16))) +
      ' | ' + describeGuard(guard));
  }
  if (from.id === null || to.id === null) {
    // One end is off the navmesh -- a lift shaft, mid-fall, past the snap radius, or a map whose
    // graph is not resident. A complete answer, and the arrow's cue.
    return;
  }
  requestRouteCall(route.planner, from.id, to.id,
    NV_ROUTE_PLANNER_CAPABILITY_DEFAULT, NV_ROUTE_MAX_COST_LONG_RANGE);
  route.pending = true;
  route.waited = 0;
}

/// Advance the outstanding search by one pass.
function advanceRoute() {
  const answer = pollRoute(route.planner);
  if (answer.state === 'pending') {
    route.waited += 1;
    if (route.waited > PENDING_PASS_BUDGET) {
      route.pending = false;
      console.log('[stone] a search went ' + PENDING_PASS_BUDGET + ' passes without an answer -- ' +
        'abandoning it. That is the planner not being STEPPED, which is a different failure from ' +
        'no way to walk there and must not be read as one.');
    }
    return;
  }
  route.pending = false;
  if (answer.state === 'lost') {
    console.log('[stone] the planner became unreadable -- making another');
    route.planner = null;
    return;
  }
  if (answer.state === 'failed') {
    if (!route.saidNoRoute) {
      route.saidNoRoute = true;
      console.log('[stone] NO WALKABLE ROUTE' +
        (answer.decoded === false
          ? ' -- and this one is READY with ' + answer.segments + ' segment(s) that would not ' +
            'DECODE, which is the decoder or the route base, not the world'
          : ' -- the search answered and the answer is no. Read the `guard` on the snap line ' +
            'above: `guard REFUSES` is the engine rejecting the pair before expanding a node. ' +
            '`guard OK` means the search really ran and found nothing, and THEN the capability ' +
            '(0x' + NV_ROUTE_PLANNER_CAPABILITY_DEFAULT.toString(16) + ') and the budget (' +
            NV_ROUTE_MAX_COST_LONG_RANGE + ') are worth looking at.') +
        ' No stones are laid; the arrow stands in.');
    }
    // The route is gone, so the stones laid along it are pointing at nothing.
    removedTotal += clearPlaced();
    trail.spots = [];
    trail.laid = 0;
    route.source = 'none';
    return;
  }
  const spots = resampleAlong(answer.points, MARKER_SPACING_METRES, MAX_MARKERS);
  if (!route.saidRoute) {
    route.saidRoute = true;
    console.log('[stone] first walkable route -- ' + answer.segments + ' segment(s) decoded to ' +
      answer.points.length + ' point(s), ' + pathLength(answer.points).toFixed(1) +
      ' m of path, resampled to ' + spots.length + ' stone position(s) at ' +
      MARKER_SPACING_METRES + ' m. These sit ON the navmesh, so they follow the ground.');
  }
  // Retargeting first is what stops the pile-up: a route that has not moved keeps the stones it
  // already has instead of getting a second set on top. A route that HAS moved takes its old
  // stones down before laying new ones.
  const moved = retarget(spots);
  if (moved) removedTotal += clearPlaced();
  // AUDIT EVERY FRESH ROUTE, NOT JUST THE FIRST. The first route a session produces is whatever
  // the player happened to be standing next to -- two nodes and a metre of walking, which has
  // nothing interesting in it. The route worth examining is a long one, and that one only turns
  // up after the target has moved. Re-arming on `moved` costs one pass in a hundred and is the
  // difference between auditing the question and auditing the warm-up.
  if (moved || !route.saidAudit) {
    reportAudit(auditRoute(routeObject(route.planner)), !route.saidAudit);
    route.saidAudit = true;
  }
  route.source = 'navmesh';
}

/// One roster pass: retarget, clear, prune, lay. In that order, which is the sibling's.
function pass() {
  const found = sfxSystem();
  if (found.why !== undefined || found.ready === 0) return;
  if (found.quality >= SFX_QUALITY_DROP_THRESHOLD) return;
  const ctrl = playerCtrl();
  if (ctrl === null) return;
  const player = positionOf(ctrl);
  if (player === null) return;

  // THE AREA CHANGED, PROBABLY. Before anything is extinguished, because extinguishing is the
  // thing that is unsafe here.
  if (lastPlayerAt !== null && apart(player, lastPlayerAt) > LOAD_JUMP_METRES) {
    const forgotten = forgetPlaced();
    forgottenTotal += forgotten;
    trailTarget = null;
    console.log('[stone] the character jumped ' + apart(player, lastPlayerAt).toFixed(0) +
      ' m in one pass -- a death, a warp or an area load. ' + forgotten + ' stones let go of ' +
      'WITHOUT being put out: their effect nodes went with the area, so stopping them would be a ' +
      'write into freed engine memory.');
  }
  lastPlayerAt = player;

  // THE ROUTE. A search outstanding is polled every pass; otherwise a new one is asked for every
  // `ROUTE_REFRESH_TICKS` and not more often. A pass is for LAYING; re-asking on every pass would
  // replace the trail faster than it can be walked.
  if (route.pending) {
    advanceRoute();
  } else if (navTicks - trail.computedAt >= ROUTE_REFRESH_TICKS) {
    trail.computedAt = navTicks;
    const target = nearestCharacter(player, ctrl);
    if (target !== null) {
      if (trailTarget === null || !target.ctrl.equals(trailTarget)) {
        console.log('[stone] trail target: CharacterCtrl ' + target.ctrl + ', ' +
          target.distance.toFixed(2) + ' m away');
        trailTarget = target.ctrl;
      }
      trail.heading = [
        (target.position[0] - player[0]) / target.distance,
        (target.position[1] - player[1]) / target.distance,
        (target.position[2] - player[2]) / target.distance,
      ];
      askRoute(player, target.position);
    }
  }

  // Stones you have already walked past are clutter behind you, so they go AS YOU PASS THEM
  // rather than waiting for the whole route to change.
  removedTotal += pruneBehind(player, MARKER_KEEP_BEHIND_METRES);

  if (trail.laid < trail.spots.length) laidTotal += layNext(found.system, MARKERS_PER_PASS);
}

// --------------------------------------------------------------------------------------------
// THE NAVMESH ROUTE, which is what makes the trail follow the ground and stop crossing gaps
// --------------------------------------------------------------------------------------------
//
// A straight line between two characters is not a path. It runs through walls, it hangs in the
// air over a chasm, and it cuts the corner of every staircase. DARK SOULS II answers the real
// question itself -- `NvRoutePlanner` walks its own navmesh -- and every address below is one
// `crates/ds2-rva` already carries with the disassembly it was read from. Nothing here is a new
// derivation except the one correction in `routeObject`, which is called out where it happens.
//
// All of it runs on the game thread, inside this agent's `NvNavigationSystem::Update` hook, and
// that is not a convention: the snap's inner allocator is lazily built and unguarded, and the
// planner lives on the list that function is walking.

const GAME_MANAGER_MAP_MANAGER_OFFSET = 0x38;
const MAP_MANAGER_PLAYER_MAP_INDEX_OFFSET = 0x170;
/// `MapManager+0x170` when the player has no map entity. A key built from this is WELL-FORMED
/// and matches nothing, which is why it is tested before a key exists to be wrong.
const MAP_INDEX_NONE = 0xffffffff;

/// `u32 area -> u32 key`, `(area & 0x3f) << 24 | 0xffffff`.
const NAVI_GRAPH_KEY_FROM_AREA_RVA = 0x00bab1f0;
/// `GameManagerImp* -> NvNaviGraphWorld*`, i.e. `[[gm + 0xBC0] + 0x10]`.
const NAVI_GRAPH_WORLD_FROM_GAME_MANAGER_RVA = 0x0039a9f0;
/// `(NvNaviGraphWorld*, u32 key) -> graph data*`. A linear scan over the resident graphs.
const NAVI_GRAPH_DATA_FOR_KEY_RVA = 0x00badb90;
/// `(graph data*, const f32* pos, f32 radius, u32 filter, f32* out_dist) -> u32 id`.
const NAVI_GRAPH_NEAREST_ID_RVA = 0x00babf90;
/// `(id table*, u32 key) -> NvNaviGraph*`. The hash the PLANNER uses, which is not the scan the
/// snap uses -- and that difference is why two good ids can still be refused.
const NV_NAVI_GRAPH_FOR_ROUTE_ID_RVA = 0x00bb2620;
/// `NvNavigationSystem* -> NvRoutePlanner*`, allocated, constructed, bound and LINKED.
const NV_NAVIGATION_SYSTEM_CREATE_ROUTE_PLANNER_RVA = 0x00bae8d0;
/// `(NvRoutePlanner*, u32 start, u32 goal, u32 capability, f32 max_cost)`.
const NV_ROUTE_PLANNER_REQUEST_RVA = 0x00bb4090;
/// `NvNavigationSystem -> length of its update list`. `+0x28`. Read for one thing: proving a
/// planner this agent created was actually LINKED, which is a different fact from created.
const NV_NAVIGATION_SYSTEM_LIST_COUNT_OFFSET = 0x28;

const NV_NAVI_GRAPH_WORLD_GRAPHS_OFFSET = 0x28;
const NV_NAVI_GRAPH_WORLD_GRAPH_COUNT_OFFSET = 0x68;
/// Eight, and STRUCTURAL rather than chosen: the inline array starts at `+0x28` and the count
/// that bounds it sits at `+0x68`, so a ninth pointer would overwrite its own count.
const NV_NAVI_GRAPH_WORLD_MAX_GRAPHS = 8;
const NV_NAVI_GRAPH_WORLD_ID_TABLE_OFFSET = 0x88;
const NV_NAVI_GRAPH_HEADER_OFFSET = 0x28;
const NV_NAVI_GRAPH_HEADER_KEY_OFFSET = 0x1c;
const NV_NAVI_GRAPH_LINK_COUNT_OFFSET = 0x30;
const NV_ROUTE_ID_GRAPH_KEY_MASK = 0x1ffff;

/// The most permissive snap the engine offers: `0x20 | 0x08`, class minimum zero. Right for a
/// player, who is not an AI with a movement class and should land on whatever mesh is underfoot.
const NAVI_GRAPH_SNAP_FILTER = 0x28;
/// `20.0`, the radius `0x14037be30` uses. The AI paths use `10.0`; a player on a ledge, a stair
/// or a corpse is further off the mesh than a walking AI ever is, and a missed snap costs the
/// whole route.
const NAVI_GRAPH_SNAP_RADIUS_METRES = 20.0;
const NAVI_GRAPH_ID_NONE = 0xffffffff;

const NV_ROUTE_PLANNER_FLAGS_OFFSET = 0x30;
const NV_ROUTE_PLANNER_FLAG_READY = 0x02;
const NV_ROUTE_PLANNER_FLAG_FAILED = 0x04;
/// Where the planner's `NvRoute` LIVES. See `routeObject` -- this is an offset to the object,
/// not to a pointer to it.
const NV_ROUTE_PLANNER_ROUTE_OFFSET = 0x48;
/// `0x7f8` -- bits 3..10 set, size class ZERO. This is the MOST PERMISSIVE agent the engine can
/// describe, and calling it "a person at the controls" (which this comment used to) is wrong in
/// the one direction that matters.
///
/// `0x140baf0d0`, the traversal predicate, admits a node only when
///
/// ```text
/// (nodeFlags bit8 clear || capability bit3 set) && (capability & 7) < (nodeFlags & 7)
/// ```
///
/// so the low three bits are the AGENT'S SIZE and a SMALLER number passes MORE nodes. Size class
/// 0 is the smallest agent the format has: it fits through every gap with a capacity of one or
/// more. The engine's own caller never uses it blind -- `0x14042ee40` builds the word as
/// `FUN_14042c180(chrParam) | 0x7f8`, where `0x14042c180` maps a character's size parameter
/// 1..7 onto size class 0..6. Requesting `0x7f8` alone therefore asks for a route a MOUSE could
/// walk, and the answer comes back running through gaps, drops and doorways a player cannot use.
///
/// See `auditRoute`, which measures that gap instead of guessing at it.
const NV_ROUTE_PLANNER_CAPABILITY_DEFAULT = 0x7f8;
/// The feature half of the capability word -- everything except the size class in bits 0..2.
/// `0x14042ee40` ORs exactly this constant onto the character's size, so it is the engine's own
/// literal rather than a mask derived here.
const NV_ROUTE_CAPABILITY_FEATURES = 0x7f8;
/// Size classes the format can express: `0x14042c180` is a seven-arm switch returning 0..6 and
/// `default: 0`, so this is the whole range and not a guess at one.
const NV_ROUTE_CAPABILITY_SIZE_CLASSES = 7;
/// `999.0`, the larger of the two budgets the engine passes. The target is another character
/// somewhere on the map rather than an AI's next few metres.
const NV_ROUTE_MAX_COST_LONG_RANGE = 999.0;

const NV_ROUTE_SEGMENTS_OFFSET = 0x10;
const NV_ROUTE_SEGMENT_COUNT_OFFSET = 0x18;
const NV_ROUTE_SEGMENT_STRIDE = 0x60;
const NV_ROUTE_SEGMENT_POINT_OFFSET = 0x20;
const NV_ROUTE_SEGMENT_POINTS_OFFSET = 0x40;
const NV_ROUTE_SEGMENT_POINT_COUNT_OFFSET = 0x50;
const ROUTE_POINT_STRIDE = 16;

// WHAT A ROUTE SEGMENT SAYS BESIDES WHERE IT IS.
//
// `0x140bb4ac0` is the planner state that WRITES the finished route, and every field below is
// read off one of its stores into `[planner+0x58] + i*0x60`:
//
//   *(u32 *)(seg + 0x00) = edgeRecord + 0xc   (or 0xffffffff on the last segment)
//   *(u32 *)(seg + 0x04) = edgeRecord + 0xc   (or 0xffffffff on the first)
//   *(u32 *)(seg + 0x08) = FUN_140bb9d50(...) (a packed navi id; planner+0x84 at the end)
//   *(u32 *)(seg + 0x0c) = FUN_140bb9d50(...) (a packed navi id; planner+0x88 at the start)
//   *(f32x4*)(seg + 0x10), (seg + 0x20) = (v0 + v1) * 0.5 -- a PORTAL MIDPOINT, not a node centre
//
// The two ids are the reason this block exists. A route point is a position and a position
// cannot be interrogated; a navi id can, because the same id indexes the attribute table the
// engine's own traversal test reads. That is what turns "the trail goes through that wall" from
// an observation into a lookup.
const NV_ROUTE_SEGMENT_NODE_A_OFFSET = 0x08;
const NV_ROUTE_SEGMENT_NODE_B_OFFSET = 0x0c;
/// The portal midpoint written in the same breath as [`NV_ROUTE_SEGMENT_NODE_A_OFFSET`]. Its
/// partner for node B is [`NV_ROUTE_SEGMENT_POINT_OFFSET`], which already had a name because the
/// decoder falls back to it. Pairing each id with its own position is what lets a report say
/// WHERE the unusable node is instead of only that there is one.
const NV_ROUTE_SEGMENT_POINT_A_OFFSET = 0x10;

/// `NvNaviGraph -> node attributes`. `+0x48`, a `u32` per node, indexed by the LOW FIFTEEN BITS
/// of a packed navi id.
///
/// `0x14042ee40` -- the AI's own "go here" -- does exactly this before it will even ask for a
/// route:
///
/// ```text
/// lVar7 = FUN_140bb2620(navSystem + 0x88, param_3 | 0x1ffff);       ; the graph holding the id
/// uVar1 = *(u32 *)(*(longlong *)(lVar7 + 0x48) + (param_3 & 0x7fff) * 4);
/// uVar4 = FUN_14042c180(*(u32 *)([[[chr+8]+0x38]+0x40] + 4));       ; the character's size class
/// if ((float)FUN_140baf0d0(uVar1, uVar4 | 0x7f8, 0) == DAT_1410ae854) { give up }
/// ```
///
/// so an unreachable destination is decided from a BAKED `u32` and the agent's capability word,
/// and nothing else. That is also the ceiling on what this table can ever tell you: see
/// `auditRoute`.
const NV_NAVI_GRAPH_NODE_ATTRS_OFFSET = 0x48;

/// The index half of a packed navi id. `0x7fff` everywhere the engine touches one, and
/// `0x7fff` itself is the "no node" sentinel `0x14042ee40` tests for before indexing.
const NAVI_ID_INDEX_MASK = 0x7fff;
const NAVI_ID_INDEX_NONE = 0x7fff;

/// The type field of a node attribute word: bits 3..6, the value `0x140baf0d0` switches on.
const NAVI_NODE_TYPE_MASK = 0x78;

/// Node types whose admission depends on the capability word AND on the traveller's HEIGHT
/// relative to the edge.
///
/// `0x140baf0d0` gives types `0x20` and `0x40` two different tests depending on its third
/// argument, and `0x140bba040` -- the gate check that actually passes that argument -- computes
/// it as geometry, not as a direction:
///
/// ```text
/// cVar7 = (midY(edgeA) < refY) || (midY(edgeB) < refY);   ; refY = *(float *)(param_6 + 4)
/// ... (**(code **)(*estimator + 0x18))(estimator, attrs, capability, cVar7)
/// ```
///
/// so the flag means THIS EDGE SITS BELOW ME. A type `0x20` or `0x40` edge can therefore be a
/// drop you may take and a climb you may not, from the same node -- which is the shape of every
/// ledge in the game. Calling it "direction" would be a guess; calling it height is what the
/// engine computes. `0x10` is gated on the capability alone.
const NAVI_NODE_TYPE_GATED = [0x10, 0x20, 0x40];

/// `NvNaviGraphCostEstimator`'s traversal predicate, and the `float` it returns when the answer
/// is no.
///
/// `0x140baf0d0(nodeAttrs, capability, direction)` returns a cost, and `0x1401c1e60` -- the
/// engine's own boolean wrapper, the one RTTI names `NvNaviGraphCostEstimator` -- is nothing but
/// `cost != DAT_1410ae854`. So the sentinel IS the refusal, and it is read from the image rather
/// than assumed to be any particular float.
const NAVI_EDGE_TRAVERSAL_COST_RVA = 0x00baf0d0;
const NAVI_IMPASSABLE_COST_RVA = 0x010ae854;

/// Bounds on a structure read out of live memory. A count past these means the pointer was not a
/// route, so the decode REFUSES rather than truncating: a route half-read draws a confident line
/// to somewhere nobody is.
const MAX_ROUTE_SEGMENTS = 4096;
const MAX_POINTS_PER_SEGMENT = 1024;
const MAX_ROUTE_POINTS = 8192;

/// How many passes a search may go unanswered before it is abandoned. Ten passes is a hundred
/// nav ticks. Abandoning is NOT "no way to walk there" -- it is "the planner stopped being
/// stepped", a different failure that must not be reported as the first one.
const PENDING_PASS_BUDGET = 10;

let keyFromArea = null;
let graphWorldOf = null;
let graphDataForKey = null;
let nearestGraphId = null;
let graphForRouteId = null;
let createRoutePlanner = null;
let requestRouteCall = null;
let traversalCost = null;
let impassableCost = null;

/// Sixteen bytes, sixteen-byte ALIGNED, and both halves matter: the engine's own callers fill
/// this with a `movaps` of a whole `__m128` and the snap's inner loop reads it back the same way.
/// A misaligned buffer on an aligned move is a fault, not a wrong answer.
let snapPoint = null;
let snapDistance = null;

function alignTo16(pointer) {
  const slack = pointer.and(15).toUInt32();
  return slack === 0 ? pointer : pointer.add(16 - slack);
}

function bindNav() {
  try {
    keyFromArea = new NativeFunction(base.add(NAVI_GRAPH_KEY_FROM_AREA_RVA), 'uint32', ['uint32']);
    graphWorldOf = new NativeFunction(
      base.add(NAVI_GRAPH_WORLD_FROM_GAME_MANAGER_RVA), 'pointer', ['pointer']);
    graphDataForKey = new NativeFunction(
      base.add(NAVI_GRAPH_DATA_FOR_KEY_RVA), 'pointer', ['pointer', 'uint32']);
    nearestGraphId = new NativeFunction(
      base.add(NAVI_GRAPH_NEAREST_ID_RVA), 'uint32',
      ['pointer', 'pointer', 'float', 'uint32', 'pointer']);
    graphForRouteId = new NativeFunction(
      base.add(NV_NAVI_GRAPH_FOR_ROUTE_ID_RVA), 'pointer', ['pointer', 'uint32']);
    createRoutePlanner = new NativeFunction(
      base.add(NV_NAVIGATION_SYSTEM_CREATE_ROUTE_PLANNER_RVA), 'pointer',
      ['pointer', 'pointer', 'pointer', 'pointer']);
    requestRouteCall = new NativeFunction(
      base.add(NV_ROUTE_PLANNER_REQUEST_RVA), 'void',
      ['pointer', 'uint32', 'uint32', 'uint32', 'float']);
    traversalCost = new NativeFunction(
      base.add(NAVI_EDGE_TRAVERSAL_COST_RVA), 'float', ['uint32', 'uint32', 'uint8']);
    // READ THE SENTINEL, DO NOT ASSUME IT. `0x1401c1e60` compares against this exact global, so
    // the only correct "impassable" is whatever bit pattern is sitting there.
    impassableCost = base.add(NAVI_IMPASSABLE_COST_RVA).readFloat();
    snapPoint = alignTo16(Memory.alloc(32));
    snapDistance = Memory.alloc(4);
    return true;
  } catch (error) {
    console.log('[stone] the navigation calls would not bind: ' + error.message);
    return false;
  }
}

function safeRead(at, how) {
  try {
    return how(at);
  } catch (error) {
    return null;
  }
}

function readPoint(at) {
  const values = safeRead(at, function (p) {
    return [p.readFloat(), p.add(4).readFloat(), p.add(8).readFloat()];
  });
  if (values === null) return null;
  for (const value of values) {
    if (!Number.isFinite(value)) return null;
  }
  return values;
}

/// Snap a world position to a navigation-graph id, and say how it got there.
///
/// # Why this sweeps every graph instead of trusting the map key
///
/// The engine's own path is one keyed lookup, and it is only as good as the `area` fed to the key
/// builder -- which keeps six bits of a number the engine itself is inconsistent about
/// (`0x14037be30` reads `MapManager+0x170`, `0x14042c9a0` reads a byte off the character).
/// Twenty-eight meshes cannot be told apart by six bits, so at least one of those is a slot index
/// and nobody has established which. Asking every resident graph and letting the DISTANCE decide
/// is nearly free where it is redundant -- the inner call rejects a sub-graph on an AABB test
/// before allocating anything -- and correct where the key is not. The keyed lookup still runs,
/// so the log can say whether the two agreed.
function snapReporting(position) {
  const report = {
    id: null, graphs: 0, mapIndex: null, key: 0, keyedHit: false,
    chosen: null, chosenKey: null, distanceSquared: 0,
  };
  for (const value of position) {
    if (!Number.isFinite(value)) return report;
  }
  const manager = gameManager();
  if (manager === null) return report;
  const world = graphWorldOf(manager);
  if (world.isNull()) return report;

  // THE SENTINEL, TESTED BEFORE A KEY EXISTS TO BE WRONG. `0xffffffff` through the key builder
  // yields `0x3fffffff` -- a well-formed key for a map index no map has -- and the engine's own
  // `!= -1` guard cannot catch it, because the builder's range never includes `-1`.
  const mapManager = readPointer(manager.add(GAME_MANAGER_MAP_MANAGER_OFFSET));
  if (mapManager !== null) {
    const index = safeRead(mapManager.add(MAP_MANAGER_PLAYER_MAP_INDEX_OFFSET),
      function (p) { return p.readU32(); });
    if (index !== null) {
      report.mapIndex = index;
      if (index !== MAP_INDEX_NONE) {
        report.key = keyFromArea(index);
        report.keyedHit = !graphDataForKey(world, report.key).isNull();
      }
    }
  }

  const count = safeRead(world.add(NV_NAVI_GRAPH_WORLD_GRAPH_COUNT_OFFSET),
    function (p) { return p.readU32(); });
  if (count === null) return report;
  report.graphs = count;
  const bounded = Math.min(count, NV_NAVI_GRAPH_WORLD_MAX_GRAPHS);

  snapPoint.writeFloat(position[0]);
  snapPoint.add(4).writeFloat(position[1]);
  snapPoint.add(8).writeFloat(position[2]);
  snapPoint.add(12).writeFloat(0);

  // A SENTINEL, NOT `Infinity`. The comparison below decides whether a hit is kept, and
  // `anything < NaN` is false -- so a cutoff that is quietly NaN discards EVERY hit while the
  // call that produced it looks perfectly healthy in a log. `report.id === null` is tested first
  // so the first hit is kept on its own terms rather than on the arithmetic.
  let best = 0;
  for (let index = 0; index < bounded; index += 1) {
    const graph = readPointer(world.add(NV_NAVI_GRAPH_WORLD_GRAPHS_OFFSET + index * 8));
    if (graph === null) continue;
    // FLT_MAX, written as a literal. `writeFloat(Infinity)` is what the engine's own callers do
    // not do, and what comes back out of a 32-bit slot after it is not worth relying on.
    snapDistance.writeFloat(3.4028234663852886e38);
    const id = nearestGraphId(graph, snapPoint, NAVI_GRAPH_SNAP_RADIUS_METRES,
      NAVI_GRAPH_SNAP_FILTER, snapDistance);
    // SQUARED distances on both sides -- the callee writes the square and takes the root only to
    // narrow its own search. Comparing them is comparing like with like.
    const distance = snapDistance.readFloat();
    if (sweepsSaid < 2) {
      console.log('[stone] sweep ' + (sweepsSaid === 0 ? 'START' : 'GOAL') + ' at ' +
        position.map(function (v) { return v.toFixed(2); }).join(', ') +
        ' slot ' + index + ' graph ' + graph + ' radius ' +
        NAVI_GRAPH_SNAP_RADIUS_METRES + ' filter 0x' + NAVI_GRAPH_SNAP_FILTER.toString(16) +
        ' -> id ' + (id === NAVI_GRAPH_ID_NONE ? 'MISS' : '0x' + id.toString(16)) +
        ', distance^2 ' + distance);
    }
    if (id !== NAVI_GRAPH_ID_NONE && (report.id === null || distance < best)) {
      best = distance;
      report.id = id;
      report.chosen = index;
      report.distanceSquared = distance;
      const header = readPointer(graph.add(NV_NAVI_GRAPH_HEADER_OFFSET));
      report.chosenKey = header === null ? null
        : safeRead(header.add(NV_NAVI_GRAPH_HEADER_KEY_OFFSET), function (p) { return p.readU32(); });
    }
  }
  sweepsSaid += 1;
  if (report.id === null && !saidMiss && bounded > 0) {
    const first = readPointer(world.add(NV_NAVI_GRAPH_WORLD_GRAPHS_OFFSET));
    if (first !== null) {
      saidMiss = true;
      explainMiss(first, position);
    }
  }
  return report;
}

/// The miss diagnosis is printed once. Sixty of them a second buries the first.
let saidMiss = false;
/// The per-graph sweep line, once.
let sweepsSaid = 0;

/// WHY A SNAP MISSED, measured rather than reasoned about. Printed once.
///
/// A player standing on the floor of Majula being more than twenty metres from every navmesh poly
/// is not a plausible geometry -- so either the position is in a different space from the mesh,
/// or the call is not reaching the mesh at all. Those two look identical from the return value and
/// completely different in the numbers below.
///
/// `FUN_140babf90` walks sub-graphs at `[graph+0x28] + 0x40 + i*8`, `[[graph+0x28]+8]` of them,
/// and `FUN_140bac070` tests each against an AABB built from its own `+0x10..0x28`. Printing that
/// box beside the player's position answers the question outright: overlapping boxes mean the
/// filter or the radius, boxes somewhere else entirely mean the coordinate space.
function explainMiss(graph, position) {
  const header = readPointer(graph.add(NV_NAVI_GRAPH_HEADER_OFFSET));
  if (header === null) {
    console.log('[stone] MISS UNEXPLAINED: graph ' + graph + ' has no header at +0x28.');
    return;
  }
  const subCount = safeRead(header.add(8), function (p) { return p.readS32(); });
  console.log('[stone] MISS DIAGNOSIS -- graph ' + graph + ', header ' + header + ', ' +
    (subCount === null ? '?' : subCount) + ' sub-graph(s); the point that missed is ' +
    position.map(function (v) { return v.toFixed(2); }).join(', '));
  const most = Math.min(subCount === null ? 0 : subCount, 4);
  for (let index = 0; index < most; index += 1) {
    const sub = readPointer(header.add(0x40 + index * 8));
    if (sub === null) continue;
    const lo = readPoint(sub.add(0x10));
    const hi = readPoint(sub.add(0x1c));
    console.log('[stone]   sub-graph ' + index + ' ' + sub + ' AABB ' +
      (lo === null ? '?' : lo.map(function (v) { return v.toFixed(1); }).join(', ')) + '  ..  ' +
      (hi === null ? '?' : hi.map(function (v) { return v.toFixed(1); }).join(', ')));
  }
  // ESCALATING RADIUS. If ten kilometres still misses, distance is not what is wrong.
  for (const radius of [20, 100, 1000, 10000]) {
    snapDistance.writeFloat(3.4028234663852886e38);
    const id = nearestGraphId(graph, snapPoint, radius, NAVI_GRAPH_SNAP_FILTER, snapDistance);
    // A FILTER OF 0x28 KEEPS A CLASS MINIMUM OF ZERO, and `FUN_140bac070` requires
    // `(filter & 7) < (poly & 7)` -- STRICTLY -- so a poly of class 0 can never be snapped to by
    // any filter. `0x2f` is the same feature bits with the class minimum at 7, which rejects
    // everything; it is here as the control, not as a candidate.
    snapDistance.writeFloat(3.4028234663852886e38);
    const permissive = nearestGraphId(graph, snapPoint, radius, 0x38, snapDistance);
    console.log('[stone]   radius ' + radius + ' m: filter 0x28 -> ' +
      (id === NAVI_GRAPH_ID_NONE ? 'MISS' : '0x' + id.toString(16)) + ', filter 0x38 -> ' +
      (permissive === NAVI_GRAPH_ID_NONE ? 'MISS' : '0x' + permissive.toString(16)));
  }
}

function describeSnap(report) {
  if (report.id === null) {
    return 'MISS (0xffffffff -- nothing within ' + NAVI_GRAPH_SNAP_RADIUS_METRES +
      ' m on any of the ' + report.graphs + ' resident graph(s))';
  }
  // SWEEP SLOT, NOT GRAPH. This is which entry of the world's eight-pointer array won; the graph
  // identity is in the guard's keys, and printing this as "graph 0" made two ids in genuinely
  // different graphs read as though they shared one -- the exact distinction a refusal turns on.
  return '0x' + report.id.toString(16) + ' (sweep slot ' + report.chosen + ', ' +
    Math.sqrt(Math.max(report.distanceSquared, 0)).toFixed(1) + ' m off)';
}

/// The four conditions `0x140bb4310` tests BEFORE expanding a single node.
///
/// `NO ROUTE` used to be blamed on the capability mask and the cost budget, and neither is
/// reachable from the branch that refuses. The planner's step resolves each end's graph by
/// hashing `id | 0x1ffff`, and when the two differ it takes a cross-graph path that opens with
/// `area(start) == area(goal) && start_graph && goal_graph && start_links > 0 && goal_links > 0`
/// and sets READY|FAILED when any of those is false. Reading the same four turns a suspect list
/// into a named cause.
function readGuard(start, goal) {
  const manager = gameManager();
  if (manager === null) return null;
  const world = graphWorldOf(manager);
  if (world.isNull()) return null;
  // THE TABLE IS NOT THE WORLD. The planner hashes into a separate table at `+0x88`; passing the
  // world itself would read its graph array as bucket geometry.
  const table = readPointer(world.add(NV_NAVI_GRAPH_WORLD_ID_TABLE_OFFSET));
  if (table === null) return null;
  const startKey = (start | NV_ROUTE_ID_GRAPH_KEY_MASK) >>> 0;
  const goalKey = (goal | NV_ROUTE_ID_GRAPH_KEY_MASK) >>> 0;
  const startGraph = graphForRouteId(table, startKey);
  const goalGraph = graphForRouteId(table, goalKey);
  const links = function (graph) {
    if (graph.isNull()) return 0;
    const value = safeRead(graph.add(NV_NAVI_GRAPH_LINK_COUNT_OFFSET),
      function (p) { return p.readS16(); });
    return value === null ? 0 : value;
  };
  return {
    startKey: startKey,
    goalKey: goalKey,
    startGraph: startGraph,
    goalGraph: goalGraph,
    startLinks: links(startGraph),
    goalLinks: links(goalGraph),
    startArea: graphDataForKey(world, start),
    goalArea: graphDataForKey(world, goal),
  };
}

function describeGuard(guard) {
  if (guard === null) return 'guard NOT READ (one end missed, or the world was not there to ask)';
  const wrong = [];
  if (!guard.startArea.equals(guard.goalArea)) {
    wrong.push('different areas (' + guard.startArea + ' vs ' + guard.goalArea + ')');
  }
  if (guard.startGraph.isNull()) wrong.push('start key 0x' + guard.startKey.toString(16) + ' is in no graph');
  if (guard.goalGraph.isNull()) wrong.push('goal key 0x' + guard.goalKey.toString(16) + ' is in no graph');
  if (!guard.startGraph.isNull() && guard.startLinks <= 0) wrong.push('start graph has ' + guard.startLinks + ' links');
  if (!guard.goalGraph.isNull() && guard.goalLinks <= 0) wrong.push('goal graph has ' + guard.goalLinks + ' links');
  const keys = 'keys 0x' + guard.startKey.toString(16) + '/0x' + guard.goalKey.toString(16) +
    (guard.startKey === guard.goalKey ? ' (same graph)' : ' (CROSS-GRAPH)');
  return wrong.length === 0 ? 'guard OK -- ' + keys : 'guard REFUSES -- ' + keys + ': ' + wrong.join('; ');
}

/// Where the planner's finished route LIVES.
///
/// **THIS IS AN ADDRESS, NOT A DEREFERENCE, AND THE DIFFERENCE IS A WHOLE SESSION.** The Rust
/// crate next door reads a pointer out of `planner + 0x48` and decodes from the value. It is not
/// a pointer: `0x14042ee40` -- the engine's own consumer -- does
///
/// ```text
/// lVar6 = *(longlong *)(param_1 + 0x10);            ; the planner
/// FUN_140bb5cd0(..., lVar6 + 0x48);                 ; and 0x140bb5cd0 reads param_4 + 0x18
/// FUN_140bb3a10(*(longlong *)(param_1 + 0x10) + 0x48);  ; the clear path, same address-of
/// ```
///
/// so the `NvRoute` is EMBEDDED at `+0x48` and its segment count is at `planner + 0x60`. Reading
/// a pointer there yields the route's own first field, and decoding from that returns nothing --
/// which the crate then reports as NO ROUTE, indistinguishable from a search that genuinely
/// found no way through. That is the failure this agent exists to not repeat.
function routeObject(planner) {
  return planner.add(NV_ROUTE_PLANNER_ROUTE_OFFSET);
}

/// Decode a finished `NvRoute` into a world-space polyline, START FIRST.
///
/// Two reversals, and applying only one of them produces a line that is plausible, connected and
/// inside out. Both are read off `0x140bb3bd0`, the engine's own "position of route node N":
/// a route is stored GOAL-FIRST (its navigator starts at `(count - 1) << 16` and walks down), and
/// the points inside a segment are stored in reverse as well. A segment whose point count is not
/// positive still contributes one node -- its own `+0x20` -- which is the engine's branch, not an
/// edge case this invented.
function decodeRoute(route) {
  const segments = readPointer(route.add(NV_ROUTE_SEGMENTS_OFFSET));
  const count = safeRead(route.add(NV_ROUTE_SEGMENT_COUNT_OFFSET), function (p) { return p.readS32(); });
  if (segments === null || count === null || count <= 0 || count > MAX_ROUTE_SEGMENTS) return null;
  const out = [];
  for (let index = count - 1; index >= 0; index -= 1) {
    const segment = segments.add(index * NV_ROUTE_SEGMENT_STRIDE);
    const points = safeRead(segment.add(NV_ROUTE_SEGMENT_POINT_COUNT_OFFSET),
      function (p) { return p.readS16(); });
    if (points === null || points > MAX_POINTS_PER_SEGMENT) return null;
    if (points <= 0) {
      const only = readPoint(segment.add(NV_ROUTE_SEGMENT_POINT_OFFSET));
      if (only === null) return null;
      out.push(only);
      continue;
    }
    const array = readPointer(segment.add(NV_ROUTE_SEGMENT_POINTS_OFFSET));
    if (array === null) return null;
    for (let point = points - 1; point >= 0; point -= 1) {
      if (out.length >= MAX_ROUTE_POINTS) return null;
      const at = readPoint(array.add(point * ROUTE_POINT_STRIDE));
      if (at === null) return null;
      out.push(at);
    }
  }
  return out.length === 0 ? null : out;
}

/// The attribute word the engine tests for one packed navi id, or `null` if it cannot be read.
///
/// The two-step lookup -- hash `id | 0x1ffff` to a graph, then index that graph's `+0x48` table
/// by `id & 0x7fff` -- is `0x14042ee40`'s, quoted at [`NV_NAVI_GRAPH_NODE_ATTRS_OFFSET`]. Doing
/// it any other way (indexing the graph the SNAP happened to choose, say) reads one graph's
/// table with another graph's index, and a wrong `u32` here reads as a confident verdict.
function nodeAttributes(table, id) {
  const index = id & NAVI_ID_INDEX_MASK;
  if (index === NAVI_ID_INDEX_NONE) return null;
  const graph = graphForRouteId(table, (id | NV_ROUTE_ID_GRAPH_KEY_MASK) >>> 0);
  if (graph === null || graph.isNull()) return null;
  const attrs = readPointer(graph.add(NV_NAVI_GRAPH_NODE_ATTRS_OFFSET));
  if (attrs === null) return null;
  return safeRead(attrs.add(index * 4), function (p) { return p.readU32(); });
}

/// Ask the engine's own predicate whether an agent of `size` may use a node, with `below` saying
/// whether the edge sits under the traveller. See [`NAVI_NODE_TYPE_GATED`] for why that is a
/// height and not a direction.
function nodePassable(attrs, size, below) {
  const capability = ((size & 0x7) | NV_ROUTE_CAPABILITY_FEATURES) >>> 0;
  return traversalCost(attrs, capability, below) !== impassableCost;
}

/// Name every node on a finished route that a real character could not use, and say plainly what
/// this test can and cannot see.
///
/// # Why this exists
///
/// The complaint it answers is "the trail leads up to and through things that block me". There
/// are two different causes with the same appearance, and only one of them is discoverable from
/// the navigation data:
///
///   * **The route uses nodes sized for a smaller agent, or crosses a one-way drop backwards.**
///     Discoverable, exactly, right here: the request's capability word is `0x7f8`, whose size
///     class is ZERO -- the smallest agent the format can describe -- and `0x140baf0d0` admits a
///     node only while `(capability & 7) < (attrs & 7)`. So the search was told the traveller is
///     a mouse. Re-running the same predicate at each of the seven size classes measures how big
///     the traveller may actually be before the route stops existing.
///   * **A shut door, a crate, a fog gate, a boulder.** NOT discoverable here, and this function
///     says so rather than letting silence imply a clean bill. `0x140baf0d0` takes a baked `u32`
///     and the capability word. No world pointer, no object list, no time. `0x1401c1e60`, the
///     `NvNaviGraphCostEstimator` wrapper RTTI names, is one comparison on its return value.
///     A route straight through a locked door is a CORRECT route by every question the planner
///     is able to ask, so catching that one needs the collision world, not the navigation world.
function auditRoute(routeAt) {
  if (traversalCost === null || impassableCost === null) return null;
  // A NaN sentinel would make `cost !== impassable` true for every node and report a route with
  // no problems at all -- the same silent-pass shape that hid a working snap for a whole run.
  // Refuse instead of reporting.
  if (impassableCost !== impassableCost) {
    return { broken: 'the impassable-cost sentinel read back as NaN, so every comparison ' +
      'against it would answer PASSABLE. No audit is better than one that always agrees.' };
  }
  const manager = gameManager();
  if (manager === null) return null;
  const world = graphWorldOf(manager);
  if (world.isNull()) return null;
  const table = readPointer(world.add(NV_NAVI_GRAPH_WORLD_ID_TABLE_OFFSET));
  if (table === null) return null;
  const segments = readPointer(routeAt.add(NV_ROUTE_SEGMENTS_OFFSET));
  const count = safeRead(routeAt.add(NV_ROUTE_SEGMENT_COUNT_OFFSET), function (p) { return p.readS32(); });
  if (segments === null || count === null || count <= 0 || count > MAX_ROUTE_SEGMENTS) return null;

  const nodes = [];
  let unreadable = 0;
  const seen = {};
  for (let index = 0; index < count; index += 1) {
    const segment = segments.add(index * NV_ROUTE_SEGMENT_STRIDE);
    const ends = [
      { idAt: NV_ROUTE_SEGMENT_NODE_A_OFFSET, pointAt: NV_ROUTE_SEGMENT_POINT_A_OFFSET, side: 'A' },
      { idAt: NV_ROUTE_SEGMENT_NODE_B_OFFSET, pointAt: NV_ROUTE_SEGMENT_POINT_OFFSET, side: 'B' },
    ];
    for (let end = 0; end < ends.length; end += 1) {
      const id = safeRead(segment.add(ends[end].idAt), function (p) { return p.readU32(); });
      if (id === null || id === NAVI_GRAPH_ID_NONE) continue;
      if (seen[id] === true) continue;
      seen[id] = true;
      const attrs = nodeAttributes(table, id);
      if (attrs === null) { unreadable += 1; continue; }
      nodes.push({
        id: id,
        attrs: attrs,
        type: attrs & NAVI_NODE_TYPE_MASK,
        capacity: attrs & 0x7,
        segment: index,
        side: ends[end].side,
        at: readPoint(segment.add(ends[end].pointAt)),
      });
    }
  }
  if (nodes.length === 0) return { broken: 'no node id on any segment could be resolved' };

  // The largest agent that still gets this whole route. Bigger is MORE restricted, because the
  // predicate wants the agent's class strictly below the node's capacity.
  let widest = -1;
  for (let size = 0; size < NV_ROUTE_CAPABILITY_SIZE_CLASSES; size += 1) {
    let all = true;
    for (let n = 0; n < nodes.length; n += 1) {
      if (!nodePassable(nodes[n].attrs, size, 0)) { all = false; break; }
    }
    if (all) widest = size; else break;
  }

  const trouble = [];
  for (let n = 0; n < nodes.length; n += 1) {
    const node = nodes[n];
    const level = nodePassable(node.attrs, 0, 0);
    const below = nodePassable(node.attrs, 0, 1);
    const gated = NAVI_NODE_TYPE_GATED.indexOf(node.type) !== -1;
    // A node the smallest possible agent cannot use at all, or one whose answer flips with the
    // traveller's height -- a ledge you may drop from and not climb back up.
    if (!level || level !== below || gated) {
      trouble.push({ node: node, level: level, below: below, gated: gated });
    }
  }
  return { nodes: nodes, unreadable: unreadable, widest: widest, trouble: trouble, segments: count };
}

function reportAudit(audit, sayTheLimit) {
  if (audit === null) return;
  if (audit.broken !== undefined) {
    console.log('[stone] route audit could not run -- ' + audit.broken);
    return;
  }
  console.log('[stone] route audit -- ' + audit.segments + ' segment(s), ' + audit.nodes.length +
    ' distinct node(s) read' + (audit.unreadable === 0 ? '' : ', ' + audit.unreadable + ' unreadable') +
    '. Widest agent this route admits: size class ' +
    (audit.widest < 0 ? 'NONE (not even the smallest -- read that as a wrong attribute table, ' +
      'not as a fact about the world)' : audit.widest + ' of ' +
      (NV_ROUTE_CAPABILITY_SIZE_CLASSES - 1)) +
    '. The request asked as size class 0, the smallest the format has.');
  // THE RAW WORDS, ALWAYS. A verdict with no evidence under it cannot be checked, and the
  // vocabulary of this table is not documented anywhere -- it gets learned by watching which
  // values turn up on routes that behave and routes that do not.
  const sample = [];
  for (let index = 0; index < audit.nodes.length && index < 10; index += 1) {
    const node = audit.nodes[index];
    sample.push('0x' + ('00000000' + node.attrs.toString(16)).slice(-8) +
      '(t' + node.type.toString(16) + '/c' + node.capacity + ')');
  }
  console.log('[stone]   attrs: ' + sample.join(' ') +
    (audit.nodes.length > 10 ? ' ... ' + (audit.nodes.length - 10) + ' more' : ''));
  for (let index = 0; index < audit.trouble.length && index < 12; index += 1) {
    const item = audit.trouble[index];
    const node = item.node;
    console.log('[stone]   node 0x' + node.id.toString(16) + ' (segment ' + node.segment + node.side +
      (node.at === null ? '' : ' at ' + node.at[0].toFixed(2) + ', ' + node.at[1].toFixed(2) +
        ', ' + node.at[2].toFixed(2)) +
      ') attrs 0x' + ('00000000' + node.attrs.toString(16)).slice(-8) +
      ' type 0x' + node.type.toString(16) + ' capacity ' + node.capacity +
      (item.level !== item.below
        ? ' -- ASYMMETRIC, usable only when the edge sits ' + (item.below ? 'BELOW you (a drop, ' +
          'not a climb)' : 'LEVEL WITH OR ABOVE you')
        // NOT "we are cheating here". `0x14042ee40` ORs `0x7f8` onto EVERY character's size, so
        // the feature bits are identical for us and for every NPC in the game; a gated type that
        // passes both ways passes for them too. The only per-character knob in the whole
        // capability word is the three-bit size class.
        : item.level ? ' -- gated type, passable both ways at 0x7f8, which is what every NPC has'
          : ' -- IMPASSABLE even to the smallest agent'));
  }
  if (audit.trouble.length > 12) {
    console.log('[stone]   ... and ' + (audit.trouble.length - 12) + ' more.');
  }
  // THE HONEST HALF. Silence here would read as "nothing else is wrong", which is not what this
  // test measured and not what it is able to measure. Said once a session rather than once a
  // route, because a paragraph that repeats is a paragraph nobody reads.
  if (!sayTheLimit) return;
  console.log('[stone]   Scope of this audit: it reads the per-node attribute word LIVE, so ' +
    'anything that rewrites that word -- and this engine has an NvNaviGraphCostUpdater and a ' +
    'MapObjNaviGraphLocationComponent, both runtime-shaped -- shows up here. What it does NOT ' +
    'cover is the GATE layer (NvNaviGraphGate, NvNaviGatePathFindingTask) or plain collision ' +
    'geometry that was never registered with the navigation graph at all. Neither of those is ' +
    'ruled out by a clean line above.');
}

/// Read a planner's flags, and decode the route when there is one.
///
/// # The order of the two bit tests is the whole function
///
/// All three of the engine's give-up paths end with `or byte [planner+0x30], 6` -- READY and
/// FAILED in one write. Testing READY first therefore believes every failure and goes on to read
/// a route those paths never built.
function pollRoute(planner) {
  const flags = safeRead(planner.add(NV_ROUTE_PLANNER_FLAGS_OFFSET), function (p) { return p.readU8(); });
  if (flags === null) return { state: 'lost' };
  if ((flags & NV_ROUTE_PLANNER_FLAG_FAILED) !== 0) return { state: 'failed' };
  if ((flags & NV_ROUTE_PLANNER_FLAG_READY) === 0) return { state: 'pending' };
  const route = routeObject(planner);
  const segments = safeRead(route.add(NV_ROUTE_SEGMENT_COUNT_OFFSET), function (p) { return p.readS32(); });
  const points = decodeRoute(route);
  // BOTH NUMBERS, ALWAYS. Forty segments decoding to two points means the decoder is wrong; two
  // and two means the walk really is that short. One number cannot tell those apart.
  if (points === null) return { state: 'failed', segments: segments === null ? -1 : segments, decoded: false };
  return { state: 'ready', points: points, segments: segments === null ? -1 : segments };
}

/// Points every `step` metres ALONG a polyline, rather than along the straight line between its
/// ends. This is the whole difference between a trail that follows the ground and one that does
/// not: the route's own vertices sit on the navmesh, so every interpolated point between two
/// adjacent ones does too.
function resampleAlong(points, step, most) {
  const spots = [];
  if (points.length < 2) return spots;
  let carried = 0;
  for (let index = 1; index < points.length && spots.length < most; index += 1) {
    const from = points[index - 1];
    const to = points[index];
    const leg = apart(from, to);
    if (!(leg > 1e-6)) continue;
    let walked = step - carried;
    while (walked <= leg && spots.length < most) {
      const t = walked / leg;
      spots.push([
        from[0] + (to[0] - from[0]) * t,
        from[1] + (to[1] - from[1]) * t,
        from[2] + (to[2] - from[2]) * t,
      ]);
      walked += step;
    }
    carried = leg - (walked - step);
  }
  return spots;
}

function pathLength(points) {
  let total = 0;
  for (let index = 1; index < points.length; index += 1) total += apart(points[index - 1], points[index]);
  return total;
}

function attach() {
  if (base === null) {
    console.log('[stone] REFUSING: the game image was not found, so every address here is wrong.');
    return;
  }
  console.log('[stone] game image: ' + base);

  const prologue = prologueMatches();
  if (!prologue.ok) {
    console.log('[stone] REFUSING TO HOOK: ' + prologue.why);
    return;
  }
  console.log('[stone] NvNavigationSystem::Update prologue matches: ' + prologue.hex);

  if (!bindCalls()) return;
  // REFUSE RATHER THAN FALL BACK. Without `VirtualAlloc` the only other storage is the script's
  // own, which `ds2-frida-watch.py` frees on every reload while the engine still points into it.
  // Silently using it would trade a refusal now for a crash on some later save.
  // REFUSE RATHER THAN LAY A STRAIGHT LINE. Without these the only trail available is the one
  // through walls, and laying it would look like success.
  if (!bindNav()) {
    console.log('[stone] REFUSING: without the navigation calls the only trail available is a ' +
      'straight line through the scenery, which is the thing this is meant to stop doing.');
    return;
  }
  if (!bindAllocator()) {
    console.log('[stone] REFUSING: no VirtualAlloc, and the script\'s own memory is freed on ' +
      'reload while the engine still links it.');
    return;
  }

  // PRESENT IS NOT HOOKED HERE, deliberately. `arrow.js` hooks it in its own Frida session, and
  // two sessions patching the same function is a real way to lose the game -- for a number that
  // can be had by other means. bd `ds2-mods-rs-zbo` wants the COMPARISON, not the mechanism: each
  // agent prints the thread it runs on and the two logs are read side by side.

  Interceptor.attach(base.add(NAV_UPDATE_RVA), {
    onEnter(args) {
      navTicks += 1;
      if (navThread === null) navThread = Process.getCurrentThreadId();
      // The `NvNavigationSystem` the engine is stepping THIS tick, taken from `rcx`. Not read
      // from `GameManagerImp + 0xBC0`: two sources of truth for one pointer is how a planner
      // ends up linked onto a list nobody walks.
      navSystemAt = args[0];
    },
    // EVERYTHING ELSE RUNS AFTER THE ORIGINAL, and that is not tidiness. Creating a planner
    // pushes onto the head of the intrusive list the original is walking; doing it from inside
    // that walk corrupts the walk.
    onLeave() {
      if (pending !== null) {
        const asked = pending;
        pending = null;
        try {
          serve(asked);
        } catch (error) {
          console.log('[stone] ' + asked + ' THREW: ' + error.message + '\n' + error.stack);
        }
      }
      if (STAGE !== 'follow' || passStopped !== null) return;
      if (route.planner === null && navSystemAt !== null) {
        try {
          const made = createRoutePlanner(navSystemAt, NULL, NULL, NULL);
          if (!made.isNull()) {
            route.planner = made;
            if (!route.saidPlanner) {
              route.saidPlanner = true;
              const listed = safeRead(navSystemAt.add(NV_NAVIGATION_SYSTEM_LIST_COUNT_OFFSET),
                function (p) { return p.readS32(); });
              console.log('[stone] route planner ' + made + ' linked -- ' +
                (listed === null ? '?' : listed) + ' object(s) on the navigation system\'s list. ' +
                'From here the ENGINE steps it every frame; nothing in this agent calls its Update.');
            }
          }
        } catch (error) {
          passStopped = error.message;
          console.log('[stone] MAKING A PLANNER THREW: ' + error.message + '\n' + error.stack);
          return;
        }
      }
      // ONE PASS EVERY TEN TICKS. `er_invasion_path::ROSTER_EVERY_TICKS`, which is what the
      // three-stones-per-pass rate above is counted against; doing this every tick would lay the
      // whole trail in half a second and defeat the point of laying it gradually.
      if (navTicks % ROSTER_EVERY_TICKS !== 0) return;
      try {
        pass();
      } catch (error) {
        // STOP AFTER THE FIRST ONE. Sixty of these a second buries its own first occurrence,
        // which is the only one that says what actually went wrong.
        passStopped = error.message;
        console.log('[stone] THE TRAIL PASS THREW AND IS NOW OFF: ' + error.message + '\n' +
          error.stack);
      }
    },
  });
  console.log('[stone] hooked NvNavigationSystem::Update at ' + base.add(NAV_UPDATE_RVA) +
    '; stage is "' + STAGE + '"');
}

setInterval(function () {
  const found = sfxSystem();
  console.log('[stone] ' + navTicks + ' nav ticks, stage "' + STAGE + '"' +
    (pending === null ? '' : ' (waiting for a tick to serve "' + pending + '")') +
    (STAGE !== 'follow' ? '' :
      '\n[stone]   trail: ' + trail.placed.length + ' stones standing, ' + trail.laid + ' of ' +
      trail.spots.length + ' laid, ' + laidTotal + ' laid in all, ' + removedTotal +
      ' put out, ' + forgottenTotal + ' let go of, ' + arenaPages + ' pages, from the ' + route.source +
      (route.pending ? ' (a search is outstanding, ' + route.waited + ' pass(es) in)' : '') +
      (passStopped === null ? '' : '\n[stone]   PASSES ARE OFF: ' + passStopped)));
  if (!reported) { console.log('[stone] ' + describeSystem(found)); reported = true; }
  if (navTicks === 0) {
    console.log('[stone] NO NAV TICKS. Either the hook did not take or nothing is driving the ' +
      'navigation system in this area -- and nothing can be spawned from a seam that never runs.');
  }
  // HALF OF bd `ds2-mods-rs-zbo`'s THREAD QUESTION. The other half is the number `arrow.js`
  // prints for Present, in its own log. Equal means the queue between the two seams in
  // `crate::gametick` is belt-and-braces; different means it is load-bearing and every `try_lock`
  // in it starts mattering. The static evidence says equal.
  if (!threadsCompared && navThread !== null) {
    threadsCompared = true;
    console.log('[stone] NvNavigationSystem::Update runs on thread ' + navThread +
      ' -- compare against the Present thread in the arrow log.');
  }
  // Re-read every window, because "ready" flips on an area load and a stage served before it
  // does is a stage that quietly did nothing.
  if (found.why === undefined && found.quality >= SFX_QUALITY_DROP_THRESHOLD) {
    console.log('[stone] quality is ' + found.quality + ' RIGHT NOW -- spawns are being dropped.');
  }
}, 5000);

attach();
