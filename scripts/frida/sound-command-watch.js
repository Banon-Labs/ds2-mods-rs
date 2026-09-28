// Who queues the sound command that sets the "music" category to 0?
//
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/sound-command-watch.js`
//
// MOFmodSoundManager's command drain (0x1409e0910) pops 0x10-byte-plus commands off the queue at
// manager+0xd0 and, for the category-volume case, calls EventCategory::setVolume with the float at
// cmd+0xc on the category named by the u16 id at cmd+0x8 (0x1409e0b3c..0x1409e0b5f). The drain
// switches on cmd+0x6 (type) and cmd+0x7 (subtype). fmod-category-watch.js caught that drain setting
// "music" to 0 after an autoload, and nothing setting it back until the pause menu opened.
//
// 0x1409ebea0 has 71 callers, all in the manager's API wrappers at 0x1409d5xxx: the push. This
// logs every push whose u16 at +0x8 is 1 (the music category's id, read from the region cue's
// [cue+0x238]) or whose float at +0xc is 0, with the command bytes and a backtrace. Hooks a game
// function's entry with Interceptor; it changes nothing it reads.

'use strict';

const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const PUSH = exe.add(0x9ebea0);
function where(addr) {
  const m = Process.findModuleByAddress(addr);
  return m ? m.name + '+0x' + addr.sub(m.base).toString(16) : addr.toString();
}
function hex(p, n) {
  try { return Array.from(new Uint8Array(p.readByteArray(n))).map((b) => b.toString(16).padStart(2, '0')).join(''); } catch (e) { return null; }
}
let seen = 0;
Interceptor.attach(PUSH, { onEnter(args) {
  // Which argument carries the command is not established; report whichever looks like one.
  for (const [i, p] of [[1, args[1]], [2, args[2]]]) {
    let id, value, type, sub;
    try { type = p.add(6).readU8(); sub = p.add(7).readU8(); id = p.add(8).readU16(); value = p.add(0xc).readFloat(); } catch (e) { continue; }
    if (type !== 1 || sub !== 6) continue;
    // type 1 subtype 6 is the category-volume case (jump table at 0x1409e13d4 -> 0x1409e0a99).
    seen += 1;
    if (seen > 200) return;
    send({ kind: 'push', arg: i, type, sub, id, value, bytes: hex(p, 0x18), rcx: args[0].toString(), r8: args[2].toString(), caller: where(this.returnAddress), stack: Thread.backtrace(this.context, Backtracer.FUZZY).slice(0, 8).map(where), t: Date.now(), tid: Process.getCurrentThreadId() });
  }
} });
// 0x14019d8e0 builds those pushes from an options block in rcx: music = byte +0xc / 10.0 (the
// constant at 0x1410acb18), then +0xd, +0xe. Dump the block and who handed it over.
Interceptor.attach(exe.add(0x19d8e0), { onEnter(args) {
  send({ kind: 'options-apply', block: args[0].toString(), bytes: hex(args[0], 0x20), music_byte: (() => { try { return args[0].add(0xc).readU8(); } catch (e) { return null; } })(), stack: Thread.backtrace(this.context, Backtracer.FUZZY).slice(0, 8).map(where), t: Date.now() });
} });
send({ kind: 'ready', t: Date.now() });
