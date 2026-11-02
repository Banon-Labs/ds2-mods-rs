// Who creates summon signs, from where, and what happens when the Red Sign Soapstone is used
// offline (bd: NPC duel signs feature, memory ds2-npc-summon-sign-api-static-2026-11-02).
//
// Mostly observe: the sign hooks log and return. Three things write: a one-shot ItemGive of the
// soapstone, and two return-value overrides (offline state check, net requirement 2) that enable
// Use offline for the Red Sign Soapstone only.
//
// Static, from darksoulsii-deobf.bin (all CLEAN of Arxan redirects, Ds2ArxanStubs 2026-11-02):
//   ISummonSignSetCtrl vtable 0x1410cb6e8, reached by FUN_140210ea0(SignManager) =
//   [[SignManager+0x68]+0x20]+0x28, SignManager = [GameManagerImp+0x90].
//     slot 1 FUN_140213080(iface, &out, u8 chrType, pos*, ...)  the player's OWN sign (kind 1)
//     slot 2 FUN_140213160(iface, &out, u8 chrType, pos*, ...)  a NETWORK player's sign (kind 2)
//     slot 3 FUN_140213260(iface, &out, &entityId)               an NPC's sign (kind 3)
//   0x140412aa0(CharacterCtrl*)  SetSummonSign: ESD event command 0x2029d calls it; it calls slot 3
//                                and stores the handle at chr+0x84
//   FUN_1402d7480(u32 code)      shows "Failed to create summon sign." and kin, chosen by code
// The open question this answers: offline, does using the soapstone reach slot 1 at all?
'use strict';

const image = Process.getModuleByName('DarkSoulsII.exe');
const at = (rva) => image.base.add(rva);

const HOOKS = [
  { name: 'own_sign', rva: 0x00213080, prologue: [0x48, 0x89, 0x5c, 0x24, 0x08] },
  { name: 'net_sign', rva: 0x00213160, prologue: [0x48, 0x89, 0x5c, 0x24, 0x08] },
  { name: 'npc_sign', rva: 0x00213260, prologue: [0x40, 0x53, 0x55, 0x56, 0x57] },
  { name: 'set_summon_sign', rva: 0x00412aa0, prologue: [0x40, 0x57, 0x48, 0x83, 0xec] },
  { name: 'sign_fail_text', rva: 0x002d7480, prologue: [0xff, 0xc9, 0x83, 0xf9, 0x45] },
];

function rel(p) {
  const off = p.sub(image.base);
  return off.compare(ptr(image.size)) < 0 ? 'exe+0x' + off.toString(16) : p.toString();
}

function readPos(p) {
  try {
    return [p.readFloat(), p.add(4).readFloat(), p.add(8).readFloat()].map((v) => v.toFixed(2));
  } catch (e) {
    return 'unreadable';
  }
}

function details(name, args) {
  switch (name) {
    case 'own_sign':
    case 'net_sign':
      return { chrType: args[2].toInt32() & 0xff, pos: readPos(args[3]) };
    case 'npc_sign':
      return { entityId: '0x' + (args[2].readU32() >>> 0).toString(16) };
    case 'set_summon_sign':
      return { chr: args[0].toString(), signHandle: '0x' + (args[0].add(0x84).readU32() >>> 0).toString(16) };
    case 'sign_fail_text':
      return { code: args[0].toInt32() };
  }
  return {};
}

const installed = [];
for (const h of HOOKS) {
  const target = at(h.rva);
  const found = Array.from(new Uint8Array(target.readByteArray(h.prologue.length)));
  if (found.join(',') !== h.prologue.join(',')) {
    send({ family: 'summon-sign', tag: 'refused', hook: h.name, found: found.map((b) => b.toString(16)) });
    continue;
  }
  Interceptor.attach(target, {
    onEnter(args) {
      let info;
      try {
        info = details(h.name, args);
      } catch (e) {
        info = { error: String(e) };
      }
      const bt = Thread.backtrace(this.context, Backtracer.ACCURATE).slice(0, 8).map(rel);
      send({ family: 'summon-sign', tag: 'call', hook: h.name, info, backtrace: bt });
    },
    onLeave(ret) {
      if (h.name === 'own_sign' || h.name === 'net_sign' || h.name === 'npc_sign') {
        let handle = 'unreadable';
        try {
          handle = '0x' + (ret.readU32() >>> 0).toString(16);
        } catch (e) {}
        send({ family: 'summon-sign', tag: 'return', hook: h.name, handle });
      }
    },
  });
  installed.push(h.name);
}
send({ family: 'summon-sign', tag: 'hooked', installed });

