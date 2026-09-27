// Prototype of ds2-weapon-sync: cap our weapon levels to the highest level any other player in the
// world has equipped, and put them back when the cap goes away.
//
// Do not attach this as it stands. Its one attach, 2026-09-26 ~15:57, never reported "attached",
// and the game (pid 2416423) was found with every thread idle afterwards: the main thread in
// futex_wait, no CPU in 3 s, and every later attach hanging. The prime suspect is `redrive()`
// calling 0x14037f890 from inside the NvNavigationSystem::Update onLeave (a re-entrant call into
// hooked code, or a lock). Not proven: the loader log carries no heartbeat to date the freeze. The
// shipped crate pushes from native code after the net session update returns instead.
//
// `python3 scripts/ds2-frida-up.py`, then attach a FROZEN COPY of this file (bd
// ds2-frida-hot-reload-kills-game-2026-09-25 -- a hot reload has killed the game):
//   cp scripts/frida/weapon-sync-cap.js $SCRATCH/cap-frozen.js
//   uv run --with frida python3 scripts/ds2-frida-watch.py --agent $SCRATCH/cap-frozen.js
//
// Control file, re-read every 30 frames: <Game>/weapon-sync.json
//   {"force": 3}      cap at +3 regardless of who is in the world (solo test of the mechanism)
//   {"force": null}   cap = highest remote-person weapon level; no remote person = no cap
//
// WHAT IT CHANGES, and what it never touches:
//   * 0x14037f890 (the local weapon update) is attached; onEnter lowers req+0xC to the cap.
//     That function writes the ChrAsmCtrl record table and ChrAsmEquip +0x70, then sends P2P
//     packet 61 built from the same req.
//   * When the cap changes, 0x14037f890 is called once per equipped internal weapon slot 0..5,
//     from inside NvNavigationSystem::Update (game thread), with a req built from the inventory
//     entry exactly as 0x1401b66a0 builds it. The call itself carries the capped level.
//   * The inventory entry (+0x25, the save's source) is only READ.
// Every packet 61 the game builds is printed (0x140162c50 onEnter), so what a peer would receive
// is visible even solo.
'use strict';

const image = Process.getModuleByName('DarkSoulsII.exe');
const at = (rva) => image.base.add(rva);
const WEAPON_UPDATE = at(0x37f890);
const SEND_PACKET_61 = at(0x162c50);
const NAV_UPDATE = at(0xbaeb20);
const PLAYER_CTRL_VTABLE = at(0x010e4bb8);
const INTERNAL_TO_CHR_SLOT = [1, 0, 3, 2, 5, 4]; // DAT_1410c44e0, read from the image
const say = (t) => { console.log('[weapon-sync-cap] ' + t); send({ kind: 'log', text: t }); };
const weaponUpdate = new NativeFunction(WEAPON_UPDATE, 'void', ['pointer', 'pointer']);
const controlPath = image.path.replace(/[^\\\/]*$/, '') + 'weapon-sync.json';

function p(base, off) {
  try { const v = base.add(off).readPointer(); return v.isNull() ? null : v; } catch (e) { return null; }
}
function u8(a) { try { return a.readU8(); } catch (e) { return null; } }

function playerCtrl() { const gm = p(at(0x16148f0), 0); return gm && p(gm, 0xd0); }

function bag() {
  const gm = p(at(0x16148f0), 0);
  const gdm = gm && p(gm, 0xa8);
  const inv = gdm && p(gdm, 0x10);
  const m = inv && p(inv, 0x10);
  return m && p(m, 0x10);
}

function charName(chr) {
  try {
    const s = chr.add(0x118);
    const len = s.add(0x10).readU64().toNumber();
    const cap = s.add(0x18).readU64().toNumber();
    if (len === 0 || len > 64) return '';
    return (cap > 7 ? s.readPointer() : s).readUtf16String(len);
  } catch (e) { return ''; }
}

