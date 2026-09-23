// Where does DARK SOULS II actually keep the view-projection it renders with?
//
// WHY THIS EXISTS. `ds2-invasion-path` drew an arrow in the wrong direction for a whole session.
// Ten build/stage/launch cycles went into the arrow's arithmetic. The cause was the matrix it was
// being handed, which nobody had printed:
//
//   [0.0000 3.4405 0.0000 0.0000 | 0.0000 0.0000 2.4751 2.4751
//    | 0.0000 0.0000 -0.1000 0.0000 | 1.4 0.0 -0.1 0.0]
//
// Its first column is zero, so `clip.x` was the constant 1.4 for every point in the world, and no
// arrow arithmetic could ever have been right. The crate accepted it because both of its oracles
// -- "the player projects near the middle of the frame" and "the head rises a plausible number of
// pixels above the feet" -- are blind to the horizontal axis by construction.
//
// TWO TESTS, AND NEITHER IS A HEURISTIC.
//
// 1. `length(m[3], m[7], m[11]) == 1`. The engine's projection (`0x140001a90`) is row-vector
//    left-handed with `m23 == 1` and `m33 == 0`, so in the product `V * P` the whole of column 3
//    comes from `V`'s column 2 -- the camera's forward axis in world space, which is a unit vector
//    because it is a rotation's axis. The impostor above scores 2.4751.
//
// 2. The LOCAL PLAYER, read independently from `GameManagerImp`, must project near the middle of
//    the frame -- and near it on BOTH axes, which is the part the crate's version could not do,
//    because it measured a distance from the centre and a mirror about the centre does not change
//    a distance. Here the signed offsets are printed, so a mirror is visible as a number.
//
// A FIRST ATTEMPT HOOKED D3D11 AND MEASURED NOTHING. `Map` and `UpdateSubresource` were hooked
// through a vtable taken from a throwaway `D3D11CreateDevice`, and the counters read `Map 0,
// UpdateSubresource 0` -- the game's context is a different concrete class, so the hook watched an
// object nobody called. That is recorded rather than deleted: zero hits and a hook that never
// fired look identical without a counter, and the counter is what said which it was.
//
// This version does not touch D3D at all. The matrix exists in the game's own memory before it is
// ever uploaded, and the repo already knows the way to it.
//
// HOW TO RUN IT
//
//   python3 /home/banon/projects/ds2-mods-rs/scripts/ds2-frida-up.py
//   nohup setsid uv run --with frida python3 \
//       /home/banon/projects/ds2-mods-rs/scripts/ds2-frida-watch.py \
//       --agent /home/banon/projects/ds2-mods-rs/scripts/frida/camera-matrix.js &
//
// `nohup setsid` matters: the harness caps a foreground command at ninety seconds and SIGTERMs it,
// which is the hard kill `AGENTS.md` says never to do to a watcher. Edit this file to change what
// it reports; it reloads in place, with no rebuild and no relaunch.

'use strict';

// ---------------------------------------------------------------------------------------------
// The address chain, from `ds2-rva`. Every one of these is a fact the repo already established.
// ---------------------------------------------------------------------------------------------

const GAME_MANAGER_IMP_RVA = 0x016148f0;
const GAME_MANAGER_CAMERA_MANAGER_OFFSET = 0x20;
const GAME_MANAGER_PLAYER_CTRL_OFFSET = 0xd0;
const CHARACTER_CTRL_POSITION_OFFSET = 0x90;
const CAMERA_MANAGER_OPERATOR_OFFSETS = [0x18, 0x20, 0x28];

/// How far a forward column's length may sit from one and still count.
const UNIT_TOLERANCE = 0.02;

/// How far into a `CameraOperator` to sweep for matrices. The known view and projection offsets
/// are `0x10`/`0x20` and `0x50`; this is wide enough to find anything near them and still cheap.
const OPERATOR_SWEEP_BYTES = 0x2000;

/// Reported once per place, because this runs on a timer and a place does not change address.
const seen = new Set();

