// The live view-projection, taken from the upload the renderer actually makes.
//
// WHY NOT THE GAME'S OBJECTS. `camera-matrix.js` sweeps `CameraOperator` and finds real
// projections (`+0x50`, x 1.0789 / y 1.9210, aspect 1.7806) and real view matrices (`+0x10`,
// identity, eye `(0,0,10)`) -- but no `view * projection` product anywhere. The world camera is
// composed somewhere those offsets do not name. The renderer does not have that problem: whatever
// it composes, it uploads.
//
// THE BUG IN THE FIRST ATTEMPT. `UpdateSubresource` was hooked and the counter read 4,102,421
// calls with 0 buffers scanned: every one was rejected on `bytes = args[5]`, because `srcRowPitch`
// is IGNORED for a buffer resource and the game passes 0. The hook was fine; the size was nonsense.
//
// THE BUG IN THE SECOND ATTEMPT, WHICH IS WHAT THIS FILE FIXES. A single-frame filter cannot tell
// a camera from a shader parameter blob. Over a kilobyte at four-byte stride, times two
// conventions, tens of thousands of windows are examined per second; some of them will be dense,
// will have a unit column, and will happen to put the player near the middle of the frame. Seven
// were "found" that way and every one had a column of exact zeros -- 0.15/0.05/0.8/1.7806/235.0
// material constants read sideways.
//
// So the test here is NOT a filter, it is a FOLLOW. A candidate is keyed by the destination
// RESOURCE it was uploaded to, and that resource is then scanned on every subsequent call rather
// than sampled. A real view-projection is re-uploaded every frame and keeps the character centred
// at a few metres while the camera spins and the character walks. A blob that scored once by
// accident cannot score two hundred times in a row across a moving player -- and if the player
// never moved, the run says so instead of claiming a pass, because `spread` is reported.

'use strict';

const GAME_MANAGER_IMP_RVA = 0x016148f0;
const GAME_MANAGER_PLAYER_CTRL_OFFSET = 0xd0;
const CHARACTER_CTRL_POSITION_OFFSET = 0x90;

const SLOT_UPDATE_SUBRESOURCE = 48;
const SLOT_MAP = 14;

/// How far into an upload to look. A constant buffer holding a camera is small; this covers the
/// usual 256-byte register window several times over without reading into another allocation.
const SCAN_BYTES = 512;

const UNIT_TOLERANCE = 0.02;

/// How many times a DISTINCT destination resource is scanned before it is written off.
///
/// NOT A SAMPLE RATE, AND THE DIFFERENCE IS THE WHOLE POINT. The first version sampled one upload
/// in 50021 because a full scan inside a hook that runs 4.1 million times starves Frida's JS
/// thread (measured: even `setInterval` stops, which reads exactly like a hung game while the game
/// is fine). But the game uploads ~757 buffers per frame, so a blind sample lands on the camera's
/// buffer with probability 1/757 per look -- it examined 24 buffers in 25 seconds and found
/// nothing, which is the expected result of that arithmetic, not evidence about the camera.
///
/// Coverage by resource costs the same and cannot miss: the expensive scan runs a bounded number
/// of times per distinct resource no matter how often that resource is written, and every
/// resource gets looked at. Twenty looks rather than one because a single look can fail for
/// reasons that have nothing to do with the matrix -- the character momentarily unreadable, a
/// frame where the buffer holds the previous pass's contents.
const LOOKS_PER_RESOURCE = 20;

/// How far a third-person camera sits from the character, in metres. `clip.w` is exactly that
/// distance, so these bound it. Generous on both sides: a wall shoves the camera in, a lock-on
/// pulls it out, and neither reaches twenty metres.
const CAMERA_MIN_METERS = 0.5;
const CAMERA_MAX_METERS = 12.0;

/// Consecutive passes before a candidate is believed. At 60 fps this is a couple of seconds of
/// continuous agreement.
const PASSES_TO_BELIEVE = 200;

/// Metres the character must have travelled across those passes. Without this, standing still
/// would let a constant matrix "agree" forever.
const SPREAD_TO_BELIEVE = 1.5;

const gameModule = Process.findModuleByName('DarkSoulsII.exe');
const base = gameModule === null ? null : gameModule.base;

// An instrument that cannot tell "found nothing" from "never ran" is worse than no instrument.
const counters = { update: 0, map: 0, scanned: 0, framed: 0, discovered: 0, followed: 0 };