// Highest record-table weapon level over every remote PERSON (NetworkPlayer_, not a replay).
function remoteMax() {
  const gm = p(at(0x16148f0), 0);
  const pc = gm && p(gm, 0xd0);
  const cm = gm && p(gm, 0x18);
  const b = cm && p(cm, 0x10), e = cm && p(cm, 0x18);
  if (!b || !e) return { people: 0, max: null };
  const n = Math.min(e.sub(b).toInt32() / 8, 512);
  let people = 0, max = null;
  for (let k = 0; k < n; k++) {
    const c = p(b, k * 8);
    if (!c || (pc && c.equals(pc))) continue;
    let vt; try { vt = c.readPointer(); } catch (x) { continue; }
    if (!vt.equals(PLAYER_CTRL_VTABLE)) continue;
    if (!charName(c).startsWith('NetworkPlayer')) continue;
    const blk = p(c, 0xb0);
    const pp = blk && u8(blk.add(0x3c));
    if (pp === 0x12 || pp === 0x13 || pp === null) continue;
    const asm = p(c, 0x378), table = asm && p(asm, 0x20);
    if (!table) continue;
    people++;
    for (let r = 0; r < 6; r++) {
      const lv = u8(table.add(8 + r * 0x14 + 0xe));
      if (lv !== null) max = max === null ? (lv & 0xf) : Math.max(max, lv & 0xf);
    }
  }
  return { people, max };
}

let cap = null; // null = no cap
let force = null;
let controlText = null;

function readControl() {
  let text;
  try { text = File.readAllText(controlPath); } catch (e) { text = ''; }
  if (text === controlText) return;
  controlText = text;
  try {
    const j = text ? JSON.parse(text) : {};
    force = typeof j.force === 'number' ? j.force : null;
  } catch (e) { force = null; }
  say('control ' + controlPath + ' force=' + force);
}

function snapshot(tag) {
  const pc = playerCtrl(), bg = bag();
  const asm = pc && p(pc, 0x378), table = asm && p(asm, 0x20), eq = asm && p(asm, 0x28);
  const parts = [];
  for (let i = 0; i < 6; i++) {
    const entry = bg && p(bg, 0x25830 + i * 8);
    const r = INTERNAL_TO_CHR_SLOT[i];
    const inv = entry ? (u8(entry.add(0x25)) & 0xf) : '-';
    const rec = table ? u8(table.add(8 + r * 0x14 + 0xe)) : '?';
    parts.push('s' + i + '(inv=' + inv + ' rec=' + rec + ')');
  }
  const live = [];
  for (let n = 0; eq && n < 6; n++) live.push(u8(eq.add(n * 0x48 + 0x70)));
  say(tag + ' cap=' + cap + ' ' + parts.join(' ') + ' live=[' + live.join(',') + ']');
}

function redrive() {
  const pc = playerCtrl(), bg = bag();
  if (!pc || !bg) { say('redrive skipped: pc=' + pc + ' bag=' + bg); return; }
  const req = Memory.alloc(0x10);
  let driven = 0;
  for (let i = 0; i < 6; i++) {
    const entry = p(bg, 0x25830 + i * 8);
    if (!entry) continue;
    const type = u8(entry.add(0x1e));
    const real = type < 6 ? (u8(entry.add(0x25)) & 0xf) : 0;
    const infusion = type < 2 ? (u8(entry.add(0x26)) & 0xf) : 0;
    const level = cap === null ? real : Math.min(real, cap);
    req.writeS32(INTERNAL_TO_CHR_SLOT[i]);
    req.add(4).writeU32(entry.add(0x14).readU32());
    req.add(8).writeU32(entry.add(0x20).readU32());
    req.add(0xc).writeU16(level);
    req.add(0xe).writeU8(infusion);
    req.add(0xf).writeU8(0);
    weaponUpdate(pc, req);
    driven++;
  }
  say('redrive slots=' + driven);
}

Interceptor.attach(WEAPON_UPDATE, {
  onEnter(args) {
    const req = args[1];
    const lv = req.add(0xc).readU8();
    if (cap !== null && lv > cap) {
      req.add(0xc).writeU8(cap);
      say('clamped game weapon update slot=' + req.readS32() + ' item=' + req.add(4).readU32() + ' +' + lv + ' -> +' + cap);
    }
  },
});

Interceptor.attach(SEND_PACKET_61, {
  onEnter(args) {
    const b = args[0];
    say('packet61 item=' + b.readU32() + ' slot=' + b.add(4).readU8() + ' broken=' + b.add(5).readU8() +
      ' level=' + b.add(6).readU8() + ' infusion=' + b.add(7).readU8());
  },
});

let frame = 0;
Interceptor.attach(NAV_UPDATE, {
  onLeave() {
    if (frame++ % 30 !== 0) return;
    readControl();
    const rm = remoteMax();
    const want = force !== null ? force : rm.max;
    if (want !== cap) {
      say('cap ' + cap + ' -> ' + want + ' (force=' + force + ' people=' + rm.people + ' remoteMax=' + rm.max + ')');
      snapshot('before');
      cap = want;
      redrive();
      snapshot('after');
    }
  },
});

readControl();
snapshot('attach');
