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

/// Points every `step` metres along the straight line from `from` to `to`.
///
/// A STRAIGHT LINE, NOT A ROUTE, and the difference matters: this proves the spawn and the
/// cadence, not the pathfinding. A line through a wall is still a line. The walkable route is
/// `NvRoutePlanner`'s job and is separately stuck on `NO ROUTE` -- bd `ds2-mods-rs-zbo`.
function resample(from, to, step, most) {
  const total = apart(from, to);
  if (!(total > 1e-6)) return [];
  const count = Math.min(most, Math.floor(total / step));
  const spots = [];
  for (let index = 1; index <= count; index += 1) {
    const t = (index * step) / total;
    spots.push([
      from[0] + (to[0] - from[0]) * t,
      from[1] + (to[1] - from[1]) * t,
      from[2] + (to[2] - from[2]) * t,
    ]);
  }
  return spots;
}

/// How far the character may move in one pass before it counts as a load rather than a walk.
///
/// A sixth of a second of running is a couple of metres; a death, a bonfire warp or an area
/// transition is hundreds. The stones from before such a jump belong to an area that has been
/// torn down, and the pointers in them are the engine's freed memory -- so they are FORGOTTEN
/// rather than put out. Chosen well above anything a player can cover in a pass and well below
/// any real transition.
const LOAD_JUMP_METRES = 100.0;

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

  // THE ROUTE, every `ROUTE_REFRESH_TICKS` and not more often. A pass is for LAYING; re-asking
  // where the target is on every pass would replace the trail faster than it can be walked.
  if (navTicks - trail.computedAt >= ROUTE_REFRESH_TICKS) {
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
      // Retargeting first is what stops the pile-up: a route that has not moved keeps the stones
      // it already has instead of getting a second set on top. A route that HAS moved takes its
      // old stones down before laying new ones.
      if (retarget(resample(player, target.position, MARKER_SPACING_METRES, MAX_MARKERS))) {
        removedTotal += clearPlaced();
      }
    }
  }

  // Stones you have already walked past are clutter behind you, so they go AS YOU PASS THEM
  // rather than waiting for the whole route to change.
  removedTotal += pruneBehind(player, MARKER_KEEP_BEHIND_METRES);

  if (trail.laid < trail.spots.length) laidTotal += layNext(found.system, MARKERS_PER_PASS);
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
    onEnter() {
      navTicks += 1;
      if (navThread === null) navThread = Process.getCurrentThreadId();
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
      ' put out, ' + forgottenTotal + ' let go of, ' + arenaPages + ' pages' +
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
