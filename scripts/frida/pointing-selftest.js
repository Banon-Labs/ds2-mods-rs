// Does the arrow point the right way? Answered on a camera whose answer is already known.
//
// `node scripts/frida/pointing-selftest.js`
//
// The arrow agent's projection maths is pure arithmetic over sixteen floats, so it does not need a
// running game to be wrong in public. This builds view-projections from a camera whose basis is
// chosen here -- position, yaw, pitch, field of view -- works out where a world point MUST land on
// screen straight from that basis, and checks `bearing` agrees. Ground truth never goes through
// the matrix, so a matrix-side mistake cannot cancel itself out.
//
// The case that matters is the last one. A bearing taken in normalised device coordinates instead
// of pixels is sheared by the aspect ratio, which leaves every axis-aligned arrow perfect and
// every diagonal one wrong by up to sixteen degrees -- right enough to survive a log review and
// wrong enough to walk you into a wall. That test fails against the old code on purpose.

'use strict';

const fs = require('fs');
const path = require('path');
const vm = require('vm');

// --- enough of Frida that the agent loads outside a game -------------------------------------
const stub = new Proxy(function () {}, {
  get: () => stub,
  apply: () => stub,
  construct: () => stub,
});

const context = vm.createContext({
  console,
  // No modules, so `prepare` refuses at its first line and `hook` returns without touching COM.
  // The maths under test is reached directly and never goes near the drawing path.
  Process: { findModuleByName: () => null, pointerSize: 8 },
  Memory: stub,
  Interceptor: stub,
  NativeFunction: stub,
  NULL: stub,
  rpc: {},
  setInterval: () => 0,
  Math,
  Number,
  Object,
  Array,
  JSON,
});

// --- the camera whose answers are known ------------------------------------------------------
const WIDTH = 2560;
const HEIGHT = 1440;
const ASPECT = WIDTH / HEIGHT;
const VERTICAL_FOV = (60 * Math.PI) / 180;
const SY = 1 / Math.tan(VERTICAL_FOV / 2);
const SX = SY / ASPECT;
const NEAR = 0.1;
const FAR = 3000;

const dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const cross = (a, b) => [
  a[1] * b[2] - a[2] * b[1],
  a[2] * b[0] - a[0] * b[2],
  a[0] * b[1] - a[1] * b[0],
];
const unit = (v) => {
  const n = Math.hypot(v[0], v[1], v[2]);
  return [v[0] / n, v[1] / n, v[2] / n];
};
const add = (p, v, s) => [p[0] + v[0] * s, p[1] + v[1] * s, p[2] + v[2] * s];

const WORLD_UP = [0, 1, 0];

/// A camera with no roll, which is every third-person camera in every one of these games.
function camera(eye, yaw, pitch) {
  const forward = unit([
    Math.cos(pitch) * Math.sin(yaw),
    Math.sin(pitch),
    Math.cos(pitch) * Math.cos(yaw),
  ]);
  const right = unit(cross(WORLD_UP, forward));
  const up = cross(forward, right);
  return { eye, forward, right, up };
}

/// The row-vector `V * P` the engine uploads: `clip = [x y z 1] * m`.
function viewProjection(cam) {
  const { eye, forward, right, up } = cam;
  const a = FAR / (FAR - NEAR);
  const b = -NEAR * a;
  return [
    SX * right[0], SY * up[0], a * forward[0], forward[0],
    SX * right[1], SY * up[1], a * forward[1], forward[1],
    SX * right[2], SY * up[2], a * forward[2], forward[2],
    -SX * dot(eye, right), -SY * dot(eye, up), -a * dot(eye, forward) + b, -dot(eye, forward),
  ];
}

/// Where a world point lands on screen, worked out from the camera basis and never from `m`.
function truthPixels(cam, p) {
  const rel = [p[0] - cam.eye[0], p[1] - cam.eye[1], p[2] - cam.eye[2]];
  const depth = dot(rel, cam.forward);
  const ndcX = (SX * dot(rel, cam.right)) / depth;
  const ndcY = (SY * dot(rel, cam.up)) / depth;
  return [(ndcX * 0.5 + 0.5) * WIDTH, (0.5 - ndcY * 0.5) * HEIGHT, depth];
}

