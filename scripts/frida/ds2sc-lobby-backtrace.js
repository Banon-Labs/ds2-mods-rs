// Who called Seamless's lobby calls? A backtrace at CreateLobby, RequestLobbyList and the bracket
// SetLobbyData, to walk back from the Steam call to the game code that used the item (Pharros'
// Heliograph hosts, Fragment of Malevolance searches).
//
// `Thread.backtrace` is FUZZY on purpose: the packed ds2sc.dll leaves frames an exact unwinder cannot
// cross (er-mods-rs lobby-key-watch.js measured the same on ersc.dll). Fuzzy frames include stale
// stack values, so a frame is a lead to read in the disassembly, not a proven caller. The thread id
// says whether the call ran on the game thread or on one of Seamless's own.
//
// Observers only, on ISteamMatchmaking v009 slots 4 (RequestLobbyList), 13 (CreateLobby),
// 20 (SetLobbyData, bracket key only). Attach as its own session; do not hot-reload.

'use strict';

const BRACKET_KEY = '8c1e0aea8218b194b8598fc57ec01ccf4c5444e3522d367f1f8ec903f1d12ae5';
const FRAMES = 48;

const steam = Process.getModuleByName('steam_api64.dll');
const iface = new NativeFunction(steam.getExportByName('SteamMatchmaking'), 'pointer', [])();
const vtable = iface.readPointer();
const mainThread = Process.enumerateThreads()[0].id;

function where(address) {
  const mod = Process.findModuleByAddress(address);
  return mod === null ? address.toString() : mod.name + '+0x' + address.sub(mod.base).toString(16);
}

function trace(context) {
  return Thread.backtrace(context, Backtracer.FUZZY).slice(0, FRAMES).map(where);
}

const slots = { 4: 'RequestLobbyList', 13: 'CreateLobby', 20: 'SetLobbyData' };
for (const [slot, name] of Object.entries(slots)) {
  Interceptor.attach(vtable.add(Number(slot) * Process.pointerSize).readPointer(), {
    onEnter(args) {
      if (name === 'SetLobbyData') {
        let key = null;
        try {
          key = args[2].readUtf8String();
        } catch (e) {
          key = null;
        }
        if (key !== BRACKET_KEY) return;
      }
      send({ tag: 'backtrace', call: name, thread: Process.getCurrentThreadId(), first_thread: mainThread, frames: trace(this.context) });
    },
  });
}
send({ tag: 'hooked', calls: Object.values(slots) });