/// Discovery scans allowed per heartbeat window. A backstop, not a policy: if the renderer churns
/// through fresh resource pointers every frame, per-resource coverage degenerates into scanning
/// everything, and scanning everything is what wedges the JS thread. The heartbeat reports when
/// this bites so it cannot quietly cap coverage and read as "looked everywhere".
const SCANS_PER_WINDOW = 400;

/// resource pointer (as string) -> { label, offsets: Map<key, candidate> }
const tracked = new Map();
/// resource pointer (as string) -> how many times it has been scanned during discovery.
const examined = new Map();
let scansThisWindow = 0;
let throttled = 0;
let believed = null;

/** True once, for each of a resource's first `LOOKS_PER_RESOURCE` appearances. */
function shouldDiscover(key) {
  const looks = examined.get(key) || 0;
  if (looks >= LOOKS_PER_RESOURCE) return false;
  if (scansThisWindow >= SCANS_PER_WINDOW) {
    throttled += 1;
    return false;
  }
  examined.set(key, looks + 1);
  scansThisWindow += 1;
  return true;
}

function readFloats(pointer, count) {
  const out = [];
  for (let index = 0; index < count; index += 1) {
    out.push(pointer.add(index * 4).readFloat());
  }
  return out;
}

function finite(values) {
  return values.every((value) => Number.isFinite(value) && Math.abs(value) < 1e12);
}

function playerPosition() {
  if (base === null) return null;
  try {
    const manager = base.add(GAME_MANAGER_IMP_RVA).readPointer();
    if (manager.isNull()) return null;
    const player = manager.add(GAME_MANAGER_PLAYER_CTRL_OFFSET).readPointer();
    if (player.isNull()) return null;
    const position = readFloats(player.add(CHARACTER_CTRL_POSITION_OFFSET), 3);
    return finite(position) ? position : null;
  } catch (error) {
    return null;
  }
}

function toClip(m, world) {
  const [x, y, z] = world;
  return [
    x * m[0] + y * m[4] + z * m[8] + m[12],
    x * m[1] + y * m[5] + z * m[9] + m[13],
    x * m[2] + y * m[6] + z * m[10] + m[14],
    x * m[3] + y * m[7] + z * m[11] + m[15],
  ];
}

function format(m) {
  const cell = (value) => value.toFixed(4).padStart(10);
  return (
    '\n      [' + m.slice(0, 4).map(cell).join(' ') +
    '\n       ' + m.slice(4, 8).map(cell).join(' ') +
    '\n       ' + m.slice(8, 12).map(cell).join(' ') +
    '\n       ' + m.slice(12, 16).map(cell).join(' ') + ' ]'
  );
}

/** Both conventions, because HLSL packs `float4x4` column-major and a row-vector engine uploads
 *  the transpose. Whichever scores is the one the crate must read. */
function transpose(m) {
  const out = [];
  for (let row = 0; row < 4; row += 1) {
    for (let column = 0; column < 4; column += 1) out.push(m[column * 4 + row]);
  }
  return out;
}

function score(m, player) {
  if (!finite(m)) return null;
  const forward = Math.sqrt(m[3] * m[3] + m[7] * m[7] + m[11] * m[11]);
  if (Math.abs(forward - 1.0) > UNIT_TOLERANCE) return null;
  // EVERY WORLD AXIS MUST REACH THE SCREEN. A view-projection has a rotation multiplied through
  // it, so none of the three world-axis rows can be degenerate. Every false positive this probe
  // has ever reported failed exactly here: each had one row of literal 0.0000, because it was a
  // run of material constants, not a transform.
  for (let row = 0; row < 3; row += 1) {
    const a = m[row * 4], b = m[row * 4 + 1], c = m[row * 4 + 2];
    if (Math.sqrt(a * a + b * b + c * c) < 0.05) return null;
  }
  const clip = toClip(m, player);
  // `clip.w` IS THE CAMERA-TO-PLAYER DISTANCE, and it is the strongest single test here because it
  // is physical rather than statistical. The third-person camera sits a few metres behind the
  // character; it is never twenty.
  if (!(clip[3] > CAMERA_MIN_METERS && clip[3] < CAMERA_MAX_METERS)) return null;
  const ndcX = clip[0] / clip[3];
  const ndcY = clip[1] / clip[3];
  if (!Number.isFinite(ndcX) || !Number.isFinite(ndcY)) return null;
  if (Math.abs(ndcX) > 0.25 || Math.abs(ndcY) > 0.45) return null;
  // A metre must move the screen by a sane amount on ALL THREE axes, not two: two is what a matrix
  // that collapses one world direction still passes.
  for (const axis of [[1, 0, 0], [0, 1, 0], [0, 0, 1]]) {
    const near = toClip(m, [player[0] + axis[0], player[1] + axis[1], player[2] + axis[2]]);
    if (!(near[3] > 0.05)) return null;
    const shift = Math.hypot(near[0] / near[3] - ndcX, near[1] / near[3] - ndcY);
    if (!(Number.isFinite(shift) && shift > 0.005 && shift < 2.0)) return null;
  }
  return { ndcX, ndcY, w: clip[3], forward };
}