// Ruling out the near end is the first job of any probe: an instrument that cannot tell "found
// nothing" from "never ran" is worse than no instrument.
const counters = { passes: 0, operators: 0, windows: 0, unit: 0, framed: 0 };

const gameModule = Process.findModuleByName('DarkSoulsII.exe');
const base = gameModule === null ? null : gameModule.base;

function readPointer(address) {
  try {
    const value = address.readPointer();
    return value.isNull() ? null : value;
  } catch (error) {
    return null;
  }
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

function forwardColumnLength(m) {
  return Math.sqrt(m[3] * m[3] + m[7] * m[7] + m[11] * m[11]);
}

/** Row-vector: `clip = [x y z 1] * M`. The same arithmetic `geometry::to_clip` does. */
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

/** The local player's world position, read independently of any camera. */
function playerPosition() {
  if (base === null) return null;
  const manager = readPointer(base.add(GAME_MANAGER_IMP_RVA));
  if (manager === null) return null;
  const player = readPointer(manager.add(GAME_MANAGER_PLAYER_CTRL_OFFSET));
  if (player === null) return null;
  try {
    const position = readFloats(player.add(CHARACTER_CTRL_POSITION_OFFSET), 3);
    return finite(position) ? position : null;
  } catch (error) {
    return null;
  }
}

function cameraOperators() {
  if (base === null) return [];
  const manager = readPointer(base.add(GAME_MANAGER_IMP_RVA));
  if (manager === null) return [];
  const cameras = readPointer(manager.add(GAME_MANAGER_CAMERA_MANAGER_OFFSET));
  if (cameras === null) return [];
  const out = [];
  for (const offset of CAMERA_MANAGER_OPERATOR_OFFSETS) {
    const operator = readPointer(cameras.add(offset));
    if (operator !== null) out.push({ offset, operator });
  }
  return out;
}

/**
 * Sweep one object for a matrix that is a view-projection AND frames the player.
 *
 * Sixteen-byte steps: a matrix in a game object is `__m128`-aligned, and stepping by four would
 * quadruple the work to find the same things shifted.
 */
function sweep(label, object, bytes, player) {
  // FOUR-BYTE STRIDE, NOT SIXTEEN. The sixteen-byte assumption -- "a matrix in a game object is
  // __m128-aligned" -- is false here and it hid every real matrix. At stride 16 the projection at
  // `operator@0x20+0x310` read as a shifted window whose rows were `[0 1.5 -5.0 1.0]`; one float
  // later it is `[1.4258 0 0 0 | 0 2.5386 0 0 | 0 0 1 1]`, the engine's own signature with
  // `m23 == 1` and an aspect of 1.78. Four times the work on a 1 KB window is nothing.
  for (let offset = 0; offset + 64 <= bytes; offset += 4) {
    let m;
    try {
      m = readFloats(object.add(offset), 16);
    } catch (error) {
      return; // Past the end of the mapping.
    }
    counters.windows += 1;
    if (!finite(m)) continue;

    // THE VIEW MATRIX IS CHECKED FIRST, BECAUSE THE UNIT-COLUMN GATE BELOW WOULD THROW IT AWAY.
    // A world-to-camera matrix has column 3 = (0, 0, 0, 1), so its forward-column length is ZERO,
    // not one -- that test is for the PRODUCT. Running it first is why the earlier passes found
    // projections and no views, and reported "no dense product in the operators" without being
    // able to say whether a view was there at all.
    //
    // The signature: the upper-left 3x3 is a rotation, so each of its rows is a unit vector; the
    // last column is (0,0,0,1); and the last row is the translation, which for a camera in a
    // loaded map is never the origin.
    const rowLength = (a, b, c) => Math.sqrt(m[a] * m[a] + m[b] * m[b] + m[c] * m[c]);
    const unitRow = (a, b, c) => Math.abs(rowLength(a, b, c) - 1.0) <= UNIT_TOLERANCE;
    const isView =
      Math.abs(m[3]) < 1e-6 && Math.abs(m[7]) < 1e-6 && Math.abs(m[11]) < 1e-6 &&
      Math.abs(m[15] - 1.0) < 1e-4 &&
      unitRow(0, 1, 2) && unitRow(4, 5, 6) && unitRow(8, 9, 10) &&
      rowLength(12, 13, 14) > 0.5;
    if (isView) {
      const key = label + '+0x' + offset.toString(16) + ' VIEW';
      if (!seen.has(key)) {
        seen.add(key);
        // The eye is `-t * R^T`, not the translation row -- reading that row as a position is a
        // plausible-looking answer wrong by the camera's own rotation. Printed because "is this
        // near the player" is the only cheap check on whether it is the RIGHT view.
        const t = [m[12], m[13], m[14]];
        const eye = [
          -(t[0] * m[0] + t[1] * m[1] + t[2] * m[2]),
          -(t[0] * m[4] + t[1] * m[5] + t[2] * m[6]),
          -(t[0] * m[8] + t[1] * m[9] + t[2] * m[10]),
        ];
        const away = Math.hypot(eye[0] - player[0], eye[1] - player[1], eye[2] - player[2]);
        console.log(
          '[camera-matrix] ' + key + ' at ' + object.add(offset) +
          '\n      eye ' + eye.map((v) => v.toFixed(1)).join(',') +
          ', ' + away.toFixed(1) + ' m from the player' + format(m)
        );
      }
      continue;
    }

    const forward = forwardColumnLength(m);
    if (Math.abs(forward - 1.0) > UNIT_TOLERANCE) continue;
    counters.unit += 1;

    // TWO SHAPES ARE WORTH REPORTING, AND SPARSE PERMUTATION MATRICES ARE NEITHER.
    //
    // At a four-byte stride the object is full of windows like `[0 0 0 1 | 0 0 0 0 | 1 0 0 0 |
    // 0 1 0 0]` -- a lone `1.0` in the forward column scores exactly 1.00000, and shuffling the
    // axes moves the world, so both earlier tests pass. They are quaternions, bases and padding.
    //
    // `projection` is the engine's own signature from `0x140001a90`: diagonal x and y scales,
    // `m23 == 1`, `m33 == 0`, everything else zero. `product` is `V * P`, which is DENSE -- a
    // rotation multiplied through leaves no structural zeros in the upper three rows. Anything
    // that is neither is not a camera.
    const nearlyZero = (value) => Math.abs(value) < 1e-6;
    const isProjection =
      m[0] > 0.1 && m[5] > 0.1 &&
      nearlyZero(m[1]) && nearlyZero(m[2]) && nearlyZero(m[3]) &&
      nearlyZero(m[4]) && nearlyZero(m[6]) && nearlyZero(m[7]) &&
      nearlyZero(m[8]) && nearlyZero(m[9]) &&
      Math.abs(m[11] - 1.0) < 1e-4 && nearlyZero(m[15]);
    const dense = m.slice(0, 12).filter((value) => !nearlyZero(value)).length;
    const isProduct = dense >= 9;
    if (!isProjection && !isProduct) continue;

    // THE SECOND TEST, WITH SIGNS. A third-person camera puts the player near the middle of the
    // frame. Printing the signed normalised offsets rather than a distance is the whole point: a
    // mirrored matrix has the right distance and the wrong sign, which is invisible to the test
    // the crate was using.
    // A pure projection is reported on its shape alone: the player's clip w through a projection
    // is the camera-space depth only if the view has already been applied, which it has not.
    if (isProjection) {
      const key = label + '+0x' + offset.toString(16) + ' PROJECTION';
      if (!seen.has(key)) {
        seen.add(key);
        console.log(
          '[camera-matrix] ' + key + ' at ' + object.add(offset) +
          '\n      x scale ' + m[0].toFixed(4) + ', y scale ' + m[5].toFixed(4) +
          ', aspect ' + (m[5] / m[0]).toFixed(4) + format(m)
        );
      }
      continue;
    }
    const clip = toClip(m, player);
    if (!(clip[3] > 0.05)) continue;
    const ndcX = clip[0] / clip[3];
    const ndcY = clip[1] / clip[3];
    if (!Number.isFinite(ndcX) || !Number.isFinite(ndcY)) continue;
    if (Math.abs(ndcX) > 0.6 || Math.abs(ndcY) > 0.9) continue;

    // NON-DEGENERACY, AND IT IS WHAT THE UNIT TEST ALONE COULD NOT DO. The first sweep reported
    // thirteen places, every one a sparse matrix like
    //
    //     [0 0 1 0 | 0 0 0 1 | 0 0 0 0 | 0 0 0 0]
    //
    // whose forward column is a lone `1.0`, so it scores exactly 1.00000, and whose zeros happen
    // to put the player at ndc 0,0. Both tests pass and it is not a camera -- it is a quaternion,
    // or a basis, or padding.
    //
    // A camera moves the world. Step one metre along each world axis from the player and require
    // the screen position to change by a sane amount on at least two of the three: too little is a
    // matrix that collapses the world to a point, too much is one that is not in metres. This is
    // the 3D form of the scale test `crate::capture` does vertically, and unlike that one it
    // cannot be passed by a matrix that ignores an axis.
    const AXES = [[1, 0, 0], [0, 1, 0], [0, 0, 1]];
    let moved = 0;
    for (const axis of AXES) {
      const near = toClip(m, [player[0] + axis[0], player[1] + axis[1], player[2] + axis[2]]);
      if (!(near[3] > 0.05)) continue;
      const shift = Math.hypot(near[0] / near[3] - ndcX, near[1] / near[3] - ndcY);
      if (Number.isFinite(shift) && shift > 0.005 && shift < 2.0) moved += 1;
    }
    if (moved < 2) continue;
    counters.framed += 1;

    const key = label + '+0x' + offset.toString(16);
    if (seen.has(key)) continue;
    seen.add(key);
    console.log(
      '[camera-matrix] ' + key + ' at ' + object.add(offset) +
      '\n      forward column length ' + forward.toFixed(5) +
      ', player ndc ' + ndcX.toFixed(3) + ',' + ndcY.toFixed(3) +
      ' (w=' + clip[3].toFixed(2) + ')' +
      format(m)
    );
  }
}

function pass() {
  counters.passes += 1;
  const player = playerPosition();
  if (player === null) {
    if (counters.passes % 5 === 1) {
      console.log('[camera-matrix] no local player yet -- the game is not in a world.');
    }
    return;
  }
  const operators = cameraOperators();
  counters.operators = operators.length;
  for (const { offset, operator } of operators) {
    sweep('operator@0x' + offset.toString(16), operator, OPERATOR_SWEEP_BYTES, player);
  }
  if (counters.passes % 5 === 0) {
    console.log(
      '[camera-matrix] pass ' + counters.passes + ': player ' +
      player.map((value) => value.toFixed(1)).join(',') +
      ' | ' + counters.operators + ' operator(s), ' + counters.windows + ' window(s), ' +
      counters.unit + ' unit-column, ' + counters.framed + ' also framing, ' +
      seen.size + ' place(s) reported'
    );
  }
}

if (base === null) {
  console.log('[camera-matrix] DarkSoulsII.exe module not found; nothing is being read.');
} else {
  console.log('[camera-matrix] image base ' + base + '; sweeping camera operators every 2s.');
  // REPORT THE FAILURE, DO NOT SWALLOW IT. An exception inside a `setInterval` callback stops the
  // timer and prints nothing, which reads exactly like "the sweep found nothing" -- and did, for
  // two minutes, while the watcher sat alive and attached with a dead loop. The rule this breaks
  // is the one in AGENTS.md: an instrument that reports an absence it cannot detect is worse than
  // no instrument.
  const guarded = function () {
    try {
      pass();
    } catch (error) {
      console.log('[camera-matrix] pass threw: ' + error + '\n' + (error.stack || '(no stack)'));
    }
  };
  setInterval(guarded, 2000);
  guarded();
}

// Nothing here installs a watchpoint or a trampoline, so there is nothing in a debug register to
// service after unload -- but the export exists so a reload is explicit about leaving clean.
rpc.exports = {
  dispose() {
    console.log('[camera-matrix] done; ' + seen.size + ' place(s) reported.');
  },
};
