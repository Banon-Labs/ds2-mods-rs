// Read-only: the inputs of Attack_SwingSpeed, sampled on a timer while the player swings.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/attack-speed-read.js`
// Optional: `--config-json '{"clicks":3,"interval_ms":8}'` posts that many left-clicks (R1 on the
// default mouse binding) to the game's own window, 1.5 s apart, through PostMessageW -- the window
// message path `ds2-input-harness/src/click.rs` documents. Measured 2026-10-01: with the game
// window not in the foreground those clicks started no attack, so in practice a human swings.
//
// Hooks nothing (the two recorded attach freezes, 2026-09-26, were hooking agents). Every 8 ms it
// reads what `0x14035d580` reads and sends a line when any value changed:
//
//   C      = [[GameManagerImp] + 0xd0]          the local PlayerCtrl (ds2_rva PLAYER_CTRL_OFFSET)
//   flags  = [C + 0xb8]   +0x1f8 start, +0x1fc end, +0x200 start factor (f32), +0x195 (u8)
//   S      = [C + 0xc0]   +0xa0 / +0xa4, the event-150 / event-151 counters (i32)
//
// and the swing speed `0x14035d580` would set from them: S+0xa0 != 0 -> +0x200 x +0x1f8;
// else S+0xa4 != 0 -> +0x1fc (0 if +0x195); else [0x1410ac698]. That `0x14035d580`'s
// `[this+8]` is this PlayerCtrl is checked by the values themselves: +0x1f8/+0x1fc must turn into
// the swung weapon's WeaponAttackMotionParam start/end and S+0xa0 must count up in the windup.
//
// The repo's notes say Frida timers do not always fire in this game; a heartbeat every 2 s
// says whether they do in this session.

'use strict';

const config = globalThis.__ER_FRIDA_CONFIG || {};
const INTERVAL_MS = typeof config.interval_ms === 'number' ? config.interval_ms : 8;
const CLICKS = typeof config.clicks === 'number' ? config.clicks : 0;

const image = Process.getModuleByName('DarkSoulsII.exe');
const at = (rva) => image.base.add(rva);
const PLAYER_CTRL_VTABLE = at(0x010e4bb8);
const DEFAULT_SPEED = at(0x010ac698);
const KATANA_MAIN_APP_SLOT = 0x16751f8;
const t0 = Date.now();
const say = (t) => console.log('[attack-speed] t=' + (Date.now() - t0) + ' ' + t);

function ptr_(p, off) {
  try {
    const v = p.add(off).readPointer();
    return v.isNull() ? null : v;
  } catch (e) {
    return null;
  }
}

function rva(p) {
  return p === null ? 'null' : '0x' + p.sub(image.base).toString(16);
}

say('default speed [0x1410ac698] = ' + DEFAULT_SPEED.readFloat() + ', interval ' + INTERVAL_MS +
  ' ms, clicks ' + CLICKS);

// What the player carries, once: the ChrAsmCtrl record table (ds2_rva EQUIP_RECORD_*), records
// 0..5 being the weapon slots, and the grip word armor-sync/player-ctrl read. Item ids name the
// weapon; the swing's +0x1f8/+0x1fc then pin the WeaponAttackMotionParam row.
function equipped() {
  const gm = ptr_(at(0x16148f0), 0);
  const pc = gm && ptr_(gm, 0xd0);
  const asm = pc && ptr_(pc, 0x378);
  const table = asm && ptr_(asm, 0x20);
  if (!table) return 'no record table';
  const ids = [];
  for (let i = 0; i < 6; i++) {
    const r = table.add(8 + i * 0x14);
    ids.push(i + ':' + r.add(4).readU32() + '+' + r.add(0xe).readU8());
  }
  const equip = ptr_(asm, 0x28);
  return ids.join(' ') + (equip ? ' grip=' + equip.add(0x10).readS32() : '');
}
say('weapon records ' + equipped());
const getForegroundWindow = new NativeFunction(
  Process.getModuleByName('user32.dll').getExportByName('GetForegroundWindow'), 'pointer', []);