// --- load the agent and reach into its scope --------------------------------------------------
const agent = fs.readFileSync(path.join(__dirname, 'arrow.js'), 'utf8');
const probe = `
  ({
    score: score,
    toClip: toClip,
    bearing: bearing,
    proof: proof,
    upAxis: () => upAxis,
    setProved: (value) => { proved = value; },
  })
`;
const api = vm.runInContext(agent + '\n' + probe, context, { filename: 'arrow.js' });

const state = { width: WIDTH, height: HEIGHT };
const DEGREES = 180 / Math.PI;
let failures = 0;

function check(name, ok, detail) {
  if (!ok) failures += 1;
  console.log((ok ? '  ok   ' : '  FAIL ') + name + (detail === undefined ? '' : '  ' + detail));
}

/// The angle `bearing` reports, in degrees clockwise from screen right.
function bearingDegrees(m, player, target) {
  const here = api.toClip(m, player);
  const direction = api.bearing(state, m, here, target);
  return direction === null ? null : Math.atan2(direction[1], direction[0]) * DEGREES;
}

function angleGap(a, b) {
  let gap = ((a - b) % 360 + 540) % 360 - 180;
  return Math.abs(gap);
}

// --- 1. the agent's own structural test must accept a real view-projection --------------------
console.log('\nthe structural test accepts a genuine V*P');
for (const [name, cam] of [
  ['level, facing +Z', camera([10, 2, -5], 0, 0)],
  ['yawed 137 deg   ', camera([-120, 31, 64], 2.39, -0.21)],
  ['steep look down ', camera([4, 40, 4], 1.1, -0.9)],
]) {
  const m = viewProjection(cam);
  const player = add(cam.eye, cam.forward, 6);
  const verdict = api.score(m, player, true);
  check(name, verdict !== null && Math.abs(verdict.w - 6) < 1e-4,
    verdict === null ? 'rejected' : 'w = ' + verdict.w.toFixed(4) + ' m');
}

// --- 2. the projection lands where the camera basis says it lands -----------------------------
console.log('\nprojection agrees with ground truth computed outside the matrix');
{
  const cam = camera([-33.5, 12.25, 108.75], 0.77, -0.19);
  const m = viewProjection(cam);
  let worst = 0;
  for (let index = 0; index < 64; index += 1) {
    const point = add(add(add(cam.eye, cam.forward, 5 + index), cam.right, (index % 9) - 4),
      cam.up, ((index * 7) % 11) - 5);
    const here = api.toClip(m, point);
    const gotX = (here[0] / here[3] * 0.5 + 0.5) * WIDTH;
    const gotY = (0.5 - here[1] / here[3] * 0.5) * HEIGHT;
    const [wantX, wantY] = truthPixels(cam, point);
    worst = Math.max(worst, Math.hypot(gotX - wantX, gotY - wantY));
  }
  check('64 points, worst error', worst < 1e-6, worst.toExponential(2) + ' px');
}

// --- 3. a world vertical is vertical on screen, from every angle ------------------------------
console.log('\na world-space vertical points straight up the screen (the green arrow)');
{
  let worst = 0;
  let worstAt = '';
  for (let yawStep = 0; yawStep < 12; yawStep += 1) {
    for (let pitchStep = -3; pitchStep <= 3; pitchStep += 1) {
      const yaw = (yawStep / 12) * Math.PI * 2;
      const pitch = (pitchStep / 3) * 0.7;
      const cam = camera([21, 9, -4], yaw, pitch);
      const m = viewProjection(cam);
      const player = add(cam.eye, cam.forward, 7);
      const got = bearingDegrees(m, player, add(player, WORLD_UP, 8));
      const gap = got === null ? 999 : angleGap(got, -90);
      if (gap > worst) {
        worst = gap;
        worstAt = 'yaw ' + (yaw * DEGREES).toFixed(0) + ' pitch ' + (pitch * DEGREES).toFixed(0);
      }
    }
  }
  check('84 camera angles, worst tilt off vertical', worst < 1e-6,
    worst.toExponential(2) + ' deg at ' + worstAt);
}

// --- 4. camera-right is horizontal on screen --------------------------------------------------
console.log('\na step along camera-right points straight across the screen');
{
  const cam = camera([0, 5, 0], 1.9, -0.35);
  const m = viewProjection(cam);
  const player = add(cam.eye, cam.forward, 9);
  const right = bearingDegrees(m, player, add(player, cam.right, 6));
  const left = bearingDegrees(m, player, add(player, cam.right, -6));
  check('+right is screen right', angleGap(right, 0) < 1e-6, right.toFixed(6) + ' deg');
  check('-right is screen left ', angleGap(left, 180) < 1e-6, left.toFixed(6) + ' deg');
}

