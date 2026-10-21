// Start one Seamless invasion search that finds nobody, to watch what the Invadable hosts census
// does while Seamless stages and sends its own query.
//
// Uses the Fragment of Malevolance through the belt's use request (as use-item.js) and adds one
// filter no lobby carries to the RequestLobbyList whose stack passes through ds2sc.dll, so Steam
// returns no world and nobody is invaded (the owner's rule, 2026-10-21). The census's own requests
// (no ds2sc.dll frame) are left alone. Re-issues the use every 4 s until a search is seen, at most
// 30 s, then reports and stops voiding.

'use strict';

const FRAGMENT_OF_MALEVOLANCE = 100000002;
const OBSCURING_STONE = 100000003;
const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const rp = (p) => p.readPointer();
const steam = Process.getModuleByName('steam_api64.dll');
const iface = new NativeFunction(steam.getExportByName('SteamMatchmaking'), 'pointer', [])();
// The census points the interface at a wrapped copy; the original entry is what both reach.
const table = iface.readPointer();
const sc = Process.getModuleByName('ds2sc.dll');
const addString = new NativeFunction(table.add(5 * 8).readPointer(), 'void', ['pointer', 'pointer', 'pointer', 'int']);
const voidKey = Memory.allocUtf8String('ds2_mods_void_one_search_no_lobby_has_this');
const voidValue = Memory.allocUtf8String('1');
let voided = 0;
let others = 0;
let stopped = false;

function fromSeamless(context) {
  return Thread.backtrace(context, Backtracer.FUZZY).some((a) => a.compare(sc.base) >= 0 && a.compare(sc.base.add(sc.size)) < 0);
}

// Hook the request the wrapped table forwards to: the census's wrapper calls the same function.
const requestTarget = (() => {
  // The census's copy replaced slot 4 with its wrapper; the wrapper calls steamclient's original,
  // which is what every request reaches last. Find it from the module that owns the copy's other
  // slots (slot 0 is never wrapped).
  const anyOriginal = table.readPointer();
  const owner = Process.findModuleByAddress(anyOriginal);
  send({ tag: 'table', slot0_owner: owner === null ? null : owner.name });
  return table.add(4 * 8).readPointer();
})();
send({ tag: 'request-slot', target: requestTarget.toString(), owner: (Process.findModuleByAddress(requestTarget) || { name: '?' }).name });

Interceptor.attach(requestTarget, {
  onEnter(args) {
    if (stopped) return;
    // Only Seamless reaches the wrapper; the census calls the original entry. Count the stack
    // check anyway, to prove it.
    addString(args[0], voidKey, voidValue, 0);
    voided += 1;
    if (!fromSeamless(this.context)) others += 1;
  },
});

// The safety net: a join while this runs means a search was not voided; leave at once.
Interceptor.attach(table.add(14 * 8).readPointer(), {
  onEnter() {
    send({ tag: 'JOIN-SEEN' });
    setTimeout(() => use(OBSCURING_STONE), 0);
  },
});

function use(goods) {
  const gm = rp(exe.add(0x016148f0));
  let bag = rp(gm.add(0xa8));
  bag = rp(rp(rp(bag.add(0x10)).add(0x10)).add(0x10));
  let handle = null;
  for (let i = 0; i < 3840; i++) {
    const e = bag.add(0x28 + i * 0x28);
    if (e.add(0x14).readU32() === goods) {
      handle = e.add(0x1c).readU16();
      break;
    }
  }
  const req = rp(rp(gm.add(0xd0)).add(0xc8));
  req.add(0x40).writeU16(handle);
  req.add(0x42).writeU16(1);
  req.add(0xfc).writeU32(req.add(0xfc).readU32() | 0x800);
  return handle;
}

// A host cannot search: close the session with the Obscuring Stone first, then search.
send({ tag: 'stone', handle: use(OBSCURING_STONE) });
let started = 0;
setTimeout(() => {
  started = Date.now();
  send({ tag: 'use', handle: use(FRAGMENT_OF_MALEVOLANCE) });
}, 6000);
const timer = setInterval(() => {
  if (started === 0) return;
  const ms = Date.now() - started;
  if (voided > 0 || ms > 30000) {
    clearInterval(timer);
    setTimeout(() => {
      stopped = true;
      send({ tag: 'done', voided, requests_without_a_ds2sc_frame: others, ms });
    }, 8000);
  } else if (ms % 4000 < 250) {
    use(FRAGMENT_OF_MALEVOLANCE);
  }
}, 250);