let last = '';
let samples = 0;
let errors = 0;
let shown = false;

function sample() {
  samples += 1;
  const gm = ptr_(at(0x16148f0), 0);
  const pc = gm && ptr_(gm, 0xd0);
  if (!pc) { if (last !== 'no player') { last = 'no player'; say('no player'); } return; }
  const flags = ptr_(pc, 0xb8);
  const s = ptr_(pc, 0xc0);
  if (!flags || !s) { if (last !== 'no flags') { last = 'no flags'; say('flags/S null'); } return; }
  if (!shown) {
    shown = true;
    say('player ' + pc + ' vtable ' + rva(ptr_(pc, 0)) +
      (ptr_(pc, 0) !== null && ptr_(pc, 0).equals(PLAYER_CTRL_VTABLE) ? ' (PlayerCtrl)' : ' (NOT PlayerCtrl)') +
      ' flags ' + flags + ' vt ' + rva(ptr_(flags, 0)) + ' S ' + s + ' vt ' + rva(ptr_(s, 0)));
  }
  let v;
  try {
    v = {
      start: flags.add(0x1f8).readFloat(),
      end: flags.add(0x1fc).readFloat(),
      factor: flags.add(0x200).readFloat(),
      f200hex: flags.add(0x200).readU32().toString(16),
      b195: flags.add(0x195).readU8(),
      a0: s.add(0xa0).readS32(),
      a4: s.add(0xa4).readS32(),
    };
  } catch (e) {
    errors += 1;
    return;
  }
  const swing = v.a0 !== 0 ? v.factor * v.start : v.a4 !== 0 ? (v.b195 ? 0 : v.end) : DEFAULT_SPEED.readFloat();
  const line = 'a0=' + v.a0 + ' a4=' + v.a4 + ' +0x1f8=' + v.start.toFixed(4) + ' +0x1fc=' +
    v.end.toFixed(4) + ' +0x200=' + v.factor.toFixed(4) + ' (0x' + v.f200hex + ') +0x195=' + v.b195 +
    ' -> Attack_SwingSpeed=' + swing.toFixed(4);
  if (line !== last) {
    last = line;
    say(line);
    send({ t: Date.now() - t0, a0: v.a0, a4: v.a4, start: v.start, end: v.end, factor: v.factor,
      b195: v.b195, swing });
  }
}

setInterval(sample, INTERVAL_MS);
setInterval(() => say('heartbeat samples=' + samples + ' errors=' + errors), 10000);

function gameWindow() {
  const app = ptr_(at(KATANA_MAIN_APP_SLOT), 0);
  const input = app && ptr_(app, 0x60);
  const cursor = input && ptr_(input, 8);
  const first = cursor && ptr_(cursor, 0xd0);
  const device = first && ptr_(first, 0);
  return device && ptr_(device, 0x20);
}

if (CLICKS > 0) {
  const postMessage = new NativeFunction(
    Process.getModuleByName('user32.dll').getExportByName('PostMessageW'),
    'int', ['pointer', 'uint', 'pointer', 'pointer']);
  const hwnd = gameWindow();
  if (hwnd === null) {
    say('no game window; no clicks posted');
  } else {
    let n = 0;
    const lparam = ptr((200 << 16) | 200);
    const click = () => {
      n += 1;
      postMessage(hwnd, 0x201, ptr(0x1), lparam); // WM_LBUTTONDOWN, MK_LBUTTON
      setTimeout(() => postMessage(hwnd, 0x202, ptr(0), lparam), 80); // WM_LBUTTONUP
      say('posted left-click ' + n + ' to hwnd ' + hwnd + ' foreground ' + getForegroundWindow());
      if (n < CLICKS) setTimeout(click, 1500);
    };
    setTimeout(click, 1000);
  }
}