// --- 5. every bearing matches the pixel angle ground truth gives ------------------------------
console.log('\nevery bearing matches the angle between the two ground-truth pixel positions');
{
  let worst = 0;
  let worstDetail = '';
  for (let index = 0; index < 200; index += 1) {
    const cam = camera([index * 0.7 - 70, 6 + (index % 13), 40 - index * 0.3],
      (index / 200) * Math.PI * 2, Math.sin(index) * 0.6);
    const m = viewProjection(cam);
    const player = add(add(cam.eye, cam.forward, 6 + (index % 5)), cam.right, (index % 7) - 3);
    const target = add(add(add(player, cam.forward, ((index * 3) % 41) - 8),
      cam.right, ((index * 5) % 37) - 18), WORLD_UP, ((index * 11) % 23) - 11);
    const [px, py, depth] = truthPixels(cam, player);
    const [tx, ty, targetDepth] = truthPixels(cam, target);
    if (depth < 0.5 || targetDepth < 0.5) continue;       // ground truth is undefined behind the lens
    const want = Math.atan2(ty - py, tx - px) * DEGREES;
    const got = bearingDegrees(m, player, target);
    const gap = got === null ? 999 : angleGap(got, want);
    if (gap > worst) {
      worst = gap;
      worstDetail = 'wanted ' + want.toFixed(3) + ', got ' + got.toFixed(3);
    }
  }
  check('worst disagreement', worst < 1e-6, worst.toExponential(2) + ' deg  ' + worstDetail);
}

// --- 6. a target behind the camera points away from its reflection ----------------------------
console.log('\na target behind the camera points away from it, not at its mirror image');
{
  const cam = camera([0, 3, 0], 0, 0);
  const m = viewProjection(cam);
  const player = add(cam.eye, cam.forward, 5);
  // Well behind the lens and off to the camera's left: the correct arrow leans LEFT, because that
  // is the shorter way to turn round to face it. Projected naively it would lean right.
  const behind = add(add(cam.eye, cam.forward, -20), cam.right, -6);
  const got = bearingDegrees(m, player, behind);
  check('leans toward the near side', got !== null && Math.cos(got / DEGREES) < 0,
    got === null ? 'no bearing' : got.toFixed(2) + ' deg');
}

// --- 7. the test can actually see the aspect-ratio bug it exists to catch ---------------------
console.log('\nthe old normalised-device-coordinate bearing is measurably wrong');
{
  const cam = camera([12, 7, -30], 0.4, -0.15);
  const m = viewProjection(cam);
  const player = add(cam.eye, cam.forward, 8);
  const target = add(add(player, cam.right, 10), WORLD_UP, 10);
  const here = api.toClip(m, player);
  const there = api.toClip(m, target);
  const dx = there[0] * here[3] - here[0] * there[3];
  const dy = there[1] * here[3] - here[1] * there[3];
  const old = Math.atan2(-dy, dx) * DEGREES;                    // no width/height scaling: the bug
  const [px, py] = truthPixels(cam, player);
  const [tx, ty] = truthPixels(cam, target);
  const want = Math.atan2(ty - py, tx - px) * DEGREES;
  const now = bearingDegrees(m, player, target);
  check('fixed bearing is right', angleGap(now, want) < 1e-6, now.toFixed(3) + ' deg');
  check('old bearing was wrong ', angleGap(old, want) > 5,
    old.toFixed(3) + ' deg, off by ' + angleGap(old, want).toFixed(2) + ' deg');
}

// --- 8. the proof block the agent prints in game, on a matrix known to be good ----------------
console.log('\nthe in-game proof block, run here first');
{
  const cam = camera([-8, 14, 22], 2.1, -0.28);
  const m = viewProjection(cam);
  const player = add(cam.eye, cam.forward, 6.5);
  api.setProved(false);
  api.proof(state, m, player, api.toClip(m, player));
  const up = api.upAxis();
  check('world up recovered', up[0] === 0 && up[1] === 1 && up[2] === 0, JSON.stringify(up));
}

console.log('\n' + (failures === 0
  ? 'POINTING SELFTEST: all checks passed.'
  : 'POINTING SELFTEST: ' + failures + ' FAILED.'));
process.exit(failures === 0 ? 0 : 1);
