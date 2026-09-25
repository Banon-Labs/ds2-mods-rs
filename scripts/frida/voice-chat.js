// Where is the game's own Voice Chat option, and does flipping it through the game's setter stick?
//
// The option block is `GameManagerImp(0x1416148f0) -> +0xa8 GameDataManager -> +0xc8`, the struct
// `SaveDataOption` serialises. The Game tab's rows map to its bytes through `0x140083d60`'s table
// (ROW_TO_BYTE below), and the menu's own text, read out of memory by `labels`, puts "Voice chat"
// on row 10 -> byte 0x0b. `0x1402c9540` (the per-frame net session update) reads that byte every
// frame and re-mutes every peer through `0x1402cb140` when it changes.
//
// Modes, by --config-json:
//   {}                read the block, count 0x1402c9540 calls and the threads they run on (3 s)
//   {"labels": 1}     also scan memory for the menu's "Voice chat" text and print its neighbours
//   {"flip": 1}       inside 0x1402c9540, on the game thread, call the Game tab's apply routine
//                     0x14019d7d0 with a copy whose byte 0x0b is inverted -- the call the options
//                     menu makes on confirm -- then read the byte back and watch the net update's
//                     cached "voice active" flag (+0x90 of its object) follow it
//
//   uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/voice-chat.js \
//       --role voice-chat --log ~/.cache/er-frida/voice-chat.jsonl --config-json '{}'

'use strict';

const cfg = globalThis.__ER_FRIDA_CONFIG || {};
const base = Process.getModuleByName('DarkSoulsII.exe').base;
const IMAGE = ptr('0x140000000');
const rva = (va) => base.add(ptr(va).sub(IMAGE));

const GAME_MANAGER_IMP = rva('0x1416148f0');
const GAME_TAB_APPLY = rva('0x14019d7d0');
const NET_SESSION_UPDATE = rva('0x1402c9540');
const VOICE_CHAT_BYTE = 0x0b;
const NET_VOICE_ACTIVE_CACHE = 0x90;

function optionBlock() {
  const gm = GAME_MANAGER_IMP.readPointer();
  if (gm.isNull()) return null;
  const gdm = gm.add(0xa8).readPointer();
  if (gdm.isNull()) return null;
  const opt = gdm.add(0xc8).readPointer();
  return opt.isNull() ? null : opt;
}

function dump(tag, extra) {
  const opt = optionBlock();
  if (opt === null) {
    send(Object.assign({ tag, error: 'option block not reachable' }, extra || {}));
    return null;
  }
  const bytes = Array.from(new Uint8Array(opt.readByteArray(0x14)));
  send(Object.assign({ tag, option_block: opt.toString(), bytes, voice_chat_byte: bytes[VOICE_CHAT_BYTE] }, extra || {}));
  return opt;
}

// The Game tab's rows, in menu order, map to these option bytes (0x140083d60's table).
const ROW_TO_BYTE = [0, 1, 2, 3, 4, 5, 6, 7, 8, 10, 11, 9, 12, 13, 14];

function readStr(p) {
  try { return p.readUtf16String(); } catch (e) { return null; }
}

function labelScan() {
  const needle = 'Voice'.split('').map((c) => c.charCodeAt(0).toString(16).padStart(2, '0') + ' 00').join(' ');
  let hits = 0;
  for (const r of Process.enumerateRanges({ protection: 'rw-', coalesce: true })) {
    if (r.size > 0x4000000) continue;
    let found;
    try { found = Memory.scanSync(r.base, r.size, needle); } catch (e) { continue; }
    for (const f of found) {
      const s = readStr(f.address);
      if (s === null || !/^Voice [Cc]hat/.test(s)) continue;
      const around = [];
      let p = f.address.sub(2);
      for (let i = 0; i < 60; i += 1) {
        let q = p.sub(2);
        let n = 0;
        while (n < 200 && q.readU16() !== 0) { q = q.sub(2); n += 1; }
        around.unshift(readStr(q.add(2)));
        p = q;
      }
      around.push('>>> ' + s);
      let t = f.address;
      while (t.readU16() !== 0) t = t.add(2);
      for (let i = 0; i < 30; i += 1) {
        t = t.add(2);
        const str = readStr(t) || '';
        around.push(str);
        t = t.add(str.length * 2);
      }
      send({ tag: 'label', at: f.address.toString(), around });
      hits += 1;
      if (hits >= 12) return hits;
    }
  }
  return hits;
}

dump('read');
send({ tag: 'row-map', ROW_TO_BYTE, main_thread: Process.enumerateThreads()[0].id });
if (cfg.labels) send({ tag: 'label-hits', n: labelScan() });

const apply = new NativeFunction(GAME_TAB_APPLY, 'void', ['pointer', 'pointer']);
const copy = Memory.alloc(0x10);
const threads = {};
let calls = 0;
let flipped = false;
let after = 0;
let netObj = null;

const listener = Interceptor.attach(NET_SESSION_UPDATE, {
  onEnter(args) {
    calls += 1;
    const tid = Process.getCurrentThreadId();
    threads[tid] = (threads[tid] || 0) + 1;
    netObj = args[0];
    if (!cfg.flip || flipped) return;
    flipped = true;
    const opt = optionBlock();
    if (opt === null) { send({ tag: 'flip', error: 'option block not reachable' }); return; }
    const cacheBefore = netObj.add(NET_VOICE_ACTIVE_CACHE).readU8();
    Memory.copy(copy, opt, 0x10);
    const was = copy.add(VOICE_CHAT_BYTE).readU8();
    copy.add(VOICE_CHAT_BYTE).writeU8(was ? 0 : 1);
    apply(opt, copy);
    const now = opt.add(VOICE_CHAT_BYTE).readU8();
    send({ tag: 'flip', thread: tid, was, now, ok: now === (was ? 0 : 1), net_voice_active_cache_before: cacheBefore });
  },
  onLeave() {
    if (flipped && after < 1 && netObj !== null) {
      after += 1;
      // The original has now run once with the new byte: its cached flag should read
      // (byte == 0), i.e. it noticed and re-applied mute/unmute to the session's peers.
      send({ tag: 'after-original', net_voice_active_cache: netObj.add(NET_VOICE_ACTIVE_CACHE).readU8() });
    }
  },
});

setTimeout(() => {
  send({ tag: 'net-update-census', seconds: 3, calls, threads });
  dump('final');
}, 3000);

rpc.exports.dispose = function () {
  listener.detach();
};