// One Red Sign Soapstone, so the offline use can be tried. The game's own ItemGive (ds2-rva
// ITEM_GIVE), spawn mode 0. Measured: a held, non-stacking soapstone is refused with 0x81010000
// (already held), so a re-run grants nothing twice. Called once, on the game thread, from
// NvNavigationSystem::Update.
const GAME_MANAGER_IMP = at(0x016148f0);
const GAME_DATA_MANAGER_OFFSET = 0xa8;
const ITEM_INVENTORY_OFFSET = 0x10;
const PLAYER_CTRL_OFFSET = 0xd0;
const ITEM_GIVE = at(0x001ac3d0);
const ITEM_GIVE_PROLOGUE = [0x48, 0x8b, 0x49, 0x10, 0x45];
const NAV_UPDATE = at(0x00baeb20);
const RED_SIGN_SOAPSTONE = 62045000;

const give = new NativeFunction(ITEM_GIVE, 'bool', ['pointer', 'pointer', 'uint32']);
globalThis.__spawn = Memory.alloc(0x10);
let granted = false;
const nav = Interceptor.attach(NAV_UPDATE, {
  onEnter() {
    if (granted) return;
    granted = true;
    const gm = GAME_MANAGER_IMP.readPointer();
    const player = gm.add(PLAYER_CTRL_OFFSET).readPointer();
    const inventory = gm.add(GAME_DATA_MANAGER_OFFSET).readPointer().add(ITEM_INVENTORY_OFFSET).readPointer();
    const prologue = Array.from(new Uint8Array(ITEM_GIVE.readByteArray(5)));
    if (player.isNull() || inventory.isNull() || prologue.join(',') !== ITEM_GIVE_PROLOGUE.join(',')) {
      send({ family: 'summon-sign', tag: 'give.refused', player: player.toString(), inventory: inventory.toString(), prologue });
      return;
    }
    const s = globalThis.__spawn;
    s.writeU32(0); // mode 0: grant; mode 1 returned false, which does not prove the item is held
    s.add(0x04).writeS32(RED_SIGN_SOAPSTONE);
    s.add(0x08).writeS32(-1);
    s.add(0x0c).writeU16(1);
    s.add(0x0e).writeU8(0);
    s.add(0x0f).writeU8(0);
    const ok = give(inventory, s, 1);
    // ds2-rva ITEM_GIVE_ERROR_OFFSET: inner+0x10138, bit 31 = refused, the rest says why.
    const inner = inventory.add(0x10).readPointer();
    const error = inner.add(0x10138).readU32() >>> 0;
    const detail = inner.add(0x1013c).readU32() >>> 0;
    send({ family: 'summon-sign', tag: 'give', item: RED_SIGN_SOAPSTONE, granted: ok, error: '0x' + error.toString(16), detail, player: player.toString() });
  },
});
globalThis.__nav = nav;

