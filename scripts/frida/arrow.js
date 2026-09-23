// An arrow, drawn on the game's own back buffer, from the camera the renderer actually uses.
//
// This is the prototype the Rust crate is not allowed to be written from until it works: the whole
// mechanism -- find the view-projection, project a world point, put pixels on screen where that
// point is -- proven in a file that reloads in a second, before any of it is compiled.
//
// # What it draws, and why that is the test
//
// A breadcrumb is dropped at the character's position the moment the camera is confirmed. The
// arrow then points from the character to that breadcrumb, forever. WALK AWAY FROM IT AND SPIN THE
// CAMERA: if the matrix is right the arrow keeps pointing back at the spot you left, from any
// angle, at any distance. If the matrix is mirrored the arrow points at its reflection and the
// error is obvious the first time you turn around -- which is precisely the failure that survived
// a whole session of "the log looks fine".
//
// # Drawing without a renderer
//
// The crate builds a vertex buffer and two shaders. None of that is needed to prove the geometry.
// `ID3D11DeviceContext1::ClearView` fills a list of rectangles in one call with no shader, no
// input layout and no state to save and restore, so a thick line is a run of small squares and an
// arrow is three runs. Ugly on purpose: this is an instrument, not the feature.
//
// # Nothing here is called on a slot taken from memory
//
// COM methods are vtable indices and a wrong index is a crash in someone's game, so every table
// this calls into is checked against a value only the right table can return, and the agent
// refuses rather than guesses:
//
//   * `ID3D11DeviceContext` slot 114 must be `FinishCommandList`, which returns
//     `DXGI_ERROR_INVALID_CALL` (0x887a0001) on an immediate context and nothing else does. That
//     pins the end of the table, and so pins `ClearView` at 132 in the `ID3D11DeviceContext1`
//     block that follows it.
//   * `ID3D11Device` slot 37 must be `GetFeatureLevel`, which returns a `D3D_FEATURE_LEVEL`
//     (0xb000 for 11_0). A wrong slot returns a pointer or a count, not one of six known constants.
//
// Both run on a throwaway device built here, never on the game's, so a failed check costs nothing.

'use strict';

const GAME_MANAGER_IMP_RVA = 0x016148f0;
const GAME_MANAGER_PLAYER_CTRL_OFFSET = 0xd0;
const CHARACTER_CTRL_POSITION_OFFSET = 0x90;

// --- ID3D11DeviceContext / ...Context1 -------------------------------------------------------
const SLOT_MAP = 14;
const SLOT_UPDATE_SUBRESOURCE = 48;
const SLOT_FINISH_COMMAND_LIST = 114;
const SLOT_CLEAR_VIEW = 132;

// --- ID3D11Device ----------------------------------------------------------------------------
const SLOT_CREATE_RENDER_TARGET_VIEW = 9;
const SLOT_GET_FEATURE_LEVEL = 37;

// --- IDXGISwapChain --------------------------------------------------------------------------
const SLOT_PRESENT = 8;
const SLOT_GET_BUFFER = 9;
const SLOT_SWAPCHAIN_GET_DEVICE = 7;
const SLOT_GET_DESC = 12;

const DXGI_ERROR_INVALID_CALL = 0x887a0001;
const KNOWN_FEATURE_LEVELS = [0x9100, 0x9200, 0x9300, 0xa000, 0xa100, 0xb000, 0xb100, 0xc000, 0xc100];

const SCAN_BYTES = 512;
const UNIT_TOLERANCE = 0.02;
const ORTHOGONAL_TOLERANCE = 0.02;
const PARALLEL_TOLERANCE = 0.02;
/// `|c1| / |c0|` is the aspect ratio. This game's projection measured 1.7806; the band covers
/// 4:3 through 21:9 so the test is about being a screen, not about being this screen.
const MIN_ASPECT = 1.2;
const MAX_ASPECT = 2.6;
const LOOKS_PER_RESOURCE = 20;
const SCANS_PER_WINDOW = 400;
/// `clip.w` is the camera-to-player distance, which is the one physical test here. Wide, because
/// the job of this bound is only to throw out the 21 m and 45 m parameter blobs -- narrowing it to
/// a third-person camera's resting distance also threw out every frame where the camera was
/// pulled out by a lock-on or a cutscene, and killing the real matrix on one such frame is how
/// eighteen candidates were found and then deleted.
const CAMERA_MIN_METERS = 0.3;
const CAMERA_MAX_METERS = 30.0;

/// How far off centre the character may project. NOT a third of the frame: the character is not
/// centred in DARK SOULS II and the offset grows while moving, so a tight bound here rejects the
/// real camera on ordinary frames. On screen at all is the honest requirement; persistence does
/// the discriminating.
///
/// 1.5, AND THE MEASUREMENT THAT SET IT. At 0.9 the heartbeat reported, window after window,
/// `furthest off screen: ndc -0.000,-0.976` -- a matrix that had passed every shape test AND the
/// camera-distance test, putting the character DEAD CENTRE horizontally and a hair below the
/// bottom edge. That is not a coincidence a random buffer produces: `x = -0.000` is the camera
/// looking straight at the character, and `y = -0.976` is the character's ORIGIN, which is
/// between their feet, sitting just under the frame while their body fills it. The bound was
/// rejecting the answer for being correct about where feet are.
///
/// It still has work to do -- an `ndc` of 12 or 300 is a matrix that is not looking at this
/// character at all -- but "on screen" was never the right shape for this test, and persistence
/// does the discriminating anyway.
const NDC_LIMIT = 1.5;
/// Sampled looks, not frames: at `GATE_WHILE_FOLLOWING` the camera's own buffer comes round about
/// seven times a second, so this is roughly eight seconds of unbroken agreement.
const PASSES_TO_BELIEVE = 60;
/// Metres the character must have travelled across those passes.
///
/// ZERO, DELIBERATELY. This was 1.5 m, on the reasoning that a constant matrix would otherwise
/// "agree" forever -- but the character has been standing still the whole time, so the gate could
/// never fire, and four perfect candidates sat at the top of the leaderboard at 1442 passes and
/// `spread 0.00 m` while the probe hunted for something it had already found. The orthogonality
/// and parallel-column tests in `score` do that discriminating now, and they are properties of the
/// matrix rather than of the player, so nobody has to go for a walk to confirm a camera. Spread is
/// still measured and still printed, because "confirmed while stationary" is a caveat worth
/// seeing.
const SPREAD_TO_BELIEVE = 0.0;

/// Arrow geometry, in pixels. Fixed length: the arrow says WHICH WAY, and a bearing indicator that
/// shrinks with distance is unreadable exactly when it matters most.
const ARROW_LENGTH_PX = 220;
const ARROW_BARB_PX = 60;
const ARROW_BARB_RADIANS = Math.PI / 6;
const STROKE_PX = 7;
const ARROW_COLOUR = [1.0, 0.45, 0.05, 1.0];
const BASE_COLOUR = [0.15, 0.9, 1.0, 1.0];

/// A second arrow at a world-space vertical eight metres above the character's feet.
///
/// It exists to be WRONG LOUDLY. The bearing arrow needs you to walk somewhere and remember where
/// you were; this one needs nothing, is drawable on the first frame, and has exactly one correct
/// answer from every camera angle in the game: UP THE SCREEN. A projection that is mirrored,
/// transposed, scaled by the aspect ratio or fed the wrong columns tips this arrow off vertical
/// immediately and visibly, which is the failure that a log full of plausible numbers hides.
const PLUMB_METRES = 8.0;
const PLUMB_LENGTH_PX = 120;
const PLUMB_COLOUR = [0.25, 1.0, 0.35, 1.0];