function entry(offset, convention) {
  return {
    offset,
    convention,
    passes: 0,
    best: null,
    minimum: [Infinity, Infinity, Infinity],
    maximum: [-Infinity, -Infinity, -Infinity],
  };
}

function spreadOf(candidate) {
  let worst = 0;
  for (let axis = 0; axis < 3; axis += 1) {
    const span = candidate.maximum[axis] - candidate.minimum[axis];
    if (Number.isFinite(span) && span > worst) worst = span;
  }
  return worst;
}

function note(candidate, verdict, player) {
  candidate.passes += 1;
  candidate.best = verdict;
  for (let axis = 0; axis < 3; axis += 1) {
    if (player[axis] < candidate.minimum[axis]) candidate.minimum[axis] = player[axis];
    if (player[axis] > candidate.maximum[axis]) candidate.maximum[axis] = player[axis];
  }
}

/** Every 16-float window in a freshly-seen buffer, both conventions. Discovery only. */
function discover(resource, source, label, player) {
  counters.scanned += 1;
  const key = resource.toString();
  for (let offset = 0; offset + 64 <= SCAN_BYTES; offset += 4) {
    let raw;
    try {
      raw = readFloats(source.add(offset), 16);
    } catch (error) {
      return;
    }
    for (const [convention, m] of [['as stored', raw], ['transposed', transpose(raw)]]) {
      const verdict = score(m, player);
      if (verdict === null) continue;
      counters.framed += 1;
      let slot = tracked.get(key);
      if (slot === undefined) {
        slot = { key, label, offsets: new Map() };
        tracked.set(key, slot);
      }
      const name = offset + '|' + convention;
      if (!slot.offsets.has(name)) {
        slot.offsets.set(name, entry(offset, convention));
        counters.discovered += 1;
      }
      note(slot.offsets.get(name), verdict, player);
    }
  }
}

/** A resource we are already following: score only its known offsets, every single call. */
function follow(slot, source, player) {
  counters.followed += 1;
  for (const [name, candidate] of slot.offsets) {
    let raw;
    try {
      raw = readFloats(source.add(candidate.offset), 16);
    } catch (error) {
      return;
    }
    const m = candidate.convention === 'transposed' ? transpose(raw) : raw;
    const verdict = score(m, player);
    if (verdict === null) {
      // ONE MISS RESETS IT. The camera buffer is correct on every frame it is uploaded; a blob
      // that drifts in and out of the gate is not a camera, and a streak that survives misses
      // would be counting coincidences.
      slot.offsets.delete(name);
      continue;
    }
    note(candidate, verdict, player);
    if (believed !== null) continue;
    if (candidate.passes < PASSES_TO_BELIEVE) continue;
    if (spreadOf(candidate) < SPREAD_TO_BELIEVE) continue;
    believed = { slot, candidate, matrix: m };
    console.log(
      '\n[vp] ===== VIEW-PROJECTION CONFIRMED =====\n' +
      '[vp] ' + slot.label + ' resource ' + slot.key + ' +0x' + candidate.offset.toString(16) +
      ' ' + candidate.convention + '\n' +
      '[vp] ' + candidate.passes + ' consecutive frames, character moved ' +
      spreadOf(candidate).toFixed(2) + ' m across them\n' +
      '[vp] player ndc ' + verdict.ndcX.toFixed(4) + ',' + verdict.ndcY.toFixed(4) +
      ' (w=' + verdict.w.toFixed(2) + ' m, forward=' + verdict.forward.toFixed(5) + ')' +
      format(m)
    );
  }
}