// Every read of the soapstone's own param rows, so the trace starts where the USE starts -- the menu,
// the item-use request and any offline check -- not only at sign creation. paramLookup
// (0x14003e8f0, CLEAN, `rcx` = param memory, `edx` = row id) is called hundreds of times a frame,
// so the row filter runs in C and only a match reaches JS, with the caller's return address.
const PARAM_LOOKUP = at(0x0003e8f0);
const PARAM_LOOKUP_PROLOGUE = [0x48, 0x89, 0x5c, 0x24, 0x08];
const WATCHED_ROWS = { 62045000: 'ItemParam Red Sign Soapstone', 1110: 'ItemUsageParam 1110' };
const onRow = new NativeCallback(
  (row, retaddr) => {
    send({ family: 'summon-sign', tag: 'row', row: WATCHED_ROWS[row] || row, caller: rel(retaddr) });
  },
  'void',
  ['int', 'pointer'],
);
globalThis.__onRow = onRow;
const cm = new CModule(
  `
#include <gum/guminterceptor.h>
extern void on_row (int row, void * retaddr);
void on_enter (GumInvocationContext * ic) {
  int row = (int) (gsize) gum_invocation_context_get_nth_argument (ic, 1);
  if (row == 62045000 || row == 1110)
    on_row (row, gum_invocation_context_get_return_address (ic));
}
`,
  { on_row: onRow },
);
globalThis.__cm = cm;
// paramLookup alone misses a repeat: FUN_1401bb0f0 (lookupItem, `rcx` = helper, `edx` = item id)
// caches the last item and returns before any lookup. So the item id is also filtered at its entry.
const LOOKUP_ITEM = at(0x001bb0f0);
const LOOKUP_ITEM_PROLOGUE = [0x48, 0x89, 0x5c, 0x24, 0x08];
const onItem = new NativeCallback(
  (item, retaddr) => {
    send({ family: 'summon-sign', tag: 'item', item, caller: rel(retaddr) });
  },
  'void',
  ['int', 'pointer'],
);
globalThis.__onItem = onItem;
const cmItem = new CModule(
  `
#include <gum/guminterceptor.h>
extern void on_item (int item, void * retaddr);
void on_enter (GumInvocationContext * ic) {
  int item = (int) (gsize) gum_invocation_context_get_nth_argument (ic, 1);
  if (item == 62045000)
    on_item (item, gum_invocation_context_get_return_address (ic));
}
`,
  { on_item: onItem },
);
globalThis.__cmItem = cmItem;
const itemPrologue = Array.from(new Uint8Array(LOOKUP_ITEM.readByteArray(5)));
if (itemPrologue.join(',') === LOOKUP_ITEM_PROLOGUE.join(',')) {
  globalThis.__lookupItem = Interceptor.attach(LOOKUP_ITEM, { onEnter: cmItem.on_enter });
  send({ family: 'summon-sign', tag: 'item.hooked', item: 62045000 });
} else {
  send({ family: 'summon-sign', tag: 'item.refused', found: itemPrologue });
}
// THE OFFLINE GATE (static 2026-11-02). The inventory's Use row is enabled by can-use 0x1401abaf0
// (thunk -> 0x1401a64e0). For an item in the table at 0x1410c3d70 -- {White 0, Small White 1,
// RED SIGN 2, Dragon Eye 3, Blue orb 4, Red orb 5} -- it also needs FUN_14024f350(net, req) to be
// true, and that asks NetSvrManager whether the sign service is available: offline, it is not.
// Requirement 2 is the Red Sign Soapstone's alone, so forcing it touches nothing else. The native
// answer is logged before it is replaced.
const CAN_USE_ITEM = at(0x001abaf0);
const CAN_USE_ITEM_PROLOGUE = [0x48, 0x8b, 0x49, 0x10, 0xe9];
const NET_REQUIREMENT = at(0x0024f350);
const NET_REQUIREMENT_PROLOGUE = [0x40, 0x53, 0x48, 0x83, 0xec];
const RED_SIGN_REQUIREMENT = 2;
const FORCE_RED_SIGN_REQUIREMENT = true;
function prologueMatches(p, want) {
  return Array.from(new Uint8Array(p.readByteArray(want.length))).join(',') === want.join(',');
}
if (prologueMatches(NET_REQUIREMENT, NET_REQUIREMENT_PROLOGUE)) {
  globalThis.__netReq = Interceptor.attach(NET_REQUIREMENT, {
    onEnter(args) {
      this.req = args[1].toInt32();
    },
    onLeave(ret) {
      const native = ret.toInt32() & 0xff;
      const forced = FORCE_RED_SIGN_REQUIREMENT && this.req === RED_SIGN_REQUIREMENT && native === 0;
      if (forced) ret.replace(ptr(1));
      const key = this.req + ':' + native;
      if (globalThis.__lastReq !== key) {
        globalThis.__lastReq = key;
        send({ family: 'summon-sign', tag: 'net.requirement', req: this.req, native, forced });
      }
    },
  });
  send({ family: 'summon-sign', tag: 'net.requirement.hooked', force: FORCE_RED_SIGN_REQUIREMENT });
} else {
  send({ family: 'summon-sign', tag: 'net.requirement.refused' });
}
// The step that actually refuses it offline (measured: FUN_14024f350 was never reached). can-use
// pushes net state 0 when the online byte (netService+0x3a) is 0, and runs that state's checker
// from 0x14156afc0. State 0's, 0x1401a8e20(bag, itemId), returns ItemUsageParam byte +5 bit 6 --
// "usable offline". Row 1110 (Red Sign Soapstone) has byte +5 = 0x08, so it says no.
const OFFLINE_STATE_CHECK = at(0x001a8e20);
const OFFLINE_STATE_CHECK_PROLOGUE = [0x40, 0x53, 0x48, 0x83, 0xec];
if (prologueMatches(OFFLINE_STATE_CHECK, OFFLINE_STATE_CHECK_PROLOGUE)) {
  globalThis.__offlineCheck = Interceptor.attach(OFFLINE_STATE_CHECK, {
    onEnter(args) {
      this.item = args[1].toInt32();
      if (globalThis.__probeHandle) globalThis.__probeItem = this.item;
    },
    onLeave(ret) {
      if (this.item !== RED_SIGN_SOAPSTONE) return;
      const native = ret.toInt32() & 0xff;
      if (native === 0) ret.replace(ptr(1));
      if (globalThis.__lastOffline !== native) {
        globalThis.__lastOffline = native;
        send({ family: 'summon-sign', tag: 'offline_state_check', item: this.item, native, forced: native === 0 });
      }
    },
  });
  send({ family: 'summon-sign', tag: 'offline_state_check.hooked' });
} else {
  send({ family: 'summon-sign', tag: 'offline_state_check.refused' });
}
if (prologueMatches(CAN_USE_ITEM, CAN_USE_ITEM_PROLOGUE)) {
  globalThis.__canUse = Interceptor.attach(CAN_USE_ITEM, {
    onEnter(args) {
      this.handle = args[1].toInt32() & 0xffff;
    },
    onLeave(ret) {
      const key = this.handle + ':' + (ret.toInt32() & 0xff);
      if (globalThis.__lastCanUse !== key) {
        globalThis.__lastCanUse = key;
        send({ family: 'summon-sign', tag: 'can_use', handle: '0x' + this.handle.toString(16), result: ret.toInt32() & 0xff });
      }
    },
  });
  send({ family: 'summon-sign', tag: 'can_use.hooked' });
} else {
  send({ family: 'summon-sign', tag: 'can_use.refused' });
}