/// The game image, and WHAT IT WAS FOUND BY, because one silent null here cost a whole session.
///
/// `playerPosition` opens with `if (base === null) return null`, so a module lookup that misses
/// makes every character read fail and the agent reports `CHARACTER UNREADABLE 33878 times` --
/// a message that reads as "the game is at a menu" and is not. Measured against it on
/// 2026-09-22: `scripts/ds2-player-chain.py` walked the SAME three pointers out of
/// `/proc/<pid>/mem` at the same moment and came back with a character standing at
/// `21.90, 3.90, -12.54`. The world was there; the agent just could not find its own image.
///
/// So the lookup falls back to enumeration and says which rule answered. The main module is the
/// first one Frida lists, and under Wine its name is not guaranteed to be the spelling used
/// here.
function findGameImage() {
  const named = Process.findModuleByName('DarkSoulsII.exe');
  if (named !== null) return { module: named, how: 'findModuleByName' };
  const all = Process.enumerateModules();
  for (const candidate of all) {
    if (candidate.name.toLowerCase() === 'darksoulsii.exe') {
      return { module: candidate, how: 'enumerateModules by name' };
    }
  }
  // The main image is the first module of a Windows process, and there is exactly one game here.
  if (all.length > 0 && all[0].name.toLowerCase().endsWith('.exe')) {
    return { module: all[0], how: 'first module (' + all[0].name + ')' };
  }
  return { module: null, how: 'NOT FOUND among ' + all.length + ' modules' };
}

const found = findGameImage();
const gameModule = found.module;
const base = gameModule === null ? null : gameModule.base;
console.log('[arrow] game image: ' + (base === null ? 'NOT FOUND -- every character read will ' +
  'fail and the log will say the world is empty when it is not' : base + ' via ' + found.how));

const counters = { update: 0, map: 0, scans: 0, scored: 0, tracked: 0, follows: 0, present: 0, drawn: 0 };
const tracked = new Map();
/// The same slots as `tracked`, as an array to be walked with a native pointer compare.
///
/// THE HOT PATH MAY NOT ALLOCATE. Keying `tracked` by `resource.toString()` builds a JavaScript
/// string inside a hook that runs four million times a second, and that alone starves Frida's JS
/// thread badly enough that `setInterval` stops firing -- which reads as a hung game and is not.
/// This array is almost always empty or tiny, so `.equals()` over it costs a native comparison and
/// no allocation; the string key is only built once the cheap integer budget check has passed.
const trackedList = [];
const examined = new Map();

/// One call in `gate` is looked at. EVERYTHING ELSE RETURNS BEFORE TOUCHING `args`.
///
/// Reading `args[1]` materialises a `NativePointer` object, and these hooks run about fifty
/// thousand times a second between them, so two argument reads per call is a hundred thousand
/// allocations a second -- enough on its own to starve Frida's JS thread until `setInterval` stops
/// firing. Measured twice: once blamed on the scan, once on `toString`, and the agent was silent
/// both times with the game running perfectly. An integer compare on a counter costs nothing and
/// has to come first.
///
/// Coarse while hunting, because a miss only delays discovery; fine once something is being
/// followed, because a miss there is a frame of evidence lost.
///
/// 4096 WAS TOO COARSE ONCE `examined` STARTED CLEARING. At a quarter of a million uploads per
/// window that gate admits sixty calls, so sixty looks had to cover every buffer in the game and
/// the heartbeat sat at `20 resources seen, 66 scans` while `SCANS_PER_WINDOW` -- the budget that
/// is supposed to do the throttling -- was never within three hundred of being spent. The old
/// value was tuned when `examined` was permanent and the first few windows were all that
/// mattered; with the budget doing its job the gate only has to keep the rejected path cheap.
const GATE_WHILE_HUNTING = 64;
const GATE_WHILE_FOLLOWING = 8;
let gate = GATE_WHILE_HUNTING;
let scansThisWindow = 0;
let throttled = 0;
/// Sampled calls where `GameManagerImp` had no readable character. Reported, because "no scans"
/// and "no character to scan against" look identical from outside and mean opposite things.
let unreadable = 0;
let believed = null;
let breadcrumb = null;
/// Set once the arithmetic proof below has run against a real swap chain, so it prints one time.
let proved = false;
/// World up, NAMED FROM THE MATRIX rather than assumed to be +Y. Filled by `proof`.
let upAxis = [0, 1, 0];

let clearView = null;
let createRenderTargetView = null;
let getBuffer = null;
let swapChainGetDevice = null;
let getDesc = null;
let drawState = null;
let refusal = null;

/// Two sixteen-float buffers reused for every window examined.
///
/// The scan reads 112 windows per buffer in both conventions; allocating a fresh array for each
/// read and each transpose is a hundred and eighty thousand short-lived arrays per five-second
/// window, and this agent has already been silenced three times by exactly this kind of pressure
/// on Frida's JS thread. Nothing downstream keeps a reference to these -- `score` reads and
/// discards -- with one exception, `believed.matrix`, which copies.
const RAW = new Array(16);
const TRANSPOSED = new Array(16);

function readFloats(pointer, count) {
  const out = [];
  for (let index = 0; index < count; index += 1) out.push(pointer.add(index * 4).readFloat());
  return out;
}

/** Fills `RAW` from memory. Returns false if the read faulted. */
function readMatrix(pointer) {
  try {
    for (let index = 0; index < 16; index += 1) RAW[index] = pointer.add(index * 4).readFloat();
  } catch (error) {
    return false;
  }
  return true;
}