function hook() {
  const module = Process.findModuleByName('d3d11.dll');
  if (module === null) {
    console.log('[vp] d3d11.dll not loaded; nothing hooked.');
    return;
  }
  const create = module.findExportByName('D3D11CreateDevice');
  if (create === null) {
    console.log('[vp] no D3D11CreateDevice; nothing hooked.');
    return;
  }
  const createDevice = new NativeFunction(create, 'int', [
    'pointer', 'int', 'pointer', 'uint', 'pointer', 'uint', 'uint',
    'pointer', 'pointer', 'pointer',
  ]);
  const devicePointer = Memory.alloc(Process.pointerSize);
  const contextPointer = Memory.alloc(Process.pointerSize);
  const result = createDevice(NULL, 1, NULL, 0, NULL, 0, 7, devicePointer, NULL, contextPointer);
  if (result < 0 || contextPointer.readPointer().isNull()) {
    console.log('[vp] D3D11CreateDevice failed: 0x' + (result >>> 0).toString(16));
    return;
  }
  const vtable = contextPointer.readPointer().readPointer();

  Interceptor.attach(vtable.add(SLOT_UPDATE_SUBRESOURCE * Process.pointerSize).readPointer(), {
    onEnter(args) {
      counters.update += 1;
      if (believed !== null) return;
      const resource = args[1];
      const source = args[4];
      if (resource.isNull() || source.isNull()) return;
      const slot = tracked.get(resource.toString());
      if (slot !== undefined) {
        // NOT SAMPLED. Following costs one lookup plus a handful of floats, and the whole point is
        // to see this resource on every frame it is written.
        const player = playerPosition();
        if (player === null) return;
        follow(slot, source, player);
        return;
      }
      // NO SIZE CHECK ON `srcRowPitch`. It is ignored for a buffer and the game passes zero; the
      // first version of this rejected 4.1 million uploads on it.
      if (!shouldDiscover(resource.toString())) return;
      const player = playerPosition();
      if (player === null) return;
      discover(resource, source, 'UpdateSubresource', player);
    },
  });

  Interceptor.attach(vtable.add(SLOT_MAP * Process.pointerSize).readPointer(), {
    onEnter(args) {
      this.resource = args[1];
      this.mapped = args[5];
    },
    onLeave() {
      counters.map += 1;
      if (believed !== null) return;
      if (!this.mapped || this.mapped.isNull()) return;
      if (!this.resource || this.resource.isNull()) return;
      let data;
      try {
        data = this.mapped.readPointer();
      } catch (error) {
        return;
      }
      if (data.isNull()) return;
      const slot = tracked.get(this.resource.toString());
      if (slot !== undefined) {
        const player = playerPosition();
        if (player === null) return;
        follow(slot, data, player);
        return;
      }
      if (!shouldDiscover(this.resource.toString())) return;
      const player = playerPosition();
      if (player === null) return;
      discover(this.resource, data, 'Map', player);
    },
  });

  console.log('[vp] hooked UpdateSubresource (48) and Map (14); following by resource.');
}

setInterval(function () {
  const leaders = [];
  for (const [key, slot] of tracked) {
    for (const [, candidate] of slot.offsets) {
      leaders.push({ key, slot, candidate });
    }
  }
  leaders.sort((a, b) => b.candidate.passes - a.candidate.passes);
  let line =
    '[vp] heartbeat: ' + counters.update + ' update, ' + counters.map + ' map, ' +
    examined.size + ' resources seen, ' + counters.scanned + ' scans, ' + counters.framed +
    ' scored, ' + counters.discovered + ' tracked, ' + counters.followed + ' follows' +
    (throttled > 0 ? ', THROTTLED ' + throttled + ' scans skipped' : '');
  scansThisWindow = 0;
  throttled = 0;
  for (const leader of leaders.slice(0, 4)) {
    line +=
      '\n      ' + leader.slot.label + ' ' + leader.key + '+0x' +
      leader.candidate.offset.toString(16) + ' ' + leader.candidate.convention + ': ' +
      leader.candidate.passes + ' passes, spread ' + spreadOf(leader.candidate).toFixed(2) + ' m';
  }
  console.log(line);
}, 5000);

hook();

rpc.exports = {
  dispose() {
    Interceptor.detachAll();
    console.log('[vp] detached.');
  },
};