// USE IT from here, once per load, the way the inventory's Use does. Static chain: the Use command's
// job 0x140074a30 -> 0x140073d20 -> 0x140049630, which queues 0x140500c40(this, u16 handle,
// u16 count) with this = [GameManagerImp+0x22e0]. The handle is the bag slot can_use is asked
// about (measured 0x52d this session); before using it, can_use(bag, handle) is called with the
// offline-check hook recording which item id it resolves to, and the use is refused unless that is
// the Red Sign Soapstone.
const USE_ITEM = at(0x00500c40);
const USE_ITEM_PROLOGUE = [0x4c, 0x8b, 0xdc, 0x57, 0x41, 0x56];
const ITEM_USE_OWNER_OFFSET = 0x22e0;
const SOAPSTONE_HANDLE = 0x52d;
const canUse = new NativeFunction(CAN_USE_ITEM, 'uint8', ['pointer', 'uint32']);
const useItem = new NativeFunction(USE_ITEM, 'void', ['pointer', 'uint32', 'uint32']);
let used = false;
// Which exit 0x140500c40 took. Every exit through 0xe7a clears the busy bit (+0x3b8 bit 24); the
// others keep it. 0xe61/0xe46 park the request on the PlayerCtrl (+0x40 handle, +0x42 count,
// +0xfc |= 0x800); 0xe2e dispatches through [[rsp+0xa8]]+0x68; 0xddc found no matching slot.
const USE_EXITS = { pending_e61: 0x00500e61, refused_e7a: 0x00500e7a, dispatch_e2e: 0x00500e2e, pending_e46: 0x00500e46, no_slot_ddc: 0x00500ddc };
for (const [name, rva] of Object.entries(USE_EXITS)) {
  Interceptor.attach(at(rva), function () {
    send({ family: 'summon-sign', tag: 'use.path', at: name, rsi: this.context.rsi.toString() });
  });
}
// Not from a listener: Frida does not intercept calls made inside one (memory
// frida-listener-calls-are-not-intercepted-2026-10-03), which would skip both overrides and every
// trace hook. NAV_UPDATE is (this in rcx, float dt in xmm1), so it is wrapped with replace, and
// the use runs after the original returns.
const navOriginal = new NativeFunction(NAV_UPDATE, 'void', ['pointer', 'float']);
// The soapstone's ItemParam specialEffectId is 62045000, and SpEffectActiveItem.emevd event
// 62045000 is one instruction, 100180[0] [2] (White Sign Soapstone 62030000 is 100180[0] [0]). The
// SpEffect instruction table at 0x1415712e0 maps 100180[0] to 0x14022b960: it asks 0x14022d7e0,
// then adds an effect entry (0x14022bb60) and stores the argument -- the sign type -- at
// entry+0x50. Hook both, so a use shows whether the effect runs offline at all.
const SIGN_EFFECT = at(0x0022b960);
const SIGN_EFFECT_GATE = at(0x0022d7e0);
const SIGN_EFFECT_GATE_RET = at(0x0022b994);
Interceptor.attach(SIGN_EFFECT, function () {
  send({ family: 'summon-sign', tag: 'use.effect', ctx: this.context.rcx.toString(),
    backtrace: Thread.backtrace(this.context, Backtracer.ACCURATE).slice(0, 8).map(rel) });
});
Interceptor.attach(SIGN_EFFECT_GATE, {
  onLeave(ret) {
    if (this.returnAddress.equals(SIGN_EFFECT_GATE_RET)) send({ family: 'summon-sign', tag: 'use.effect_gate', result: ret.toInt32() & 0xff });
  },
});
// false = only watch, so a use the player makes by hand is the positive control.
const AUTO_USE = false;
function useSoapstone() {
    if (!AUTO_USE || used || !granted) return;
    used = true;
    const gm = GAME_MANAGER_IMP.readPointer();
    const owner = gm.add(ITEM_USE_OWNER_OFFSET).readPointer();
    const bag = gm.add(GAME_DATA_MANAGER_OFFSET).readPointer().add(ITEM_INVENTORY_OFFSET).readPointer();
    if (owner.isNull() || bag.isNull() || !prologueMatches(USE_ITEM, USE_ITEM_PROLOGUE)) {
      send({ family: 'summon-sign', tag: 'use.refused', why: 'owner/bag/prologue', owner: owner.toString(), bag: bag.toString() });
      return;
    }
    globalThis.__probeItem = 0;
    globalThis.__probeHandle = SOAPSTONE_HANDLE;
    const usable = canUse(bag, SOAPSTONE_HANDLE);
    globalThis.__probeHandle = 0;
    const item = globalThis.__probeItem;
    if (item !== RED_SIGN_SOAPSTONE || !usable) {
      send({ family: 'summon-sign', tag: 'use.refused', why: 'handle check', handle: '0x' + SOAPSTONE_HANDLE.toString(16), item, usable });
      return;
    }
    const busy = (owner.add(0x3b8).readU32() >>> 24) & 1;
    send({ family: 'summon-sign', tag: 'use.call', handle: '0x' + SOAPSTONE_HANDLE.toString(16), owner: owner.toString(), busy });
    useItem(owner, SOAPSTONE_HANDLE, 1);
    send({ family: 'summon-sign', tag: 'use.returned', busy: (owner.add(0x3b8).readU32() >>> 24) & 1 });
}
// THE ITEM-IN-USE SEMAPHORE, the one the inventory's own WaitUsingItemJob (0x1400495d0) polls:
// 0x1401ac210 -> 0x1401a7050 -> 0x1401b2e70 = `word [inner+0x259e6] > -1`, inner =
// [[bag+0x10]+0x10], bag = [[GameManagerImp+0xa8]+0x10]. It holds the handle of the item being
// used, -1 when none. Beside it, the request 0x140500c40 parks on the action ctrl
// ([PlayerCtrl+0xc8]): +0x40 handle, +0x42 count, +0xfc bit 0x800. A handle is resolved to an item
// id the way can_use does (0x1401a64e0): 0x1401b87b0 picks the container ([X+0x10] or [X+0x18],
// X = [bag+0x10]), its vtable slot 1 returns the entry, the item id is at entry+0x14.
const PLAYER_ACTION_CTRL_OFFSET = 0xc8;
const ITEM_IN_USE_OFFSET = 0x259e6;
const handleKind = new NativeFunction(at(0x001b87b0), 'int', ['uint32']);
function itemOfHandle(x, handle) {
  if (handle === 0xffff) return -1;
  const kind = handleKind(handle);
  if (kind !== 0 && kind !== 1) return 'kind ' + kind;
  const container = x.add(kind === 0 ? 0x10 : 0x18).readPointer();
  const get = new NativeFunction(container.readPointer().add(8).readPointer(), 'pointer', ['pointer', 'uint32']);
  const entry = get(container, handle);
  return entry.isNull() ? 'no entry' : entry.add(0x14).readS32();
}
let lastSemaphore = '';
function watchItemUse() {
  const gm = GAME_MANAGER_IMP.readPointer();
  const data = gm.add(GAME_DATA_MANAGER_OFFSET).readPointer();
  const player = gm.add(PLAYER_CTRL_OFFSET).readPointer();
  if (data.isNull() || player.isNull()) return;
  const x = data.add(ITEM_INVENTORY_OFFSET).readPointer().add(0x10).readPointer();
  const inUse = x.add(0x10).readPointer().add(ITEM_IN_USE_OFFSET).readU16();
  const action = player.add(PLAYER_ACTION_CTRL_OFFSET).readPointer();
  const pending = action.isNull() ? 0 : (action.add(0xfc).readU32() & 0x800) >>> 11;
  const pendingHandle = action.isNull() ? 0xffff : action.add(0x40).readU16();
  const key = inUse + ':' + pending + ':' + pendingHandle;
  if (key === lastSemaphore) return;
  lastSemaphore = key;
  send({
    family: 'summon-sign', tag: 'item_use.semaphore',
    in_use_handle: '0x' + inUse.toString(16), in_use_item: itemOfHandle(x, inUse),
    pending, pending_handle: '0x' + pendingHandle.toString(16), pending_item: itemOfHandle(x, pendingHandle),
  });
}
globalThis.__useNavCb = new NativeCallback((self, dt) => {
  navOriginal(self, dt);
  try {
    watchItemUse();
  } catch (e) {
    send({ family: 'summon-sign', tag: 'item_use.semaphore.error', error: e.message });
  }
  try {
    useSoapstone();
  } catch (e) {
    send({ family: 'summon-sign', tag: 'use.refused', why: 'threw ' + e.message });
  }
}, 'void', ['pointer', 'float']);
Interceptor.replace(NAV_UPDATE, globalThis.__useNavCb);