/// Every element real and not absurd. LENGTH FROM THE ARRAY, not from the matrix that was the
/// first caller.
///
/// This read `index < 16` and `playerPosition` passes it THREE floats, so `values[3]` was
/// `undefined`, `Number.isFinite(undefined)` was false, and the character came back null on every
/// single frame. The agent then reported `CHARACTER UNREADABLE 33878 times`, which reads as an
/// empty world and is not one: `scripts/ds2-player-chain.py` walked the same three pointers out
/// of `/proc/<pid>/mem` at the same moment and found a character standing at 21.90, 3.90,
/// -12.54. Nothing downstream could work -- no candidate could ever be grounded, and `draw`
/// returned at its first line for the whole session.
function finite(values) {
  for (let index = 0; index < values.length; index += 1) {
    const value = values[index];
    if (!Number.isFinite(value) || Math.abs(value) >= 1e12) return false;
  }
  return true;
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

function transpose(m) {
  const out = [];
  for (let row = 0; row < 4; row += 1) {
    for (let column = 0; column < 4; column += 1) out.push(m[column * 4 + row]);
  }
  return out;
}

/** Transposes `RAW` into `TRANSPOSED` without allocating. */
function transposeRaw() {
  for (let row = 0; row < 4; row += 1) {
    for (let column = 0; column < 4; column += 1) {
      TRANSPOSED[row * 4 + column] = RAW[column * 4 + row];
    }
  }
}

/// Why the last `score` said no. A rejection that does not say which test failed is how a probe
/// reports "found nothing" for an hour while the answer sits one over-tight bound away.
let lastReason = '';

/// WHICH TEST SAID NO, COUNTED ON EVERY CALL, INCLUDING THE NINETY THOUSAND THAT CANNOT AFFORD A
/// STRING.
///
/// `lastReason` is only built when `explain` is true, which is the follow path -- so during a
/// discovery window, where candidates actually die, the log says `0 scored` and not one word
/// about why. That is the state this file has been in twice now, each time for long enough to
/// start guessing at bounds. An integer increment costs nothing and turns "nothing passed" into
/// "forty thousand failed the aspect test and none failed anything else", which is an answer.
const FAIL_NAMES = [
  'not finite', 'forward column not 1', 'a world axis row collapsed', 'a column collapsed',
  'right/up not perpendicular', 'right/forward not perpendicular', 'up/forward not perpendicular',
  'column 2 not parallel to forward', 'aspect is not a screen', 'camera distance out of range',
  'ndc not finite', 'character off screen', 'a metre lands behind the camera',
  'a metre moves the screen too far', 'a metre moves the screen not at all',
];
const failCounts = new Array(FAIL_NAMES.length).fill(0);

/// The values the last two tests actually saw, so a bound is widened on evidence or not at all.
///
/// `camera distance out of range 14` says a matrix that passed every shape test was thrown out,
/// and nothing about whether it missed by a centimetre or by a kilometre. Those two want
/// opposite responses: one is a bound set a shade too tight, the other is a matrix that is not a
/// camera. Four numbers, written on the rejection path, answer it without a string.
let distanceMin = Infinity;
let distanceMax = -Infinity;
let offScreenX = 0;
let offScreenY = 0;

/// `explain` builds the rejection message; DISCOVERY MUST PASS FALSE.
///
/// `score` runs about ninety thousand times per five-second window during a scan, and every
/// rejection message here is a string concatenation with a `toFixed` in it. Building them
/// unconditionally wedged Frida's JS thread hard enough that the log stopped growing entirely
/// while the game and the watcher both stayed alive -- the fourth and least obvious instance of
/// the same failure in this file. The messages are only wanted on the follow path, which runs a
/// few times a second and is where a candidate dying actually tells us something.
function score(m, player, explain) {
  if (!finite(m)) { failCounts[0] += 1; if (explain) lastReason = 'not finite'; return null; }
  const forward = Math.sqrt(m[3] * m[3] + m[7] * m[7] + m[11] * m[11]);
  if (Math.abs(forward - 1.0) > UNIT_TOLERANCE) {
    failCounts[1] += 1;
    if (explain) lastReason = 'forward column ' + forward.toFixed(4) + ', not 1';
    return null;
  }
  for (let row = 0; row < 3; row += 1) {
    const a = m[row * 4], b = m[row * 4 + 1], c = m[row * 4 + 2];
    if (Math.sqrt(a * a + b * b + c * c) < 0.05) {
      failCounts[2] += 1;
      if (explain) lastReason = 'world axis ' + row + ' does not reach the screen';
      return null;
    }
  }
  // THE SHAPE OF `V * P`, WHICH NEEDS NOBODY TO MOVE.
  //
  // Write the view matrix's upper three rows as a rotation `R` and the projection as
  // `diag(sx, sy, A, .)` with a `(0,0,1,0)` w column. Multiplying them out fixes the four world
  // columns of the product exactly:
  //
  //   c0 = sx * R[:,0]      c1 = sy * R[:,1]      c2 = A * R[:,2]      c3 = R[:,2]
  //
  // So `c0`, `c1` and `c3` are mutually perpendicular, `|c3|` is 1, `c2` is PARALLEL to `c3`, and
  // `|c1| / |c0|` is the aspect ratio -- measured elsewhere in this game as 1.7806.
  //
  // This matters because every other test here needs the character to be somewhere, and the
  // character has been standing still: 1442 consecutive passes at `spread 0.00 m`, which proves
  // nothing about anything. These are properties of the matrix alone.
  // SCALARS, NOT ARRAYS. `score` runs about ninety thousand times per discovery window; writing
  // this with `columns.map(...)` and a `dot` closure allocated roughly a million short-lived
  // objects a second and put the JS thread straight back into the starvation this file has
  // already been rescued from twice. It is the same arithmetic, spelled out.
  const ax = m[0], ay = m[4], az = m[8];      // c0 = sx * right
  const bx = m[1], by = m[5], bz = m[9];      // c1 = sy * up
  const cx = m[2], cy = m[6], cz = m[10];     // c2 = A  * forward
  const fx = m[3], fy = m[7], fz = m[11];     // c3 =      forward
  const la = Math.sqrt(ax * ax + ay * ay + az * az);
  const lb = Math.sqrt(bx * bx + by * by + bz * bz);
  const lc = Math.sqrt(cx * cx + cy * cy + cz * cz);
  if (!(la > 1e-3 && lb > 1e-3 && lc > 1e-3)) { failCounts[3] += 1; if (explain) lastReason = 'a column collapsed'; return null; }
  const ab = Math.abs((ax * bx + ay * by + az * bz) / (la * lb));
  if (ab > ORTHOGONAL_TOLERANCE) {
    failCounts[4] += 1;
    if (explain) lastReason = 'right and up are not perpendicular (' + ab.toFixed(4) + ')';
    return null;
  }
  const af = Math.abs(ax * fx + ay * fy + az * fz) / la;
  if (af > ORTHOGONAL_TOLERANCE) {
    failCounts[5] += 1;
    if (explain) lastReason = 'right and forward are not perpendicular (' + af.toFixed(4) + ')';
    return null;
  }
  const bf = Math.abs(bx * fx + by * fy + bz * fz) / lb;
  if (bf > ORTHOGONAL_TOLERANCE) {
    failCounts[6] += 1;
    if (explain) lastReason = 'up and forward are not perpendicular (' + bf.toFixed(4) + ')';
    return null;
  }
  const cf = Math.abs(cx * fx + cy * fy + cz * fz) / lc;
  if (cf < 1.0 - PARALLEL_TOLERANCE) {
    failCounts[7] += 1;
    if (explain) lastReason = 'column 2 is not parallel to the forward axis (' + cf.toFixed(4) + ')';
    return null;
  }
  const aspect = lb / la;
  if (!(aspect > MIN_ASPECT && aspect < MAX_ASPECT)) {
    failCounts[8] += 1;
    if (explain) lastReason = 'aspect ' + aspect.toFixed(4) + ' is not a screen';
    return null;
  }
  // EVERYTHING ABOVE IS A PROPERTY OF THE MATRIX; EVERYTHING BELOW NEEDS A CHARACTER.
  //
  // `GameManagerImp`'s player pointer is null whenever nobody is in the world -- a menu, a load, the
  // title screen -- and a whole run was spent reporting `0 scans` for that reason alone, because
  // the character read gated the scan rather than only the tests that need it. The camera is
  // uploaded on those frames too, and its shape is just as recognisable, so the hunt does not stop
  // because nobody is standing anywhere.
  if (player === null) {
    lastReason = '';
    return { ndcX: 0, ndcY: 0, w: 0, forward, structural: true };
  }
  // SCALARS BELOW THIS LINE TOO, and it is not a style preference.
  //
  // This half of `score` had never actually run: `finite` was rejecting every character read, so
  // the function returned at the branch above on every one of its ninety thousand calls per
  // window. Fixing that read turned this code on for the first time, and the version that was
  // sitting here built an array in `toClip`, another three for the world axes, and three more
  // clip vectors -- eight allocations a call, seven hundred thousand a window. That is the shape
  // of the four JS-thread starvations this file has already been rescued from, in code that had
  // been sitting here for hours looking harmless because nothing ever reached it.
  //
  // REWRITTEN ON THAT REASONING, NOT ON A MEASURED STALL. The silence that prompted it was the
  // watcher's stdout block-buffering and the agent was heartbeating throughout; run the watcher
  // under `PYTHONUNBUFFERED=1` and that ghost does not appear. The allocation count is real and
  // the rewrite stands on it; the stall it was blamed for did not happen.
  const clipW = player[0] * m[3] + player[1] * m[7] + player[2] * m[11] + m[15];
  if (!(clipW > CAMERA_MIN_METERS && clipW < CAMERA_MAX_METERS)) {
    failCounts[9] += 1;
    if (clipW < distanceMin) distanceMin = clipW;
    if (clipW > distanceMax) distanceMax = clipW;
    if (explain) lastReason = 'camera distance ' + clipW.toFixed(2) + ' m';
    return null;
  }
  const clipX = player[0] * m[0] + player[1] * m[4] + player[2] * m[8] + m[12];
  const clipY = player[0] * m[1] + player[1] * m[5] + player[2] * m[9] + m[13];
  const ndcX = clipX / clipW;
  const ndcY = clipY / clipW;
  if (!Number.isFinite(ndcX) || !Number.isFinite(ndcY)) { failCounts[10] += 1; if (explain) lastReason = 'ndc not finite'; return null; }
  if (Math.abs(ndcX) > NDC_LIMIT || Math.abs(ndcY) > NDC_LIMIT) {
    failCounts[11] += 1;
    offScreenX = ndcX;
    offScreenY = ndcY;
    if (explain) lastReason = 'character off screen at ' + ndcX.toFixed(3) + ',' + ndcY.toFixed(3);
    return null;
  }
  // A metre in world space has to be a sane number of pixels. THE FLOOR IS ACROSS THE THREE AXES,
  // NOT ON EACH: a metre along whichever world axis the camera happens to be looking down moves
  // the screen by exactly nothing, because that is what looking down an axis means. Requiring
  // every axis to move the image rejected a correct camera on every frame the player faced north,
  // and the failure is invisible from the log, which just stops finding anything for a while.
  //
  // Adding one metre to world component `axis` adds row `axis` of the matrix to the clip vector,
  // so the stepped point costs three adds rather than a point, a matrix multiply and a vector.
  let biggest = 0;
  for (let axis = 0; axis < 3; axis += 1) {
    const row = axis * 4;
    const nearW = clipW + m[row + 3];
    if (!(nearW > 0.05)) { failCounts[12] += 1; if (explain) lastReason = 'a metre away lands behind the camera'; return null; }
    const dx = (clipX + m[row]) / nearW - ndcX;
    const dy = (clipY + m[row + 1]) / nearW - ndcY;
    const shift = Math.sqrt(dx * dx + dy * dy);
    if (!(Number.isFinite(shift) && shift < 2.0)) {
      failCounts[13] += 1;
      if (explain) lastReason = 'a metre moves the screen by ' + shift.toFixed(4);
      return null;
    }
    if (shift > biggest) biggest = shift;
  }
  if (!(biggest > 0.005)) {
    failCounts[14] += 1;
    if (explain) lastReason = 'a metre moves the screen by ' + biggest.toFixed(4) + ' at most';
    return null;
  }
  lastReason = '';
  return { ndcX, ndcY, w: clipW, forward };
}

function entry(offset, convention) {
  return {
    offset, convention, passes: 0, grounded: 0, best: null,
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
  // A PASS WITH A CHARACTER IN IT IS A DIFFERENT KIND OF EVIDENCE, AND IS COUNTED SEPARATELY.
  //
  // Every structural test in `score` is satisfied by the BARE PROJECTION MATRIX `P`, because
  // `P = I * P` is a view-projection whose view is the identity: unit forward column, mutually
  // perpendicular columns, column 2 parallel to column 3, 16:9 aspect. All true, all useless.
  // Measured here on 2026-09-22 -- the upload at `0x5327a20+0x0` confirmed on 60 structural
  // passes at a menu, and it is plainly a projection with no camera in it at all:
  //
  //     [ 1.3901  0.0000  0.0000  0.0000        [ sx  0   0   0
  //       0.0000  2.4751  0.0000  0.0000    =     0   sy  0   0
  //       0.0000  0.0000  1.0000  1.0000          0   0   A   1
  //       0.0000  0.0000 -0.1000  0.0000 ]        0   0   B   0 ]
  //
  // The character is the only thing that separates the two: through `P` alone the world origin
  // is the camera, so a character standing anywhere real is far off screen or behind the lens,
  // while through `V * P` it sits a few metres in front of one. So believing a candidate now
  // costs `grounded` passes -- ones with a character in them -- and structural passes only keep
  // it alive through menus and loads.
  if (player === null || verdict.structural) return;
  candidate.grounded += 1;
  for (let axis = 0; axis < 3; axis += 1) {
    if (player[axis] < candidate.minimum[axis]) candidate.minimum[axis] = player[axis];
    if (player[axis] > candidate.maximum[axis]) candidate.maximum[axis] = player[axis];
  }
}

/** The slot following this resource, or undefined. Native compare, no allocation. */
function followed(resource) {
  for (let index = 0; index < trackedList.length; index += 1) {
    if (trackedList[index].pointer.equals(resource)) return trackedList[index];
  }
  return undefined;
}

/** Call ONLY after `scansThisWindow` has been checked: this one builds a string. */
function shouldDiscover(key) {
  const looks = examined.get(key) || 0;
  if (looks >= LOOKS_PER_RESOURCE) return false;
  examined.set(key, looks + 1);
  scansThisWindow += 1;
  return true;
}

function discover(resource, source, label, player) {
  // Never explain a rejection here: this loop rejects ninety thousand windows per window of time
  // and the messages are what wedged the thread.
  const explain = false;
  counters.scans += 1;
  const key = resource.toString();
  for (let offset = 0; offset + 64 <= SCAN_BYTES; offset += 4) {
    if (!readMatrix(source.add(offset))) return;
    transposeRaw();
    for (let which = 0; which < 2; which += 1) {
      const convention = which === 0 ? 'as stored' : 'transposed';
      const m = which === 0 ? RAW : TRANSPOSED;
      const verdict = score(m, player, explain);
      if (verdict === null) continue;
      counters.scored += 1;
      let slot = tracked.get(key);
      if (slot === undefined) {
        slot = { key, label, pointer: resource, offsets: new Map() };
        tracked.set(key, slot);
        trackedList.push(slot);
        gate = GATE_WHILE_FOLLOWING;
      }
      const name = offset + '|' + convention;
      if (!slot.offsets.has(name)) { slot.offsets.set(name, entry(offset, convention)); counters.tracked += 1; }
      note(slot.offsets.get(name), verdict, player);
    }
  }
}

function prune(slot) {
  if (slot.offsets.size > 0) return;
  tracked.delete(slot.key);
  const at = trackedList.indexOf(slot);
  if (at !== -1) trackedList.splice(at, 1);
  if (trackedList.length === 0) gate = GATE_WHILE_HUNTING;
}

function follow(slot, source, player) {
  // Explain here: this runs a few times a second, and a candidate dying is the one event in this
  // agent that says something worth reading.
  const explain = true;
  counters.follows += 1;
  for (const [name, candidate] of slot.offsets) {
    if (!readMatrix(source.add(candidate.offset))) return;
    let m = RAW;
    if (candidate.convention === 'transposed') { transposeRaw(); m = TRANSPOSED; }
    const verdict = score(m, player, explain);
    if (verdict === null) {
      // A candidate that survived a while and then died is the interesting one: either it was a
      // coincidence that ran out, or it is the camera and one of these bounds is wrong. Say which
      // bound, because guessing which is what cost the last hour.
      if (candidate.passes >= 8) {
        console.log('[arrow] dropped ' + slot.label + '+0x' + candidate.offset.toString(16) +
          ' ' + candidate.convention + ' after ' + candidate.passes + ' passes (spread ' +
          spreadOf(candidate).toFixed(2) + ' m): ' + lastReason);
      }
      slot.offsets.delete(name);
      continue;
    }
    note(candidate, verdict, player);
    // COPY. `m` is one of two scratch buffers that the very next window overwrites; handing the
    // live buffer to the Present hook would have it drawing with whatever was read last.
    if (believed !== null) { believed.matrix = m.slice(); continue; }
    if (candidate.grounded < PASSES_TO_BELIEVE) continue;
    if (spreadOf(candidate) < SPREAD_TO_BELIEVE) continue;
    believed = { slot, candidate, matrix: m.slice() };
    // The camera can be confirmed from its shape alone with nobody in the world, so there may be
    // no character to drop a breadcrumb at yet. `draw` drops it the first frame there is one.
    breadcrumb = player === null ? null : [player[0], player[1], player[2]];
    console.log(
      '\n[arrow] ===== VIEW-PROJECTION CONFIRMED =====\n' +
      '[arrow] ' + slot.label + ' resource ' + slot.key + ' +0x' + candidate.offset.toString(16) +
      ' ' + candidate.convention + '\n' +
      '[arrow] ' + candidate.grounded + ' of ' + candidate.passes +
      ' consecutive frames had a character in them; it moved ' +
      spreadOf(candidate).toFixed(2) + ' m across them\n' +
      '[arrow] player ndc ' + verdict.ndcX.toFixed(4) + ',' + verdict.ndcY.toFixed(4) +
      ' (w=' + verdict.w.toFixed(2) + ' m)\n' +
      '[arrow] ' + (breadcrumb === null
        ? 'no character in the world yet; the breadcrumb drops on the first frame there is one'
        : 'breadcrumb dropped at ' + breadcrumb.map((v) => v.toFixed(2)).join(', ') +
          ' -- WALK AWAY AND SPIN: the arrow must keep pointing back at it') +
      format(m)
    );
  }
  prune(slot);
}

// ---------------------------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------------------------

function guid(text) {
  const parts = text.split('-');
  const bytes = Memory.alloc(16);
  bytes.writeU32(parseInt(parts[0], 16));
  bytes.add(4).writeU16(parseInt(parts[1], 16));
  bytes.add(6).writeU16(parseInt(parts[2], 16));
  const tail = parts[3] + parts[4];
  for (let index = 0; index < 8; index += 1) {
    bytes.add(8 + index).writeU8(parseInt(tail.substr(index * 2, 2), 16));
  }
  return bytes;
}

const IID_ID3D11TEXTURE2D = guid('6f15aaf2-d208-4e89-9ab4-489535d34f9c');
const IID_ID3D11DEVICE = guid('db6f6ddb-ac77-4e88-8253-819df9bbf140');

function slot(object, index, returns, argTypes) {
  const address = object.readPointer().add(index * Process.pointerSize).readPointer();
  return new NativeFunction(address, returns, ['pointer'].concat(argTypes));
}

/** Builds a throwaway device, checks the two vtables against values only they can return, and
 *  resolves the methods the Present hook needs. Returns a refusal string, or null on success. */
function prepare() {
  const module = Process.findModuleByName('d3d11.dll');
  if (module === null) return 'd3d11.dll not loaded';
  const create = module.findExportByName('D3D11CreateDevice');
  if (create === null) return 'no D3D11CreateDevice';
  const createDevice = new NativeFunction(create, 'int', [
    'pointer', 'int', 'pointer', 'uint', 'pointer', 'uint', 'uint',
    'pointer', 'pointer', 'pointer',
  ]);
  const devicePointer = Memory.alloc(Process.pointerSize);
  const contextPointer = Memory.alloc(Process.pointerSize);
  const result = createDevice(NULL, 1, NULL, 0, NULL, 0, 7, devicePointer, NULL, contextPointer);
  if (result < 0) return 'D3D11CreateDevice failed: 0x' + (result >>> 0).toString(16);
  const device = devicePointer.readPointer();
  const context = contextPointer.readPointer();
  if (device.isNull() || context.isNull()) return 'D3D11CreateDevice returned null';

  // Only `FinishCommandList` answers DXGI_ERROR_INVALID_CALL on an immediate context.
  const finish = slot(context, SLOT_FINISH_COMMAND_LIST, 'int', ['int', 'pointer']);
  const answer = finish(context, 0, NULL) >>> 0;
  if (answer !== DXGI_ERROR_INVALID_CALL) {
    return 'slot ' + SLOT_FINISH_COMMAND_LIST + ' is not FinishCommandList (got 0x' +
      answer.toString(16) + ', wanted 0x' + DXGI_ERROR_INVALID_CALL.toString(16) +
      '); refusing to call ClearView at ' + SLOT_CLEAR_VIEW;
  }
  // Only `GetFeatureLevel` answers one of the D3D_FEATURE_LEVEL constants.
  const level = slot(device, SLOT_GET_FEATURE_LEVEL, 'uint', [])(device) >>> 0;
  if (KNOWN_FEATURE_LEVELS.indexOf(level) === -1) {
    return 'slot ' + SLOT_GET_FEATURE_LEVEL + ' is not GetFeatureLevel (got 0x' +
      level.toString(16) + '); refusing to call CreateRenderTargetView at ' +
      SLOT_CREATE_RENDER_TARGET_VIEW;
  }
  console.log('[arrow] vtables verified: FinishCommandList at ' + SLOT_FINISH_COMMAND_LIST +
    ', feature level 0x' + level.toString(16) + '. ClearView at ' + SLOT_CLEAR_VIEW + '.');
  return null;
}

/** One-time per swap chain: back buffer -> render target view, plus the device context to clear
 *  through and the frame size to draw in. */
function attachTo(swapChain) {
  const device = slot(swapChain, SLOT_SWAPCHAIN_GET_DEVICE, 'int', ['pointer', 'pointer']);
  const out = Memory.alloc(Process.pointerSize);
  if (device(swapChain, IID_ID3D11DEVICE, out) < 0 || out.readPointer().isNull()) return null;
  const d3dDevice = out.readPointer();

  const buffer = slot(swapChain, SLOT_GET_BUFFER, 'int', ['uint', 'pointer', 'pointer']);
  const texture = Memory.alloc(Process.pointerSize);
  if (buffer(swapChain, 0, IID_ID3D11TEXTURE2D, texture) < 0) return null;
  if (texture.readPointer().isNull()) return null;

  const makeView = slot(d3dDevice, SLOT_CREATE_RENDER_TARGET_VIEW, 'int',
    ['pointer', 'pointer', 'pointer']);
  const view = Memory.alloc(Process.pointerSize);
  if (makeView(d3dDevice, texture.readPointer(), NULL, view) < 0) return null;
  if (view.readPointer().isNull()) return null;

  const describe = slot(swapChain, SLOT_GET_DESC, 'int', ['pointer']);
  const desc = Memory.alloc(256);
  if (describe(swapChain, desc) < 0) return null;
  const width = desc.readU32();
  const height = desc.add(4).readU32();
  if (!(width > 0 && height > 0)) return null;

  const contextOut = Memory.alloc(Process.pointerSize);
  slot(d3dDevice, 40, 'void', ['pointer'])(d3dDevice, contextOut);
  const context = contextOut.readPointer();
  if (context.isNull()) return null;

  console.log('[arrow] attached to the swap chain: ' + width + 'x' + height + '.');
  return {
    context,
    clear: slot(context, SLOT_CLEAR_VIEW, 'void', ['pointer', 'pointer', 'pointer', 'uint']),
    view: view.readPointer(),
    width,
    height,
    colour: Memory.alloc(16),
    rects: Memory.alloc(16 * 512),
  };
}

/** A thick line as a run of squares, appended into the shared rect buffer. */
function lay(state, count, from, to, limit) {
  const span = Math.hypot(to[0] - from[0], to[1] - from[1]);
  const steps = Math.max(1, Math.min(limit - count, Math.ceil(span / (STROKE_PX * 0.4))));
  const half = STROKE_PX * 0.5;
  for (let step = 0; step <= steps && count < limit; step += 1) {
    const t = steps === 0 ? 0 : step / steps;
    const x = from[0] + (to[0] - from[0]) * t;
    const y = from[1] + (to[1] - from[1]) * t;
    const left = Math.max(0, Math.round(x - half));
    const top = Math.max(0, Math.round(y - half));
    const right = Math.min(state.width, Math.round(x + half));
    const bottom = Math.min(state.height, Math.round(y + half));
    if (right <= left || bottom <= top) continue;
    const rect = state.rects.add(count * 16);
    rect.writeS32(left);
    rect.add(4).writeS32(top);
    rect.add(8).writeS32(right);
    rect.add(12).writeS32(bottom);
    count += 1;
  }
  return count;
}

function paint(state, colour, count) {
  if (count === 0) return;
  for (let index = 0; index < 4; index += 1) state.colour.add(index * 4).writeFloat(colour[index]);
  state.clear(state.context, state.view, state.colour, state.rects, count);
}

/// Unit screen direction, IN PIXELS, from the character to a world point. Null if the two land on
/// the same pixel, which is the arrow having nothing to say rather than an error.
function bearing(state, m, here, target) {
  const there = toClip(m, target);
  // Cross-multiplied, so `there[3]` is never divided by: these numerators are the NDC difference
  // times the common factor `there[3] * here[3]`, which normalising cancels.
  //
  // AND THAT ALREADY HANDLES A TARGET BEHIND THE CAMERA -- do not "fix" it again. Behind the lens
  // the projection is the reflection, so the honest NDC difference points at the mirror image,
  // the wrong way round. But `there[3]` is negative there, and it is sitting in the common factor
  // these numerators carry, so the numerators come out flipped relative to that NDC difference:
  // pointing away from the mirror, which is the correct arrow, with no special case at all. An
  // explicit `if (there[3] < 0) negate` on top of this flips it a second time and sends the arrow
  // confidently at the reflection -- caught by the selftest, never by looking at it.
  const dx = there[0] * here[3] - here[0] * there[3];
  const dy = there[1] * here[3] - here[1] * there[3];
  // NDC IS NOT PIXELS, AND THIS IS WHERE THE LAST ARROW WAS WRONG. One NDC unit is `width / 2`
  // pixels across but only `height / 2` pixels down, so taking the angle before that scaling
  // shears every bearing by the aspect ratio: a true 45 degrees comes out at 60.7 on this 16:9
  // screen, and 15.7 degrees of error is exactly the kind that looks plausible in a log and
  // points at the wrong door in the game. Scale first, then take the angle.
  const px = dx * state.width;
  const py = -dy * state.height;
  const length = Math.hypot(px, py);
  return length > 1e-9 ? [px / length, py / length] : null;
}

/** Shaft plus two barbs, appended into the shared rect buffer. */
function arrow(state, count, baseX, baseY, unit, lengthPx) {
  const tip = [baseX + unit[0] * lengthPx, baseY + unit[1] * lengthPx];
  const heading = Math.atan2(unit[1], unit[0]);
  const barb = lengthPx * (ARROW_BARB_PX / ARROW_LENGTH_PX);
  count = lay(state, count, [baseX, baseY], tip, 512);
  for (const sign of [-1, 1]) {
    const angle = heading + Math.PI + sign * ARROW_BARB_RADIANS;
    count = lay(state, count, tip,
      [tip[0] + Math.cos(angle) * barb, tip[1] + Math.sin(angle) * barb], 512);
  }
  return count;
}

/// Four exact predictions about a view-projection, measured on the live matrix and printed once.
///
/// For a row-vector `VP = V * P` with rotation `R` and a diagonal projection, the four world
/// columns are `c0 = sx*right`, `c1 = sy*up`, `c2 = A*forward` and `c3 = forward`. Stepping a
/// world point one metre along each of those axes therefore has an answer that is arithmetic, not
/// opinion, and every one of them is a different way for a wrong matrix to fail:
///
///   * along `c3`: clip.x and clip.y DO NOT MOVE and clip.w rises by exactly one. Walking away
///     from the camera slides a target toward the vanishing point and nowhere else, and `w` is
///     metres. A transposed or column-major read fails this immediately.
///   * along `c0`: clip.x rises by exactly `|c0|`, with clip.y and clip.w untouched. Camera-right
///     is purely horizontal on screen.
///   * along `c1`: clip.y rises by exactly `|c1|`, with clip.x and clip.w untouched.
///   * `|c1| / |c0|` is the aspect ratio, which must match the swap chain the game is actually
///     presenting to -- the one number here that is checked against something outside the matrix.
function proof(state, m, player, here) {
  proved = true;
  const length3 = (a, b, c) => Math.sqrt(a * a + b * b + c * c);
  const sx = length3(m[0], m[4], m[8]);
  const sy = length3(m[1], m[5], m[9]);
  const sf = length3(m[3], m[7], m[11]);
  const axes = {
    'c3 forward': [m[3] / sf, m[7] / sf, m[11] / sf],
    'c0 right  ': [m[0] / sx, m[4] / sx, m[8] / sx],
    'c1 up     ': [m[1] / sy, m[5] / sy, m[9] / sy],
  };
  const want = { 'c3 forward': [0, 0, 1], 'c0 right  ': [sx, 0, 0], 'c1 up     ': [0, sy, 0] };

  // The camera does not roll, so the largest component of its up axis names the world's up axis
  // and its sign says which way that axis points. Derived and printed, never assumed to be +Y.
  const up = axes['c1 up     '];
  let biggest = 0;
  for (let index = 1; index < 3; index += 1) {
    if (Math.abs(up[index]) > Math.abs(up[biggest])) biggest = index;
  }
  upAxis = [0, 0, 0];
  upAxis[biggest] = up[biggest] > 0 ? 1 : -1;

  const lines = [];
  let failures = 0;
  for (const name of Object.keys(axes)) {
    const axis = axes[name];
    const moved = toClip(m, [player[0] + axis[0], player[1] + axis[1], player[2] + axis[2]]);
    const got = [moved[0] - here[0], moved[1] - here[1], moved[3] - here[3]];
    const expected = want[name];
    const scale = Math.max(1, sx, sy);
    const ok = got.every((value, index) => Math.abs(value - expected[index]) < 2e-3 * scale);
    if (!ok) failures += 1;
    lines.push('[arrow]   step 1 m along ' + name + ' -> d(clip.x, clip.y, clip.w) = (' +
      got.map((v) => v.toFixed(5).padStart(10)).join(', ') + ')  want (' +
      expected.map((v) => v.toFixed(5).padStart(10)).join(', ') + ')  ' + (ok ? 'PASS' : 'FAIL'));
  }

  const fromMatrix = sy / sx;
  const fromScreen = state.width / state.height;
  const drift = Math.abs(fromMatrix - fromScreen) / fromScreen;
  const aspectOk = drift < 0.01;
  if (!aspectOk) failures += 1;

  console.log(
    '\n[arrow] ===== POINTING PROOF =====\n' +
    lines.join('\n') + '\n' +
    '[arrow]   aspect |c1|/|c0| = ' + fromMatrix.toFixed(4) + ', swap chain ' + state.width + 'x' +
    state.height + ' = ' + fromScreen.toFixed(4) + ' (' + (drift * 100).toFixed(2) + '% apart)  ' +
    (aspectOk ? 'PASS' : 'FAIL') + '\n' +
    '[arrow]   camera is ' + here[3].toFixed(2) + ' m from the character; world up is ' +
    (upAxis[0] !== 0 ? 'X' : upAxis[1] !== 0 ? 'Y' : 'Z') +
    (upAxis[biggest] > 0 ? '+' : '-') + ' (camera up ' +
    up.map((v) => v.toFixed(3)).join(', ') + ')\n' +
    '[arrow] ' + (failures === 0
      ? 'ARITHMETIC IS CLEAN. The GREEN arrow is a world-space vertical and must point UP THE\n' +
        '[arrow] SCREEN from every camera angle; the CYAN dot is the character\'s projected position\n' +
        '[arrow] and must sit on the character. Those two are the part only your eyes can check.'
      : failures + ' TESTS FAILED -- the arrow below is pointing somewhere, but not where it claims.')
  );
}

function draw(state) {
  const player = playerPosition();
  if (player === null || believed === null) return;
  if (breadcrumb === null) {
    breadcrumb = [player[0], player[1], player[2]];
    console.log('[arrow] breadcrumb dropped at ' + breadcrumb.map((v) => v.toFixed(2)).join(', ') +
      ' -- WALK AWAY AND SPIN: the arrow must keep pointing back at it.');
  }
  const m = believed.matrix;

  const here = toClip(m, player);
  if (!(here[3] > 0.05)) return;
  // THE BASE IS WHERE THE CHARACTER IS, not the middle of the frame by assumption. If those two
  // are not the same place the matrix is wrong, and drawing the base at the character is what
  // makes that visible instead of hiding it.
  const baseX = (here[0] / here[3] * 0.5 + 0.5) * state.width;
  const baseY = (0.5 - here[1] / here[3] * 0.5) * state.height;
  if (!proved) proof(state, m, player, here);

  const plumb = bearing(state, m, here, [
    player[0] + upAxis[0] * PLUMB_METRES,
    player[1] + upAxis[1] * PLUMB_METRES,
    player[2] + upAxis[2] * PLUMB_METRES,
  ]);
  if (plumb !== null) {
    paint(state, PLUMB_COLOUR, arrow(state, 0, baseX, baseY, plumb, PLUMB_LENGTH_PX));
  }

  const unit = bearing(state, m, here, breadcrumb);
  if (unit !== null) {
    paint(state, ARROW_COLOUR, arrow(state, 0, baseX, baseY, unit, ARROW_LENGTH_PX));
  }
  paint(state, BASE_COLOUR, lay(state, 0, [baseX, baseY], [baseX, baseY], 4));
  counters.drawn += 1;
}

function hook() {
  refusal = prepare();
  if (refusal !== null) {
    console.log('[arrow] REFUSING TO DRAW: ' + refusal);
    return;
  }
  const module = Process.findModuleByName('d3d11.dll');
  const createAll = module.findExportByName('D3D11CreateDeviceAndSwapChain');
  if (createAll === null) {
    console.log('[arrow] no D3D11CreateDeviceAndSwapChain; cannot reach IDXGISwapChain::Present.');
    return;
  }
  // The swap chain's vtable is shared by every swap chain in the process, so one built here names
  // the same `Present` the game's uses.
  const makeAll = new NativeFunction(createAll, 'int', [
    'pointer', 'int', 'pointer', 'uint', 'pointer', 'uint', 'uint',
    'pointer', 'pointer', 'pointer', 'pointer', 'pointer',
  ]);
  const window = Memory.alloc(8);
  const user32 = Process.findModuleByName('user32.dll');
  const getDesktop = user32 === null ? null : user32.findExportByName('GetDesktopWindow');
  const hwnd = getDesktop === null ? NULL : new NativeFunction(getDesktop, 'pointer', [])();

  const desc = Memory.alloc(256);
  desc.writeU32(0);                       // BufferDesc.Width  (0 = from window)
  desc.add(4).writeU32(0);                // BufferDesc.Height
  desc.add(8).writeU32(0);                // RefreshRate.Numerator
  desc.add(12).writeU32(0);               // RefreshRate.Denominator
  desc.add(16).writeU32(28);              // DXGI_FORMAT_R8G8B8A8_UNORM
  desc.add(20).writeU32(0);
  desc.add(24).writeU32(0);
  desc.add(28).writeU32(1);               // SampleDesc.Count
  desc.add(32).writeU32(0);               // SampleDesc.Quality
  desc.add(36).writeU32(32);              // DXGI_USAGE_RENDER_TARGET_OUTPUT
  desc.add(40).writeU32(1);               // BufferCount
  desc.add(48).writePointer(hwnd);        // OutputWindow
  desc.add(56).writeU32(1);               // Windowed
  desc.add(60).writeU32(0);               // SwapEffect
  desc.add(64).writeU32(0);               // Flags

  const chain = Memory.alloc(Process.pointerSize);
  const device = Memory.alloc(Process.pointerSize);
  const context = Memory.alloc(Process.pointerSize);
  const made = makeAll(NULL, 1, NULL, 0, NULL, 0, 7, desc, chain, device, NULL, context);
  if (made < 0 || chain.readPointer().isNull()) {
    console.log('[arrow] could not build a swap chain to read Present from: 0x' +
      (made >>> 0).toString(16));
    return;
  }
  const present = chain.readPointer().readPointer()
    .add(SLOT_PRESENT * Process.pointerSize).readPointer();

  Interceptor.attach(present, {
    onEnter(args) {
      counters.present += 1;
      if (believed === null) return;
      try {
        if (drawState === null) drawState = attachTo(args[0]);
        if (drawState !== null) draw(drawState);
      } catch (error) {
        console.log('[arrow] draw failed, disabling: ' + error.message);
        believed = null;
      }
    },
  });
  console.log('[arrow] hooked IDXGISwapChain::Present (slot ' + SLOT_PRESENT + ').');
  hookUploads();
}

function hookUploads() {
  const module = Process.findModuleByName('d3d11.dll');
  const create = module.findExportByName('D3D11CreateDevice');
  const createDevice = new NativeFunction(create, 'int', [
    'pointer', 'int', 'pointer', 'uint', 'pointer', 'uint', 'uint',
    'pointer', 'pointer', 'pointer',
  ]);
  const devicePointer = Memory.alloc(Process.pointerSize);
  const contextPointer = Memory.alloc(Process.pointerSize);
  createDevice(NULL, 1, NULL, 0, NULL, 0, 7, devicePointer, NULL, contextPointer);
  const vtable = contextPointer.readPointer().readPointer();

  Interceptor.attach(vtable.add(SLOT_UPDATE_SUBRESOURCE * Process.pointerSize).readPointer(), {
    onEnter(args) {
      const seen = (counters.update += 1);
      if (seen % gate !== 0) return;
      const resource = args[1];
      const source = args[4];
      if (resource.isNull() || source.isNull()) return;
      const slotFound = followed(resource);
      if (slotFound !== undefined) {
        const player = playerPosition();
        follow(slotFound, source, player);
        return;
      }
      // Integer first, string second. Everything below here is off the hot path by construction:
      // once the window's budget is spent this hook is a pointer loop and a compare.
      if (believed !== null) return;
      if (scansThisWindow >= SCANS_PER_WINDOW) { throttled += 1; return; }
      // THE PLAYER READ COMES BEFORE THE BUDGET IS SPENT. With these two the other way round the
      // heartbeat read `20 resources seen, 0 scans`: every resource's twenty looks were consumed
      // by `shouldDiscover` and then thrown away because the character was not readable, so each
      // buffer was permanently written off without ever being examined once.
      const player = playerPosition();
      if (player === null) unreadable += 1;
      if (!shouldDiscover(resource.toString())) return;
      discover(resource, source, 'UpdateSubresource', player);
    },
  });

  Interceptor.attach(vtable.add(SLOT_MAP * Process.pointerSize).readPointer(), {
    onEnter(args) {
      const seen = (counters.map += 1);
      if (seen % gate !== 0) return;
      this.resource = args[1];
      this.mapped = args[5];
    },
    onLeave() {
      if (!this.mapped || this.mapped.isNull()) return;
      if (!this.resource || this.resource.isNull()) return;
      let data;
      try { data = this.mapped.readPointer(); } catch (error) { return; }
      if (data.isNull()) return;
      const slotFound = followed(this.resource);
      if (slotFound !== undefined) {
        const player = playerPosition();
        follow(slotFound, data, player);
        return;
      }
      if (believed !== null) return;
      if (scansThisWindow >= SCANS_PER_WINDOW) { throttled += 1; return; }
      const player = playerPosition();
      if (player === null) unreadable += 1;
      if (!shouldDiscover(this.resource.toString())) return;
      discover(this.resource, data, 'Map', player);
    },
  });
  console.log('[arrow] hooked UpdateSubresource (' + SLOT_UPDATE_SUBRESOURCE + ') and Map (' +
    SLOT_MAP + '); following by resource.');
}

setInterval(function () {
  if (refusal !== null) { console.log('[arrow] refusing to draw: ' + refusal); return; }
  const leaders = [];
  for (const [key, slotFound] of tracked) {
    for (const [, candidate] of slotFound.offsets) leaders.push({ key, slotFound, candidate });
  }
  leaders.sort((a, b) => (b.candidate.grounded - a.candidate.grounded) ||
    (b.candidate.passes - a.candidate.passes));
  let line =
    '[arrow] heartbeat: ' + counters.present + ' present, ' + counters.drawn + ' drawn, ' +
    counters.update + ' update, ' + counters.map + ' map, ' + examined.size + ' resources seen, ' +
    counters.scans + ' scans, ' + counters.scored + ' scored, ' + counters.tracked + ' tracked, ' +
    counters.follows + ' follows' + (throttled > 0 ? ', THROTTLED ' + throttled : '') +
    (unreadable > 0 ? ', CHARACTER UNREADABLE ' + unreadable + ' times' : '');
  unreadable = 0;
  for (const leader of leaders.slice(0, 4)) {
    line += '\n      ' + leader.slotFound.label + ' ' + leader.key + '+0x' +
      leader.candidate.offset.toString(16) + ' ' + leader.candidate.convention + ': ' +
      leader.candidate.passes + ' passes (' + leader.candidate.grounded +
      ' with a character), spread ' + spreadOf(leader.candidate).toFixed(2) + ' m';
  }
  // WHY NOTHING PASSED, every window, whether or not anything was being followed. `0 scored` on
  // its own has twice sent this session guessing at which bound was too tight.
  const blame = [];
  for (let code = 0; code < failCounts.length; code += 1) {
    if (failCounts[code] > 0) blame.push(FAIL_NAMES[code] + ' ' + failCounts[code]);
    failCounts[code] = 0;
  }
  if (blame.length > 0) line += '\n      rejected by: ' + blame.join(', ');
  if (Number.isFinite(distanceMin)) {
    line += '\n      camera distance saw ' + distanceMin.toFixed(2) + ' m to ' +
      distanceMax.toFixed(2) + ' m against the band ' + CAMERA_MIN_METERS + '..' +
      CAMERA_MAX_METERS + ' m';
  }
  if (offScreenX !== 0 || offScreenY !== 0) {
    line += '\n      furthest off screen: ndc ' + offScreenX.toFixed(3) + ',' +
      offScreenY.toFixed(3) + ' against the limit ' + NDC_LIMIT;
  }
  distanceMin = Infinity;
  distanceMax = -Infinity;
  offScreenX = 0;
  offScreenY = 0;
  // FORGET WHAT WAS LOOKED AT, SO DISCOVERY NEVER RETIRES. `examined` counts each resource's
  // twenty looks and was never cleared, so after about eight hundred looks -- a few seconds --
  // every buffer in the game was permanently written off and `shouldDiscover` refused
  // everything. The heartbeat then reads `39 resources seen, 345 scans, 0 scored` forever, which
  // looks like the tests being too tight and is nothing of the kind: nothing is being examined
  // at all. It matters because a buffer's CONTENTS change every frame -- twenty looks during a
  // loading screen says nothing about what the same buffer holds once a world is up, and the
  // camera was found in exactly such a buffer.
  //
  // Cleared only while nothing is tracked: once a candidate exists the budget's whole job is to
  // stay out of the way of following it.
  if (trackedList.length === 0) examined.clear();
  scansThisWindow = 0;
  throttled = 0;
  console.log(line);
}, 5000);

// DETACH BEFORE HOOKING, EVERY LOAD.
//
// The watcher reloads this file in place on every save and does not call `dispose` first, so each
// load stacked another interceptor on `UpdateSubresource` on top of all the previous ones. After a
// few dozen edits that is a few dozen copies of this agent's `onEnter` running on every one of
// forty-five thousand uploads a second, and the JS thread stops dead -- which looks exactly like
// the allocation starvation that is fixed elsewhere in this file, and is not it.
Interceptor.detachAll();
hook();

rpc.exports = {
  dispose() {
    Interceptor.detachAll();
    console.log('[arrow] detached after ' + counters.drawn + ' frames drawn.');
  },
};
