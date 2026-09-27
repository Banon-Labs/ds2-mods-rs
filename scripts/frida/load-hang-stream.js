// Which save stream is FUN_1402e47f0's header walk stuck on, and at what offset?
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/load-hang-stream.js
//  --config-json '{"tid": 332}'`
//
// Samples the game thread's context (no hooks, no writes). When the pc is inside the walk's own
// body (0x1402e4856..0x1402e4903) RSP is that function's frame, so the 0x20-byte header it last
// read sits at RSP+0x20 and RDI is the DLMemoryInputStream (+0x8 size, +0x10 data, +0x18 cursor,
// +0x24 set-up flag). Prints the stream, the header, the bytes around the cursor, the caller's
// frame and the 15-entry type table at 0x1410da1f0. Read-only.

'use strict';

const cfg = (typeof globalThis.__ER_FRIDA_CONFIG === 'object' && globalThis.__ER_FRIDA_CONFIG) || {};
const TID = cfg.tid || 332;
const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const lo = exe.add(0x2e4856);
const hi = exe.add(0x2e4904);
const TABLE = exe.add(0x10da1f0);

function hex(p, n) {
  try {
    return Array.from(new Uint8Array(p.readByteArray(n)))
      .map((b) => b.toString(16).padStart(2, '0')).join('');
  } catch (e) { return 'unreadable(' + e.message + ')'; }
}

const table = [];
for (let i = 0; i < 15; i++) table.push(TABLE.add(i * 4).readU32());
console.log('[load-hang] table=' + JSON.stringify(table));

// FUN_1402e8340: [[[GameManagerImp]+0xa8]+0xd8]+0x1368 is the slot the load setup reads.
function slotId() {
  try {
    const gm = exe.add(0x16148f0).readPointer();
    const gdm = gm.add(0xa8).readPointer();
    const sd = gdm.add(0xd8).readPointer();
    return sd.add(0x1368).readS32();
  } catch (e) { return 'unreadable(' + e.message + ')'; }
}

function nonzero(data, size) {
  try {
    const bytes = new Uint8Array(data.readByteArray(size));
    let n = 0; let first = -1;
    for (let i = 0; i < bytes.length; i++) if (bytes[i]) { n++; if (first < 0) first = i; }
    return 'nonzero=' + n + ' first-nonzero=' + first;
  } catch (e) { return 'unreadable(' + e.message + ')'; }
}

console.log('[load-hang] slot=' + slotId());

let samples = 0;
let hits = 0;
function sample() {
  samples += 1;
  const t = Process.enumerateThreads().find((x) => x.id === TID);
  if (!t) { console.log('[load-hang] tid ' + TID + ' not found'); return; }
  const c = t.context;
  const pc = c.pc;
  const inBody = pc.compare(lo) >= 0 && pc.compare(hi) < 0;
  console.log('[load-hang] sample ' + samples + ' pc=exe+0x' + pc.sub(exe).toString(16) +
    ' rsp=' + c.rsp + ' rdi=' + c.rdi + ' rsi=' + c.rsi + ' state=' + t.state);
  if (inBody && hits < 1) {
    hits += 1;
    const s = c.rdi;
    const size = s.add(8).readU64();
    const data = s.add(0x10).readPointer();
    const cur = s.add(0x18).readU64();
    const flag = s.add(0x24).readU8();
    console.log('[load-hang] stream=' + s + ' vt=exe+0x' + s.readPointer().sub(exe).toString(16) +
      ' size=' + size + ' data=' + data + ' cursor=' + cur + ' flag=' + flag);
    console.log('[load-hang] header@rsp+20=' + hex(c.rsp.add(0x20), 0x20));
    console.log('[load-hang] this-stream ' + nonzero(data, size.toNumber()));
    // The caller's two DLMemoryInputStreams (slot+8 then slot+0x12) sit 0x30 apart.
    for (const off of [0x68, 0x98]) {
      const other = c.rsp.add(0x70 + off);
      const osize = other.add(8).readU64().toNumber();
      const odata = other.add(0x10).readPointer();
      console.log('[load-hang] caller-stream@callerRsp+0x' + off.toString(16) + '=' + other +
        ' size=' + osize + ' data=' + odata + ' ' + nonzero(odata, osize));
    }
    console.log('[load-hang] data[0..0x40]=' + hex(data, 0x40));
    const back = Math.min(cur.toNumber(), 0x40);
    console.log('[load-hang] data[cursor-' + back + '..+0x40]=' +
      hex(data.add(cur.toNumber() - back), back + 0x40));
    console.log('[load-hang] handlers *rsi=' + c.rsi.readPointer() + ' -> ' +
      hex(c.rsi.readPointer(), 15 * 8));
    // caller frame: 3 pushes + 0x50 + return address
    const callerRsp = c.rsp.add(0x70);
    console.log('[load-hang] ret=exe+0x' + c.rsp.add(0x68).readPointer().sub(exe).toString(16) +
      ' callerRsp=' + callerRsp + ' caller[0..0x100]=' + hex(callerRsp, 0x100));
  }
  if (samples < 30 && hits < 1) setTimeout(sample, 30);
  else console.log('[load-hang] done samples=' + samples + ' inBody=' + hits);
}
sample();