// BROAD: with the gate forced, Use animates but none of the five sign hooks fired. So hook every
// sign-side entry that could be the placement: the SummonSignSetCtrl getter wrapper 0x1402128d0
// (50 callers), both SummonSignSetCtrl vtables (0x1410cb698, 0x1410cb6e8), the item-use action
// setup FUN_1401fc3e0, and the NetSvrSummonSign* functions. Each is reported once with a
// backtrace, then counted; counts go out every two seconds when they change.
const BROAD = {
  sign_ctrl_getter: 0x002128d0,
  item_use_setup: 0x001fc3e0,
  net_sign_iface_dec0: 0x0029dec0,
  net_sign_iface_createSignObject: 0x0029dfa0,
  net_sign_iface_e230: 0x0029e230,
  net_sign_iface_e590: 0x0029e590,
  net_sign_iface_e650: 0x0029e650,
  net_sign_iface_e7c0: 0x0029e7c0,
  net_sign_mgr_ffc0: 0x0029ffc0,
  net_sign_mgr_0330: 0x002a0330,
  net_sign_mgr_0470: 0x002a0470,
  net_sign_mgr_0800: 0x002a0800,
  net_sign_mgr_0e70: 0x002a0e70,
  net_sign_mgr_0f10: 0x002a0f10,
  net_sign_mgr_0ff0: 0x002a0ff0,
  net_sign_job_5a70: 0x002a5a70,
  net_sign_job_5b40: 0x002a5b40,
  net_sign_job_5ce0: 0x002a5ce0,
  net_sign_job_5d50: 0x002a5d50,
  net_sign_job_5dc0: 0x002a5dc0,
  // The Red Sign Soapstone is a DUEL sign. The getter's only callers outside NetSvrSummonSign* are
  // here: 0x14027b410 (called by NetSvrDuelManager 0x14027e630) and 0x14027e0c0 (by 0x14027e840).
  duel_b410: 0x0027b410,
  duel_e0c0: 0x0027e0c0,
  duel_e840: 0x0027e840,
  duel_mgr_e2c0: 0x0027e2c0,
  duel_mgr_e330: 0x0027e330,
  duel_mgr_e370: 0x0027e370,
  duel_mgr_e630: 0x0027e630,
  duel_mgr_e960: 0x0027e960,
  duel_join_mgr_e090: 0x0027e090,
  duel_accept_mgr_b310: 0x0027b310,
  duel_register_wait: 0x0027c250,
  duel_register_wait_dc00: 0x0027dc00,
  duel_update_register_wait: 0x0027c390,
  duel_search: 0x0027c170,
};
for (const [base, count] of [[0x010cb698, 9], [0x010cb6e8, 26]]) {
  for (let i = 1; i < count; i++) {
    const fn = at(base).add(i * 8).readPointer();
    const rva = fn.sub(image.base).toInt32();
    if (Object.values(BROAD).includes(rva)) continue;
    if ([0x00213080, 0x00213160, 0x00213260].includes(rva)) continue; // already hooked above
    BROAD['vt' + base.toString(16) + '_' + i] = rva;
  }
}
const broadCounts = {};
const broadSeen = {};
const broadInstalled = [];
const broadSkipped = [];
for (const [name, rva] of Object.entries(BROAD)) {
  const target = at(rva);
  const first = target.readU8();
  // A one-byte `ret` or a two-byte stub is too short to patch without clobbering its neighbour.
  if (first === 0xc3 || first === 0xcc || (first === 0xc2) || (target.readU16() === 0xc3f3)) {
    broadSkipped.push(name);
    continue;
  }
  try {
    Interceptor.attach(target, {
      onEnter() {
        broadCounts[name] = (broadCounts[name] || 0) + 1;
        if (!broadSeen[name]) {
          broadSeen[name] = true;
          const bt = Thread.backtrace(this.context, Backtracer.ACCURATE).slice(0, 10).map(rel);
          send({ family: 'summon-sign', tag: 'broad.first', hook: name, rva: '0x' + rva.toString(16), backtrace: bt });
        }
      },
    });
    broadInstalled.push(name);
  } catch (e) {
    broadSkipped.push(name + ':' + e.message);
  }
}
send({ family: 'summon-sign', tag: 'broad.hooked', installed: broadInstalled.length, skipped: broadSkipped });
let lastCounts = '';
setInterval(() => {
  const now = JSON.stringify(broadCounts);
  if (now !== lastCounts) {
    lastCounts = now;
    send({ family: 'summon-sign', tag: 'broad.counts', counts: broadCounts });
  }
}, 2000);

const lookupPrologue = Array.from(new Uint8Array(PARAM_LOOKUP.readByteArray(5)));
if (lookupPrologue.join(',') === PARAM_LOOKUP_PROLOGUE.join(',')) {
  globalThis.__lookup = Interceptor.attach(PARAM_LOOKUP, { onEnter: cm.on_enter });
  send({ family: 'summon-sign', tag: 'row.hooked', rows: Object.keys(WATCHED_ROWS) });
} else {
  send({ family: 'summon-sign', tag: 'row.refused', found: lookupPrologue });
}
