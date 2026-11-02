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
// Dedupe keys live on globalThis; clear them so a hot reload logs its first answers again.
for (const k of ['__lastCanUse', '__lastOffline', '__lastReq']) globalThis[k] = undefined;

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
const SMALL_WHITE_SIGN_SOAPSTONE = 62040000;
// WHICH SUMMON THE NEXT NPC SIGN MAKES, picked by the soapstone the player Uses. The bag's sign
// table 0x1410c3d70 pairs each soapstone with its net requirement: White 62030000/0, Small White
// 62040000/1, Red 62045000/2, Dragon Eye 62070000/3 (read from the image 2026-11-02). Hostile is
// everything measured on the red sign; friendly leaves the sign and the slot native (Pate's sign
// type 1, "mini white", user 2026-11-02) and spawns him chrType 2, measured to fight on the
// player's side (user 2026-11-02: "chrType 2 is friendly").
const SUMMON_MODES = {
  [RED_SIGN_SOAPSTONE]: { name: 'hostile', netReq: 2, signType: 5, signChrType: 2, slotChrType: 16, spawnChrType: 16, aura: 3 },
  [SMALL_WHITE_SIGN_SOAPSTONE]: { name: 'friendly', netReq: 1, signType: null, signChrType: null, slotChrType: null, spawnChrType: 2, aura: null },
};
let summonMode = SUMMON_MODES[RED_SIGN_SOAPSTONE];

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
    for (const item of [RED_SIGN_SOAPSTONE, SMALL_WHITE_SIGN_SOAPSTONE]) {
      const s = globalThis.__spawn;
      s.writeU32(0); // mode 0: grant; mode 1 returned false, which does not prove the item is held
      s.add(0x04).writeS32(item);
      s.add(0x08).writeS32(-1);
      s.add(0x0c).writeU16(1);
      s.add(0x0e).writeU8(0);
      s.add(0x0f).writeU8(0);
      const ok = give(inventory, s, 1);
      // ds2-rva ITEM_GIVE_ERROR_OFFSET: inner+0x10138, bit 31 = refused, the rest says why.
      const inner = inventory.add(0x10).readPointer();
      const error = inner.add(0x10138).readU32() >>> 0;
      const detail = inner.add(0x1013c).readU32() >>> 0;
      send({ family: 'summon-sign', tag: 'give', item, granted: ok, error: '0x' + error.toString(16), detail, player: player.toString() });
    }
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
  if (item == 62045000 || item == 62040000)
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
      const forced = FORCE_RED_SIGN_REQUIREMENT && Object.values(SUMMON_MODES).some((m) => m.netReq === this.req) && native === 0;
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
      if (!(this.item in SUMMON_MODES)) return;
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
// NPC SIGN, offline, once per load: ISummonSignSetCtrl slot 3 (vtable +0x18) the way
// 0x140412aa0 calls it -- iface from the getter 0x1402128d0 (no arguments), the chr's entity
// handle from 0x14017b5b0(&out, chr). Slot 3 maps chrType (0x1404124f0) to a sign type with
// 0x140212950 and returns handle 0 when that is 0. The PlayerCtrl at GameManagerImp+0xd0 is not
// one (measured: its +0x50 is not a pointer, so 0x14017b5b0 faults), so the candidates come from
// the table the resolver 0x14017b7e0 reads: block = [[GameManagerImp+0x40]+0x170+b*8] (b < 0x2a),
// ctrl = [[block+0x20]+i*8] (i < [block+0x28]), entity id = (i<<10)|(b<<4)|4. chrType is
// [[ctrl+0x58]+0x84], mapped through the u8 pair table 0x1410ed318 by 0x1404124f0.
const NPC_SIGN_TEST = true;
// The sign is placed when the player Uses a soapstone (USE_ITEM hook below), not on every load.
const SIGN_ON_LOAD = false;
function listCharacters() {
  const mgr = GAME_MANAGER_IMP.readPointer().add(0x40).readPointer();
  const found = [];
  if (mgr.isNull()) return found;
  for (let b = 0; b < 0x2a; b++) {
    const block = mgr.add(0x170 + b * 8).readPointer();
    if (block.isNull()) continue;
    const arr = block.add(0x20).readPointer();
    const count = block.add(0x28).readU32();
    if (arr.isNull()) continue;
    for (let i = 0; i < count && i < 512; i++) {
      const ctrl = arr.add(i * 8).readPointer();
      if (ctrl.isNull()) continue;
      try {
        const kind = ctrl.add(0x58).readPointer().add(0x84).readU8();
        const chrType = at(0x010ed318).add(kind * 2).readU8();
        found.push({ b, i, ctrl, kind, chrType, entity: (i << 10) | (b << 4) | 4 });
      } catch (e) {
        found.push({ b, i, ctrl, error: e.message });
      }
    }
  }
  return found;
}
const signIface = new NativeFunction(at(0x002128d0), 'pointer', []);
const entityOf = new NativeFunction(at(0x0017b5b0), 'pointer', ['pointer', 'pointer']);
const chrTypeOf = new NativeFunction(at(0x004124f0), 'uint8', ['pointer']);
const signTypeOf = new NativeFunction(at(0x00212950), 'uint32', ['uint32']);
globalThis.__signBuf = Memory.alloc(0x10);
let signTested = !SIGN_ON_LOAD;
function npcSignTest() {
  if (!NPC_SIGN_TEST || signTested || !granted) return;
  signTested = true;
  const iface = signIface();
  const chars = listCharacters();
  for (const c of chars) if (c.chrType !== undefined && c.chrType < 0x14) c.signType = signTypeOf(c.chrType);
  send({ family: 'summon-sign', tag: 'npc_sign_test.characters', count: chars.length,
    chars: chars.slice(0, 80).map((c) => ({ b: c.b, i: c.i, ctrl: c.ctrl.toString(), kind: c.kind, chrType: c.chrType, signType: c.signType, error: c.error })) });
  const pick = chars.find((c) => c.signType);
  if (iface.isNull() || !pick) {
    send({ family: 'summon-sign', tag: 'npc_sign_test.refused', iface: iface.toString(), why: pick ? 'no iface' : 'no character with a sign type' });
    return;
  }
  const entity = globalThis.__signBuf;
  const out = globalThis.__signBuf.add(8);
  // Every reload left its sign in the world (handles 0xc00000N1 white, 0xc00000N2), all near the
  // player, so the old white ones hid the new one. Handle = (counter << 4) | type with 0xc0000000
  // set (FUN_14020e4e0). Hide each one found (slot 5 clears rec+0x18 bit 10) before making a new one.
  {
    const findOld = new NativeFunction(iface.readPointer().add(0x98).readPointer(), 'pointer', ['pointer', 'pointer']);
    const hide = new NativeFunction(iface.readPointer().add(5 * 8).readPointer(), 'pointer', ['pointer', 'pointer']);
    const h = Memory.alloc(8);
    let hidden = 0;
    for (let n = 0; n < 0x80; n++) {
      for (const t of [1, 2, 3, 4, 5, 6]) {
        h.writeU32((0xc0000000 | (n << 4) | t) >>> 0);
        const rec = findOld(iface, h);
        if (!rec.isNull()) {
          hide(iface, rec);
          hidden += 1;
        }
      }
    }
    send({ family: 'summon-sign', tag: 'npc_sign_test.hid_old', hidden });
  }
  entity.writeU32(pick.entity);
  out.writeU32(0);
  const slot3 = new NativeFunction(iface.readPointer().add(0x18).readPointer(), 'pointer', ['pointer', 'pointer', 'pointer']);
  globalThis.__signStage = 'slot3 entity 0x' + pick.entity.toString(16);
  // The position is taken AT CREATION: slot 3 reads the chr's world matrix from 0x1404123c0 (a 0x40
  // byte out-buffer, translation at +0x30) and 0x1402122b0 makes it map-area-local before the
  // record is stored -- rewriting the record afterwards moved nothing on screen (measured). So the
  // matrix's translation is replaced by the player's position (PlayerCtrl +0x90) (under them), for
  // this one call only.
  const here = GAME_MANAGER_IMP.readPointer().add(PLAYER_CTRL_OFFSET).readPointer().add(0x90);
  const matrixHook = Interceptor.attach(at(0x004123c0), {
    onLeave(ret) {
      if (!this.returnAddress.equals(at(0x002132ca))) return;
      const before = [ret.add(0x30).readFloat(), ret.add(0x34).readFloat(), ret.add(0x38).readFloat()];
      // Under the player, exactly (user directive 2026-11-02: never in front or elsewhere).
      ret.add(0x30).writeFloat(here.readFloat());
      ret.add(0x34).writeFloat(here.add(4).readFloat());
      ret.add(0x38).writeFloat(here.add(8).readFloat());
      send({ family: 'summon-sign', tag: 'npc_sign_test.matrix', before: before.map((v) => v.toFixed(2)),
        after: [0x30, 0x34, 0x38].map((o) => ret.add(o).readFloat().toFixed(2)) });
    },
  });
  // Sign colour: 0x140212950(chrType) -> table 0x1410cb5c4 (chrType 1 -> 1 white, 2 -> 2 red, 16 ->
  // 0, no sign). Pate's chrType is 1, so his sign is white; the summon behind it is forced to 16
  // (hostile) at spawn regardless. Make the sign red for this creation only.
  // Measured by the user 2026-11-02: 1 mini white, 2 not red (0x451), 4 intense gold (0x455). 5 is
  // what chrTypes 7/9/14/15 map to (0x1410cb5c4) -- the red-phantom candidates.
  const SIGN_TYPE = summonMode.signType;
  const signTypeHook = Interceptor.attach(at(0x00212950), {
    onLeave(ret) {
      send({ family: 'summon-sign', tag: 'npc_sign_test.sign_type', mode: summonMode.name, from: rel(this.returnAddress), native: ret.toInt32(), forced: SIGN_TYPE });
      if (SIGN_TYPE !== null) ret.replace(ptr(SIGN_TYPE));
    },
  });
  // Measured 2026-11-02: forcing the sign type alone left the sign white. Slot 3 also stores
  // FUN_1404124f0(gen)'s chrType in the record at +0x20 (0x140213344), and that is what the sign and
  // the touch's NetNpcPhantom slot carry (the slot read type 1). Hand slot 3 chrType 2 (red row).
  const SIGN_CHR_TYPE = summonMode.signChrType;
  const signChrHook = Interceptor.attach(at(0x004124f0), {
    onLeave(ret) {
      if (!this.returnAddress.equals(at(0x002132e4))) return;
      send({ family: 'summon-sign', tag: 'npc_sign_test.chr_type', native: ret.toInt32() & 0xff, forced: SIGN_CHR_TYPE });
      if (SIGN_CHR_TYPE !== null) ret.replace(ptr(SIGN_CHR_TYPE));
    },
  });
  slot3(iface, out, entity);
  signChrHook.detach();
  signTypeHook.detach();
  matrixHook.detach();
  send({ family: 'summon-sign', tag: 'npc_sign_test', ctrl: pick.ctrl.toString(), kind: pick.kind, chrType: pick.chrType,
    signType: pick.signType, entity: '0x' + pick.entity.toString(16), handle: '0x' + out.readU32().toString(16) });
  // Where it went: find the sign by handle (vtable +0x98, as 0x140412aa0 does) and dump it beside
  // the player's position (PlayerCtrl +0x90, scripts/frida/arrow.js).
  const find = new NativeFunction(iface.readPointer().add(0x98).readPointer(), 'pointer', ['pointer', 'pointer']);
  const sign = find(iface, out);
  const player = GAME_MANAGER_IMP.readPointer().add(PLAYER_CTRL_OFFSET).readPointer();
  const floats = (p, n) => Array.from({ length: n }, (_, k) => p.add(k * 4).readFloat().toFixed(2));
  send({ family: 'summon-sign', tag: 'npc_sign_test.where', sign: sign.toString(),
    sign_words: sign.isNull() ? null : Array.from({ length: 0x20 }, (_, k) => '0x' + (sign.add(k * 4).readU32() >>> 0).toString(16)),
    sign_floats: sign.isNull() ? null : floats(sign, 0x20),
    point: '0x' + (pick.ctrl.add(0x50).readPointer().add(0x30).readU32() >>> 0).toString(16),
    player: floats(player.add(0x90), 3) });
  globalThis.__watchSign = sign;
  globalThis.__visSign = sign;
  // Drawn only when rec+0x18 bits 9 AND 10 are set: 9 is the per-frame distance test, 10 is set by
  // ISummonSignSetCtrl slot 7 (0x1402139b0 -> 0x14020d0e0(rec, 1, 1)) and cleared by slot 5
  // (0x140213930). A new record starts with 10 clear (measured: d0e0(rec, 1, 0) at creation).
  // Still white with type 2 in the handle and chrType 2 at +0x20 (user, 2026-11-02). Slot 3 passes
  // 2 as FUN_14020cf00's 4th argument, which lands in rec+0x14 bits 29-31 as (v | ~3) << 29: the
  // sign's source. Try a player-style source (0) before the sign is shown.
  const SIGN_SOURCE = null;
  if (!sign.isNull() && SIGN_SOURCE !== null) {
    const w = sign.add(0x14).readU32() >>> 0;
    const now = ((w & 0x1fffffff) | (((SIGN_SOURCE | 0xfffffffc) << 29) >>> 0)) >>> 0;
    sign.add(0x14).writeU32(now);
    send({ family: 'summon-sign', tag: 'npc_sign_test.source', was: '0x' + w.toString(16), now: '0x' + now.toString(16) });
  }
  if (!sign.isNull()) {
    const show = new NativeFunction(iface.readPointer().add(7 * 8).readPointer(), 'pointer', ['pointer', 'pointer']);
    show(iface, sign);
    send({ family: 'summon-sign', tag: 'npc_sign_test.shown', w18: '0x' + (sign.add(0x18).readU32() >>> 0).toString(16) });
  }
  // The touch needs the NPC's character loaded (0x140359530), which offline it is not: request it
  // now through the native "prepare generator" 0x140412970, so it is resident by the time of the touch.
  // A summoned NPC kept alive (the leave reason is suppressed) holds his NetNpcPhantom slot, and the
  // next touch of his sign then does nothing (measured 2026-11-02). Release it the game's way: set
  // the slot's next state (+0x18) to 0, so FUN_140252270 runs FUN_140252620(0) -> FUN_140252580
  // (despawn, clear). The slot is the one FUN_140252620 last reported (stable heap address in a
  // session; 0x7ffffe5935f0 in this one) -- only touched when its entity matches.
  // All five slots (manager +0x10, stride 0x4c, FUN_1402540f0) -- assuming 0x7ffffe5935f0 is slot 0.
  try {
    const base = ptr('0x7ffffe5935f0');
    const slots = [];
    for (let k = -4; k < 5; k++) {
      const s = base.add(k * 0x4c);
      slots.push({ k, slot: s.toString(), ent: '0x' + (s.add(4).readU32() >>> 0).toString(16), type: s.add(8).readU8(), cur: s.add(0x14).readS32(), next: s.add(0x18).readS32() });
    }
    send({ family: 'summon-sign', tag: 'npc_sign_test.slots', slots });
  } catch (e) {
    send({ family: 'summon-sign', tag: 'npc_sign_test.slots', error: e.message });
  }
  try {
    const slot = globalThis.__npcSlot || ptr('0x7ffffe5935f0');
    const ent = slot.add(4).readU32();
    const cur = slot.add(0x14).readS32();
    send({ family: 'summon-sign', tag: 'npc_sign_test.slot_before', slot: slot.toString(), ent: '0x' + (ent >>> 0).toString(16), cur, next: slot.add(0x18).readS32() });
    if (ent === pick.entity && cur !== 0) slot.add(0x18).writeS32(0);
  } catch (e) {
    send({ family: 'summon-sign', tag: 'npc_sign_test.slot_before', error: e.message });
  }
  prepareGenerator(pick.ctrl, 0);
  send({ family: 'summon-sign', tag: 'npc_sign_test.prepared', chrId: pick.ctrl.add(0x60).readPointer().readU32() });
}
// WHY A SIGN IS NOT DRAWN. The per-set loop 0x14020e8c0 hides a record while
// (rec+0x18 & 0xff000) >= 0xfe000 (0x14020eb50), and for 0xff000 it first runs 0x14020cb50, which
// checks map area (rec+0x14 >> 8) & 0x3f through [GameManagerImp+0x38] (0x1403bcf60, then
// 0x1403bcfa0) and casts down from rec+0x4 (0x140bc1950) to place it. Report the watched record's
// two words on change, and both area checks' answers for it.
const areaCheckA = at(0x003bcf60);
const areaCheckB = at(0x003bcfa0);
for (const [name, fn] of [['area_a', areaCheckA], ['area_b', areaCheckB]]) {
  Interceptor.attach(fn, {
    onEnter(args) {
      this.area = args[1].toInt32();
      this.mine = globalThis.__inCellAssign === true;
    },
    onLeave(ret) {
      if (this.mine) send({ family: 'summon-sign', tag: 'sign.' + name, area: this.area, result: ret.toInt32() & 0xff });
    },
  });
}
Interceptor.attach(at(0x0020cb50), {
  onEnter(args) {
    globalThis.__inCellAssign = globalThis.__watchSign !== undefined && args[0].equals(globalThis.__watchSign);
    this.rec = args[0];
  },
  onLeave() {
    if (globalThis.__inCellAssign) {
      send({ family: 'summon-sign', tag: 'sign.cell_assign', w18: '0x' + (this.rec.add(0x18).readU32() >>> 0).toString(16) });
      globalThis.__inCellAssign = false;
      globalThis.__cellAssignSeen = (globalThis.__cellAssignSeen || 0) + 1;
      if (globalThis.__cellAssignSeen > 3) globalThis.__watchSign = undefined;
    }
  },
});
// Kind-3 records (handle top bits 0xc0000000) are visible when the sign type's range
// (0x1402127c0) squared exceeds the squared distance to the player (0x14020cdb0); the answer goes to
// 0x14020d0e0(rec, 0, visible). Log that answer for the watched record on change.
let lastVisible = -1;
Interceptor.attach(at(0x0020d0e0), {
  onEnter(args) {
    const s = globalThis.__visSign;
    if (s === undefined || !args[0].equals(s)) return;
    const v = args[2].toInt32() & 0xff;
    if (v === lastVisible) return;
    lastVisible = v;
    send({ family: 'summon-sign', tag: 'sign.visible', visible: v, mode: args[1].toInt32() & 0xff,
      backtrace: Thread.backtrace(this.context, Backtracer.ACCURATE).slice(0, 8).map(rel) });
  },
});
for (const [name, rva] of [['hide_slot5', 0x00213930], ['flag_207a60', 0x00207a60], ['flag_thunk_204f10', 0x00204f10]]) {
  Interceptor.attach(at(rva), {
    onEnter(args) {
      const s = globalThis.__visSign;
      if (s === undefined) return;
      if (!args[1].equals(s) && !args[0].equals(s)) return;
      send({ family: 'summon-sign', tag: 'sign.flag_caller', name, ret: rel(this.returnAddress),
        backtrace: Thread.backtrace(this.context, Backtracer.FUZZY).slice(0, 8).map(rel) });
    },
  });
}
// WHO HIDES IT (measured): the net sign tick walks the NPC signs (iface slot +0xa8) and, offline,
// asks 0x1402a1980(mgr, typeByte, 0, &info, 3) at 0x1402a48cc; false -> slot 5 hide (+0x28), true
// -> slot 7 show (+0x38). `[rbx]` there is the record both receive. Force true for the watched
// record only, logging the native answer once.
const SIGN_RULE_RET = at(0x002a48d1);
let ruleLogged = false;
Interceptor.attach(at(0x002a1980), {
  onEnter(args) {
    this.mine = false;
    const s = globalThis.__visSign;
    if (s === undefined || !this.returnAddress.equals(SIGN_RULE_RET)) return;
    this.mine = this.context.rbx.readPointer().equals(s);
    this.type = args[1].toInt32() & 0xff;
  },
  onLeave(ret) {
    if (!this.mine) return;
    const native = ret.toInt32() & 0xff;
    if (!ruleLogged) {
      ruleLogged = true;
      send({ family: 'summon-sign', tag: 'sign.rule', type: this.type, native });
    }
    if (native === 0) ret.replace(ptr(1));
  },
});
// AFTER THE TOUCH (measured by the user 2026-11-02: touching the sign summoned Mild-Mannered Pate,
// then a banner said he was sent home before he appeared). Every call, with a backtrace: the NPC
// phantom generator FUN_140415b20 and its spawn step FUN_140418d50, EventPhantomReturn
// 0x14018d8e0. (NetNpcPhantomDeadSync's sender 0x140254c00 runs every frame: not hooked.)
for (const [name, rva] of [['npc_phantom_generate', 0x00415b20], ['npc_phantom_spawn', 0x00418d50],
  ['event_phantom_return', 0x0018d8e0]]) {
  try {
    Interceptor.attach(at(rva), {
      onEnter(args) {
        send({ family: 'summon-sign', tag: 'phantom.' + name, args: [args[0], args[1], args[2]].map((a) => a.toString()),
          backtrace: Thread.backtrace(this.context, Backtracer.ACCURATE).slice(0, 10).map(rel) });
        if (name === 'npc_phantom_generate') touchSummonEnter(this, args);
      },
      onLeave(ret) {
        if (name === 'npc_phantom_generate') touchSummonLeave(this, ret);
        send({ family: 'summon-sign', tag: 'phantom.' + name + '.ret', ret: ret.toString(),
          handle: name === 'npc_phantom_spawn' && !ret.isNull() ? '0x' + (ret.readU32() >>> 0).toString(16) : undefined });
      },
    });
  } catch (e) {
    send({ family: 'summon-sign', tag: 'phantom.refused', name, error: e.message });
  }
}
// WHO SENDS HIM HOME. Measured 2026-11-02: the touch spawned Pate (chrType 16, 0x140356060 returned
// a character), then the on-screen banner said he was sent home and he never appeared. Log, with a
// backtrace, the generator despawn 0x1404193f0(mgr, entry) and the character removal it uses,
// 0x140316010(chr, ...), for any character -- both are rare.
for (const [name, rva] of [['despawn_4193f0', 0x004193f0], ['remove_316010', 0x00316010]]) {
  Interceptor.attach(at(rva), {
    onEnter(args) {
      send({ family: 'summon-sign', tag: 'phantom.' + name, args: [args[0], args[1]].map((a) => a.toString()),
        backtrace: Thread.backtrace(this.context, Backtracer.ACCURATE).slice(0, 12).map(rel) });
    },
  });
}
// THE SEND-HOME TIMER. Measured 2026-11-02: after the touch spawned Pate, NetNpcPhantom's per-slot
// update FUN_140252270 despawned him via FUN_140252620 -> FUN_140252580 -> 0x1404193f0. In the
// active state (+0x14 == 2) the slot's timer +0x1c counts down unless 0x1402d8650() (a session byte,
// [[0x141616cf8]+0x18]+0xa) AND 0x1402520b0(slot+8) (chrType row byte 0 is 2 or 3) both hold; when
// it runs out the slot goes to state 3 (leaving) and he is sent home. Hold both true offline.
const KEEP_PHANTOM = true;
for (const [name, rva, site] of [['keep_session', 0x002d8650, 0x002522f7], ['keep_chr_type', 0x002520b0, 0x00252304]]) {
  let logged = 0;
  Interceptor.attach(at(rva), {
    onEnter(args) {
      this.on = this.returnAddress.equals(at(site));
      if (this.on && name === 'keep_chr_type') this.type = args[0].readU8();
    },
    onLeave(ret) {
      if (!this.on) return;
      const native = ret.toInt32() & 0xff;
      if (logged < 3 || native === 0 && logged < 6) {
        logged += 1;
        send({ family: 'summon-sign', tag: 'phantom.' + name, native, chrType: this.type, forced: KEEP_PHANTOM });
      }
      if (KEEP_PHANTOM && native === 0) ret.replace(ptr(1));
    },
  });
}
// With both held, he was still despawned (measured 2026-11-02): the slot still went 3 -> 0. Log
// every state change FUN_140252620(slot, ?, state) with the slot's fields and caller, and remember
// the slot so watchSlot() can report its state/timers every time they change.
// The sign's effect: FUN_14020a5d0(set, &handle, x, y, z, ...) spawns 0x140212730(handle & 0xf)
// (type 1 -> 0x44e, 2 -> 0x451, 3 -> 0x453, 4 -> 0x455). Log each spawn for our sign.
Interceptor.attach(at(0x0020a5d0), {
  onEnter(args) {
    const handle = args[1].readU32() >>> 0;
    if (globalThis.__watchSign && handle === (globalThis.__watchSign.readU32() >>> 0))
      send({ family: 'summon-sign', tag: 'sign.sfx', handle: '0x' + handle.toString(16), type: handle & 0xf });
  },
});
// THE REQUEST (OK press stopped summoning, 2026-11-02). FUN_1402524b0(slot, id, ctx, &entity) is
// what puts a slot into state 1: it takes chrType from the entity, asks FUN_1402c08e0(ctx, &out,
// &chrType) for an allocation, and refuses when out = (0, <0). Log both.
Interceptor.attach(at(0x002524b0), {
  onEnter(args) { this.entity = args[3].readU32(); },
  onLeave(ret) { send({ family: 'summon-sign', tag: 'phantom.request', entity: '0x' + (this.entity >>> 0).toString(16), ok: ret.toInt32() & 0xff }); },
});
// FUN_1402c08e0 refuses (out = 0, -1) whenever [[ctx+8]+0xc] != [[ctx+8]+0x10] -- measured
// refusing after the earlier summons I kept alive/interrupted. Log both and, with
// FORCE_OK_CHECKS, make +0x10 equal +0xc so the allocation (FUN_1402d3110) runs.
Interceptor.attach(at(0x002c08e0), {
  onEnter(args) {
    this.out = args[1];
    this.type = args[2].readU8();
    const q = args[0].add(8).readPointer();
    const a = q.add(0xc).readS32();
    const b = q.add(0x10).readS32();
    // Not forced: a != b is a pending mode change, whose FUN_1402c09a0 -> FUN_1402d3820 resets the pool.
    send({ family: 'summon-sign', tag: 'phantom.request_busy', a, b });
  },
  onLeave() { send({ family: 'summon-sign', tag: 'phantom.request_alloc', type: this.type, out: [this.out.readS32(), this.out.add(4).readS32()] }); },
});
// THE ALLOCATION ITSELF (busy counters measured equal, 14/14, and it still refused). FUN_1402d3110
// (ctx) refuses unless FUN_1402d1790 (per-type caps) and FUN_1402d1e00 pass, then FUN_1402d3040
// marks member entries used. FUN_1402d1e00 wants >= 1 entry free in a 5-entry pool at
// [ctx+0x70+0x1b0] (stride 0x14, +8 == 0 free); FUN_1402d3040 writes 2 there per summon and nothing
// frees it offline, so the sixth summon is refused. Log the pool (read only: writing state 0 by
// hand leaves table0's `used` counted, which is the same leak).
Interceptor.attach(at(0x002d3110), {
  onEnter(args) {
    const pool = args[0].add(0x220).readPointer();
    const states = [];
    for (let i = 0; i < 5; i++) states.push(pool.add(i * 0x14 + 8).readS32());
    send({ family: 'summon-sign', tag: 'phantom.alloc_pool', type: args[2].readU8(), states });
  },
});
// THE LEAK (static, scratchpad alloc-prereqs.md, 2026-11-02): the release FUN_140252580 ->
// FUN_1402c0950(q, &slot[+8], id) -> FUN_1402d3360(ctx+0x70, &type, id) finds the capacity record
// by the type's mask bit. SLOT_CHR_TYPE rewrites slot+8 to 16 after the allocation took type 1, and
// no record carries type 16's bit, so nothing is freed and table0 (type 1: cap 3) fills after three
// summons. Hand the release the type the pool entry recorded for this id (entry+0xc).
Interceptor.attach(at(0x002c0950), {
  onEnter(args) {
    const pool = args[0].readPointer().add(0x220).readPointer();
    const id = args[2];
    for (let i = 0; i < 5; i++) {
      const e = pool.add(i * 0x14);
      if (!e.readPointer().equals(id)) continue;
      const recorded = e.add(0xc).readU8();
      const passed = args[1].readU8();
      send({ family: 'summon-sign', tag: 'phantom.release_type', id: id.toString(), passed, recorded });
      if (passed !== recorded) args[1].writeU8(recorded);
      return;
    }
    send({ family: 'summon-sign', tag: 'phantom.release_type', id: id.toString(), passed: args[1].readU8(), recorded: null });
  },
});
// Measured: pool [2,2,2,0,0] (two free) and alloc_free still 0, so its other half refused:
// FUN_1402d2df0(ctx+0x70, type) wants three per-type records (FUN_1402d26e0 lookups in tables at
// +0, +0x90, +0x120) whose +8 reads 1, then 1-2, then 1-2. Log those three; while the pool has
// room, force alloc_free's answer to 1.
for (const [name, rva] of [['alloc_caps', 0x002d1790], ['alloc_free', 0x002d1e00], ['alloc_mark', 0x002d3040]]) {
  Interceptor.attach(at(rva), {
    onEnter() { this.mine = globalThis.__inAlloc === true; if (this.mine && name === 'alloc_free') globalThis.__typeStates = []; },
    onLeave(ret) {
      if (!this.mine) return;
      const native = ret.toInt32() & 0xff;
      send({ family: 'summon-sign', tag: 'phantom.' + name, result: native, typeStates: name === 'alloc_free' ? globalThis.__typeStates : undefined });
    },
  });
}
Interceptor.attach(at(0x002d2df0), {
  onEnter() { this.mine = globalThis.__inAlloc === true; if (this.mine) globalThis.__inTypeStates = true; },
  onLeave() { if (this.mine) globalThis.__inTypeStates = false; },
});
// THE MARK STEP FUN_1402d3040 (measured: refuses after alloc_free passes). It needs
// FUN_1402d20e0(pool, out, id) to find pool entries whose first 8 bytes are the new id (count at
// out+0x30), and FUN_1402d27c0(ctx+0x70, out, type) to read 1 at out+8 (first table). Log both,
// and the pool's entries after FUN_1402d3640 assigns them.
for (const [name, rva, off, wide] of [['mark_entries', 0x002d20e0, 0x30, true], ['mark_state', 0x002d27c0, 8, false]]) {
  Interceptor.attach(at(rva), {
    onEnter(args) { this.mine = globalThis.__inAlloc === true; this.out = args[1]; this.id = args[2]; },
    onLeave() {
      if (!this.mine) return;
      const v = wide ? this.out.add(off).readU64().toString() : this.out.add(off).readS32();
      send({ family: 'summon-sign', tag: 'phantom.' + name, value: v, id: name === 'mark_entries' ? this.id.toString() : undefined });
    },
  });
}
Interceptor.attach(at(0x002d3640), {
  onEnter(args) { this.mine = globalThis.__inAlloc === true; this.ctx = args[0]; this.id = args[2]; },
  onLeave() {
    if (!this.mine) return;
    const pool = this.ctx.add(0x1b0).readPointer();
    const entries = [];
    for (let i = 0; i < 5; i++) entries.push([pool.add(i * 0x14).readU32(), pool.add(i * 0x14 + 4).readU32(), pool.add(i * 0x14 + 8).readS32()].map((x) => '0x' + (x >>> 0).toString(16)).join(':'));
    send({ family: 'summon-sign', tag: 'phantom.assign', id: this.id.toString(), entries });
  },
});
Interceptor.attach(at(0x002d26e0), {
  onLeave(ret) {
    if (globalThis.__inTypeStates !== true) return;
    const r = ptr(ret.toString());
    try {
      const v = r.add(8).readS32();
      // 0 = no room, 1 = room, 2 = no record for this type; computed per lookup, not stored.
      globalThis.__typeStates.push(v);
    } catch (e) { globalThis.__typeStates.push(e.message); }
  },
});
Interceptor.attach(at(0x002d3110), {
  onEnter() { globalThis.__inAlloc = true; },
  onLeave() { globalThis.__inAlloc = false; },
});
// THE OK PRESS (stopped working after several summons, 2026-11-02). The dialog's OK reaches
// FUN_1402a41b0(mgr, &handle), which summons (FUN_1402a2f00 for an NPC sign) only if three checks
// pass: 0x140291c30 (net properties), 0x140275ea0 (mgr, chrType, isNpc), 0x1402c6460 (context).
// Log each result at its call site in FUN_1402a41b0; FORCE_OK_CHECKS makes a refusal pass.
const FORCE_OK_CHECKS = true;
for (const [name, rva, site] of [['ok_net_props', 0x00291c30, 0x002a426d], ['ok_mgr', 0x00275ea0, 0x002a4284], ['ok_ctx', 0x002c6460, 0x002a42c1]]) {
  Interceptor.attach(at(rva), {
    onEnter() { this.on = this.returnAddress.equals(at(site)); },
    onLeave(ret) {
      if (!this.on) return;
      const native = ret.toInt32() & 0xff;
      send({ family: 'summon-sign', tag: 'phantom.' + name, native, forced: FORCE_OK_CHECKS && native === 0 });
      if (FORCE_OK_CHECKS && native === 0) ret.replace(ptr(1));
    },
  });
}
// After the OK checks, FUN_1402a2f00 -> FUN_140253f60(mgr, &entity) enqueues the NPC summon
// (FUN_140254b70) only if 0x1402c06c0(ctx, &entity) and 0x140412060(gen) hold. With ok_mgr forced
// the sign vanished and nothing came (2026-11-02): log both and the enqueue; force the first.
for (const [name, rva, site] of [['enq_entity', 0x002c06c0, 0x00253f9b], ['enq_gen', 0x00412060, 0x00253fc3]]) {
  Interceptor.attach(at(rva), {
    onEnter() { this.on = this.returnAddress.equals(at(site)); },
    onLeave(ret) {
      if (!this.on) return;
      const native = ret.toInt32() & 0xff;
      send({ family: 'summon-sign', tag: 'phantom.' + name, native, forced: FORCE_OK_CHECKS && native === 0 });
      if (FORCE_OK_CHECKS && native === 0) ret.replace(ptr(1));
    },
  });
}
Interceptor.attach(at(0x00254b70), {
  onEnter(args) { send({ family: 'summon-sign', tag: 'phantom.enqueue', entity: '0x' + (args[1].readU32() >>> 0).toString(16) }); },
});
const slotFields = (s) => ({ cur: s.add(0x14).readS32(), next: s.add(0x18).readS32(), t1c: s.add(0x1c).readFloat().toFixed(2),
  t20: s.add(0x20).readFloat().toFixed(2), r34: s.add(0x34).readS32(), r38: s.add(0x38).readS32(), type: s.add(8).readU8(), ent: '0x' + (s.add(4).readU32() >>> 0).toString(16) });
Interceptor.attach(at(0x00252620), {
  onEnter(args) {
    // FUN_140252270 passes its own slot (rbx) as the first argument; the decomp shows param_1.
    globalThis.__npcSlot = args[0];
    // Entering "summoning" (1): FUN_140252620 hands slot+8 (the phantom type) to the phantom
    // director (vtable +8). Pate came in blue with type 1; make it the invader row.
    const SLOT_CHR_TYPE = summonMode.slotChrType;
    if (SLOT_CHR_TYPE !== null && args[2].toInt32() === 1 && args[0].add(4).readU32() === SUMMON_ENTITY) {
      const was = args[0].add(8).readU8();
      args[0].add(8).writeU8(SLOT_CHR_TYPE);
      send({ family: 'summon-sign', tag: 'phantom.slot_type', was, now: SLOT_CHR_TYPE });
    }
    send({ family: 'summon-sign', tag: 'phantom.slot_state', state: args[2].toInt32(), slot: args[0].toString(), fields: slotFields(args[0]),
      backtrace: Thread.backtrace(this.context, Backtracer.ACCURATE).slice(0, 6).map(rel) });
  },
});
let lastSlot = '';
function watchSlot() {
  const s = globalThis.__npcSlot;
  if (!s) return;
  const f = slotFields(s);
  const key = [f.cur, f.next, f.r34, f.r38, f.type].join(',');
  if (key === lastSlot) return;
  lastSlot = key;
  send({ family: 'summon-sign', tag: 'phantom.slot_watch', fields: f });
}
// THE LEAVE REQUEST. Measured 2026-11-02 (slot trace): in the active state the slot's +0x20 was set
// to 0.01 with +0x38 = 2, which expires into state 3 (leaving) and then 0 (despawn). The setter is
// FUN_1402521b0(slot, reason) (reason 1 -> 3 s, 2 -> 0.01 s), called only by the manager update
// FUN_1402540f0 with a reason FUN_140254780 pops for the slot's entity from the dead-notification
// queue at mgr+0x1c8. Pop it as usual, but hand back 0 for the summoned entity.
const KEEP_FROM_DEAD_QUEUE = true;
Interceptor.attach(at(0x00254780), {
  onEnter(args) {
    this.on = this.returnAddress.equals(at(0x0025436a));
    if (this.on) this.entity = args[1].readU32();
  },
  onLeave(ret) {
    if (!this.on) return;
    const reason = ret.toInt32();
    if (reason === 0) return;
    // Measured: once he is up the queue reports him every frame -- log only a change.
    const key = this.entity + ':' + reason;
    if (globalThis.__lastLeave !== key) {
      globalThis.__lastLeave = key;
      send({ family: 'summon-sign', tag: 'phantom.leave_reason', entity: '0x' + (this.entity >>> 0).toString(16), reason, kept: KEEP_FROM_DEAD_QUEUE && this.entity === SUMMON_ENTITY });
    }
    if (KEEP_FROM_DEAD_QUEUE && this.entity === SUMMON_ENTITY) ret.replace(ptr(0));
  },
});
// WHY THE SUMMON FAILS (measured: FUN_140415b20 got handle -1 back, so 0x14041e870 returned 0).
// Each spawn layer's return, and the handle FUN_140418d50 wrote to its out-parameter.
for (const [name, rva] of [['spawn_418690', 0x00418690], ['spawn_356060', 0x00356060], ['spawn_3560a0', 0x003560a0]]) {
  Interceptor.attach(at(rva), {
    onEnter(args) { this.on = globalThis.__summoning === true; },
    onLeave(ret) { if (this.on) send({ family: 'summon-sign', tag: 'phantom.' + name + '.ret', ret: ret.toString() }); },
  });
}
// FUN_140418d50's first three gates: 0x140412060 false -> exit; 0x14014b420 non-null -> 0x140419171;
// 0x1404161d0 false -> 0x14041910d (which sets ctrl+0x7a bit 0).
for (const [name, rva, site] of [['gate_412060', 0x00412060, 0x00418da4], ['gate_14b420', 0x0014b420, 0x00418dd9],
  ['gate_4161d0', 0x004161d0, 0x00418dee], ['spawn_gate_14b420', 0x0014b420, 0x004186df]]) {
  Interceptor.attach(at(rva), {
    onLeave(ret) {
      if (globalThis.__summoning === true && this.returnAddress.equals(at(site))) {
        send({ family: 'summon-sign', tag: 'phantom.' + name, ret: ret.toString() });
      }
    },
  });
}
// The spawn itself (0x140418690 at 0x140418ffd) needs r13b = 1 (the map-area compare at 0x140418e6e
// failed to match), [mgr+0x301] bit 0 clear and [mgr+0x302] bit 1 clear; else 0x1404190c2 runs
// 0x1402d55c0 and leaves. 0x140418fa8 is reached on every path to that test.
Interceptor.attach(at(0x00418fa8), function () {
  if (globalThis.__summoning !== true) return;
  const c = this.context;
  send({ family: 'summon-sign', tag: 'phantom.spawn_test', r13b: c.r13.toInt32() & 0xff,
    b301: c.rbx.add(0x301).readU8(), b302: c.rbx.add(0x302).readU8(), mgr: c.rbx.toString(),
    area_global: Array.from(new Uint8Array(at(0x01614c20).readByteArray(16))).map((b) => b.toString(16).padStart(2, '0')).join('') });
  // Measured blockers (2026-11-02, Pate in m10_10_00_00): r13b = 0 and [mgr+0x302] = 0xfc (bit 1
  // set). FORCE_SPAWN overrides both for this one call; the flag byte is restored after it.
  if (FORCE_SPAWN) {
    c.r13 = c.r13.and(ptr('0xffffffffffffff00')).or(1);
    globalThis.__b302 = { at: c.rbx.add(0x302), was: c.rbx.add(0x302).readU8() };
    globalThis.__b302.at.writeU8(globalThis.__b302.was & ~2);
  }
});
const FORCE_SPAWN = true;
// false: FUN_14040f9c0 shows the spawn position is the generator entity's +0x20 (+0x30 is the
// rotation), so summonTest writes +0x20 instead and this override is no longer the mechanism.
const SPAWN_AT_PLAYER = false;
// Measured with Stalker: the spawn gives up after 0x1404160f0(mgr, desc), whose 0x14041edc0(desc,
// state) returns bit (state-1) of the u16 at desc+0xe; state = (s8)[mgr+0x2fc], desc = [gen-ish+0x60].
// Log both, and with FORCE_SPAWN answer true for this call.
Interceptor.attach(at(0x0041edc0), {
  onEnter(args) {
    this.on = globalThis.__summoning === true && this.returnAddress.equals(at(0x00416127));
    if (!this.on) return;
    // desc +0 chr id checked against the loaded-resource list, +4/+8 event flags (0x1404160f0).
    // The loaded list: [[GameManagerImp+0x18]+0x830]+0x10, node +0x14 id, +0x60 next (0x140350fd0).
    const loaded = [];
    try {
      let node = GAME_MANAGER_IMP.readPointer().add(0x18).readPointer().add(0x830).readPointer().add(0x10).readPointer();
      for (let k = 0; k < 400 && !node.isNull(); k++) {
        loaded.push(node.add(0x14).readS32());
        node = node.add(0x60).readPointer();
      }
    } catch (e) {
      loaded.push('error ' + e.message);
    }
    send({ family: 'summon-sign', tag: 'phantom.state_mask', state: args[1].toInt32(), mask: '0x' + args[0].add(0xe).readU16().toString(16),
      desc: args[0].toString(), chr: args[0].readS32(), flag1: args[0].add(4).readS32(), flag2: args[0].add(8).readS32(), loaded });
  },
  onLeave(ret) {
    if (this.on && FORCE_SPAWN && (ret.toInt32() & 0xff) === 0) ret.replace(ptr(1));
  },
});
// 0x140418690(mgr, gen, ...) refuses when gen+0x7a bit 1 is set and gen+0x76 has bits 0-1 set
// with bits 2 and 3 clear. Log both on entry, and with FORCE_SPAWN clear 0x7a bit 1 for this call.
Interceptor.attach(at(0x00418690), {
  onEnter(args) {
    if (globalThis.__summoning !== true) return;
    const gen = args[1];
    const fl = (q) => { try { return [0, 4, 8].map((o) => q.add(o).readFloat().toFixed(2)); } catch (e) { return 'unreadable'; } };
    const sp = this.context.rsp;
    send({ family: 'summon-sign', tag: 'phantom.spawn_gen', gen: gen.toString(), b76: gen.add(0x76).readU8(), b7a: gen.add(0x7a).readU8(),
      r8: args[2].toString(), r9: args[3].toString(), at_r9: fl(args[3]),
      stack: [0x28, 0x30, 0x38, 0x40].map((o) => { const v = sp.add(o).readPointer(); return { o: '0x' + o.toString(16), v: v.toString(), f: fl(v) }; }),
      gen_floats: [0x20, 0x30, 0x40, 0x50].map((o) => ({ o: '0x' + o.toString(16), f: fl(gen.add(o)) })) });
    // Measured 2026-11-02: the spawn position is *r9, copied from the POOLED generator FUN_140418d50
    // picks (gen+0x1c), not from the entry FUN_140415b20 was given; a position written on that entry
    // arrives one level up and is dropped. Overwrite *r9 with the player's position + 3 m on x.
    if (SPAWN_AT_PLAYER) {
      const me = GAME_MANAGER_IMP.readPointer().add(PLAYER_CTRL_OFFSET).readPointer().add(0x90);
      args[3].writeFloat(me.readFloat() + 3);
      args[3].add(4).writeFloat(me.add(4).readFloat());
      args[3].add(8).writeFloat(me.add(8).readFloat());
    }
    if (FORCE_SPAWN) {
      this.restore = { at: gen.add(0x7a), was: gen.add(0x7a).readU8() };
      this.restore.at.writeU8(this.restore.was & ~2);
    }
    // Every call made inside this one spawn attempt, to find where it gives up.
    this.tid = this.threadId;
    const lo = image.base;
    const hi = image.base.add(image.size);
    globalThis.__spawnCalls = [];
    Stalker.follow(this.tid, {
      events: { call: true },
      onReceive(events) {
        for (const e of Stalker.parse(events, { annotate: false, stringify: false })) {
          const target = e[1];
          if (target.compare(lo) >= 0 && target.compare(hi) < 0) globalThis.__spawnCalls.push(rel(e[0]) + '>' + rel(target));
        }
      },
    });
  },
  onLeave(ret) {
    if (this.restore) this.restore.at.writeU8(this.restore.was);
    if (this.tid !== undefined) {
      Stalker.unfollow(this.tid);
      Stalker.flush();
      const tid = this.tid;
      setTimeout(() => {
        const calls = globalThis.__spawnCalls || [];
        send({ family: 'summon-sign', tag: 'phantom.spawn_calls', count: calls.length, tail: calls.slice(-60), head: calls.slice(0, 40) });
        Stalker.garbageCollect();
      }, 500);
    }
  },
});
// Last gate (measured): 0x140359530(chrMgr, chrId) at 0x14041616d is false because chr 744009 is
// not in the loaded list. FORCE_SPAWN answers true for this call, to see whether a player-shaped
// NPC spawns without it.
Interceptor.attach(at(0x00359530), {
  onEnter() { this.on = globalThis.__summoning === true && this.returnAddress.equals(at(0x0041616d)); },
  onLeave(ret) {
    if (!this.on) return;
    send({ family: 'summon-sign', tag: 'phantom.loaded_gate', native: ret.toInt32() & 0xff });
    if (FORCE_SPAWN && (ret.toInt32() & 0xff) === 0) ret.replace(ptr(1));
  },
});
// Then the row's two event flags (desc +4 = 810010, +8 = 744000 for Pate) through 0x140422bc0 at
// 0x14041618f / 0x1404161a1; measured: 810010 is off. FORCE_SPAWN answers true for both.
Interceptor.attach(at(0x00422bc0), {
  onEnter(args) {
    this.on = globalThis.__summoning === true && (this.returnAddress.equals(at(0x0041618f)) || this.returnAddress.equals(at(0x004161a1)));
    this.flag = args[1].toInt32();
  },
  onLeave(ret) {
    if (!this.on) return;
    send({ family: 'summon-sign', tag: 'phantom.flag_gate', flag: this.flag, native: ret.toInt32() & 0xff });
    if (FORCE_SPAWN && (ret.toInt32() & 0xff) === 0) ret.replace(ptr(1));
  },
});
// The touch, done by the agent: what NetNpcPhantomDirection_HOST (0x1402529c0) calls.
const SUMMON_TEST = false;
const SPAWN_TEAM = null;
// 0x2824 = Mild-Mannered Pate (measured: the sign named him; spawned 2026-11-02 at his generator,
// 210 m away). 0x2c24 = block 2 index 11, chrType 2 / sign type 2 (red): chr 891000, spawned
// 2026-11-02 at (90.4, -40.4, -148.4) despite gen+0x30 set beside the player. 0x3024 = index 12.
const SUMMON_ENTITY = 0x2824;
// Pate's generator already holds the character the forced spawn made at his MSB point, and
// FUN_140418d50 returns that one instead of spawning. 0x1404193f0(mgr, entry) is the native despawn
// FUN_14041abf0's eviction uses: it removes the entry's character and decrements mgr+0x368.
const DESPAWN_FIRST = true;
// Measured 2026-11-02: Pate spawned from his kind-1 entity (chrType 1) is an ally the player cannot
// hurt. FUN_140418690 puts FUN_1404124f0(gen)'s chrType into the spawn descriptor; the chrType
// table 0x1410c0050 pairs row 1 (02 01 01 02 02 01 ..) with row 2 (02 02 01 02 02 00 ..), the red
// sign type. Return SPAWN_CHR_TYPE from that one call, inside FUN_140418690 only.
Interceptor.attach(at(0x004124f0), {
  onEnter() {
    const r = this.returnAddress.sub(image.base).toInt32();
    this.on = globalThis.__summoning === true && r > 0x418690 && r < 0x418d50;
    this.r = r;
  },
  onLeave(ret) {
    if (!this.on) return;
    const SPAWN_CHR_TYPE = summonMode.spawnChrType;
    send({ family: 'summon-sign', tag: 'phantom.chr_type', mode: summonMode.name, from: '0x' + this.r.toString(16), native: ret.toInt32() & 0xff, forced: SPAWN_CHR_TYPE });
    if (SPAWN_CHR_TYPE !== null) ret.replace(ptr(SPAWN_CHR_TYPE));
  },
});
const despawn = new NativeFunction(at(0x004193f0), 'void', ['pointer', 'pointer']);
const grabChr = new NativeFunction(at(0x0014b420), 'pointer', ['pointer']); // getGrabCharacterCtrl(&handle)
const generate = new NativeFunction(at(0x00415b20), 'pointer', ['pointer', 'pointer', 'uint8']);
globalThis.__summonId = Memory.alloc(8);
let summoned = false;
// Pate's character (744009) is not loaded (0x140350fd0). 0x140412970(gen, flag) is the native
// "prepare generator": it requests the spawn row's chr id through the acquire 0x140359760 and
// registers its event flags. Call it once, poll the loaded check 0x140359530([GameManagerImp+0x18],
// id) every frame, and summon only once it says loaded (or after ~10 s, to see that it never did).
const prepareGenerator = new NativeFunction(at(0x00412970), 'void', ['pointer', 'uint8']);
const chrLoaded = new NativeFunction(at(0x00359530), 'uint8', ['pointer', 'uint32']);
let prepStage = 0;
let prepFrames = 0;
function prepareThenSummon() {
  if (!SUMMON_TEST || summoned || !granted) return;
  const gm = GAME_MANAGER_IMP.readPointer();
  const resolve = new NativeFunction(at(0x0017b7e0), 'pointer', ['pointer']);
  globalThis.__summonId.writeU32(SUMMON_ENTITY);
  const ctrl = resolve(globalThis.__summonId);
  if (ctrl.isNull()) return;
  const chrId = ctrl.add(0x60).readPointer().readU32();
  // The player took chr 866001 (entity 0x3024, PlayerCtrl, following and fighting for the player)
  // for Pate. Despawn it too, so only one NPC is near the player when the result is judged.
  if (DESPAWN_FIRST && prepStage === 0) {
    globalThis.__otherId = globalThis.__otherId || Memory.alloc(8);
    globalThis.__otherId.writeU32(0x3024);
    const other = resolve(globalThis.__otherId);
    const oe = other.isNull() ? NULL : other.add(0x68).readPointer();
    if (!oe.isNull() && !grabChr(oe.add(0x18)).isNull()) {
      despawn(gm.add(0x40).readPointer(), oe);
      send({ family: 'summon-sign', tag: 'summon_test.despawn_other', entity: '0x3024', held: grabChr(oe.add(0x18)).toString() });
    }
  }
  if (DESPAWN_FIRST && prepStage === 0) {
    const entry = ctrl.add(0x68).readPointer();
    const handle = entry.isNull() ? -1 : entry.add(0x18).readU32();
    const held = entry.isNull() ? NULL : grabChr(entry.add(0x18));
    send({ family: 'summon-sign', tag: 'summon_test.despawn', entry: entry.toString(), handle: '0x' + (handle >>> 0).toString(16), held: held.toString() });
    if (!held.isNull()) {
      despawn(gm.add(0x40).readPointer(), entry);
      send({ family: 'summon-sign', tag: 'summon_test.despawn.after', held: grabChr(entry.add(0x18)).toString(),
        count: gm.add(0x40).readPointer().add(0x368).readS32() });
    }
    prepStage = -1;
    return;
  }
  if (prepStage === -1) prepStage = 0;
  if (prepStage === 0) {
    prepStage = 1;
    send({ family: 'summon-sign', tag: 'summon_test.prepare', chrId, loaded_before: chrLoaded(gm.add(0x18).readPointer(), chrId) });
    prepareGenerator(ctrl, 0);
    return;
  }
  prepFrames += 1;
  const loaded = chrLoaded(gm.add(0x18).readPointer(), chrId);
  if (!loaded && prepFrames < 600) return;
  send({ family: 'summon-sign', tag: 'summon_test.loaded', chrId, loaded, frames: prepFrames });
  summonTest();
}
// Where a spawned NPC is: 0x14041e870(handle) -> CharacterCtrl (FUN_140415b20 uses it on the
// spawn's handle), position at +0x90 as for the player. Reported once per load.
const WATCH_SPAWN_HANDLE = 0x3021002; // chr 866001 (entry 0x3024), spawned 2026-11-02; Pate was 0x2820e02
const MOVE_SPAWN_TO_PLAYER = false;
function watchMoved() {
  const m = globalThis.__movedChr;
  if (!m) return;
  // During a load (death, warp) PlayerCtrl is null and the spawned character is freed: stop
  // watching instead of faulting every frame (measured 2026-11-02: 0x168 access violations).
  if (GAME_MANAGER_IMP.readPointer().add(PLAYER_CTRL_OFFSET).readPointer().isNull()) {
    send({ family: 'summon-sign', tag: 'spawn.watch_end', frames: m.frames, why: 'no PlayerCtrl (load)' });
    globalThis.__movedChr = undefined;
    return;
  }
  m.frames += 1;
  if ([1, 5, 30, 120, 300, 600].includes(m.frames)) {
    const q = m.chr.add(0x90);
    const me = GAME_MANAGER_IMP.readPointer().add(PLAYER_CTRL_OFFSET).readPointer().add(0x90);
    const his = [q.readFloat(), q.add(4).readFloat(), q.add(8).readFloat()];
    const d = Math.hypot(his[0] - me.readFloat(), his[1] - me.add(4).readFloat(), his[2] - me.add(8).readFloat());
    send({ family: 'summon-sign', tag: 'spawn.moved', frames: m.frames, vtable: rel(m.chr.readPointer()), at: his.map((v) => v.toFixed(2)),
      distance: d.toFixed(1), hp: m.chr.add(0x168).readS32() });
  }
  // Damage semaphore: any change of his HP after spawn, so a hit is measured, not reported.
  const hp = m.chr.add(0x168).readS32();
  if (m.hp !== undefined && hp !== m.hp) send({ family: 'summon-sign', tag: 'spawn.hp', frames: m.frames, was: m.hp, now: hp });
  m.hp = hp;
  // The player's HP too, so Pate hitting the player is measured as well.
  const php = GAME_MANAGER_IMP.readPointer().add(PLAYER_CTRL_OFFSET).readPointer().add(0x168).readS32();
  if (m.php !== undefined && php !== m.php) send({ family: 'summon-sign', tag: 'player.hp', frames: m.frames, was: m.php, now: php });
  m.php = php;
}
const ctrlOfHandle = new NativeFunction(at(0x0041e870), 'pointer', ['uint32']);
let spawnReported = false;
function reportSpawn() {
  if (spawnReported || !granted) return;
  spawnReported = true;
  const chr = ctrlOfHandle(WATCH_SPAWN_HANDLE);
  const me = GAME_MANAGER_IMP.readPointer().add(PLAYER_CTRL_OFFSET).readPointer().add(0x90);
  const p = (q) => [q.readFloat(), q.add(4).readFloat(), q.add(8).readFloat()];
  const mine = p(me);
  if (chr.isNull()) {
    send({ family: 'summon-sign', tag: 'spawn.where', handle: '0x' + WATCH_SPAWN_HANDLE.toString(16), chr: null, player: mine.map((v) => v.toFixed(2)) });
    return;
  }
  const his = p(chr.add(0x90));
  const d = Math.hypot(his[0] - mine[0], his[1] - mine[1], his[2] - mine[2]);
  // Measured 2026-11-02: he spawned at his generator's point, 210.5 m away. Try the cheapest move:
  // write the player's position + 2 m on x into his +0x90, then re-read it over the next frames to
  // see whether the game keeps it or overwrites it from physics.
  if (MOVE_SPAWN_TO_PLAYER) {
    chr.add(0x90).writeFloat(mine[0] + 2);
    chr.add(0x94).writeFloat(mine[1]);
    chr.add(0x98).writeFloat(mine[2]);
    globalThis.__movedChr = { chr, frames: 0 };
  }
  send({ family: 'summon-sign', tag: 'spawn.where', handle: '0x' + WATCH_SPAWN_HANDLE.toString(16), chr: chr.toString(),
    vtable: rel(chr.readPointer()), at: his.map((v) => v.toFixed(2)), player: mine.map((v) => v.toFixed(2)), distance: d.toFixed(1),
    hp: chr.add(0x168).readS32() + '/' + chr.add(0x170).readS32() });
}
function summonTest() {
  if (!SUMMON_TEST || summoned || !granted) return;
  summoned = true;
  const mgr = GAME_MANAGER_IMP.readPointer().add(0x40).readPointer();
  if (mgr.isNull()) {
    send({ family: 'summon-sign', tag: 'summon_test.refused', why: 'no manager' });
    return;
  }
  globalThis.__summonId.writeU32(SUMMON_ENTITY);
  // FUN_140418d50 spawns only when the generator entity's +0x8c is 3/4/5/6/8 (0x1404161d0's jump
  // table); the touch got -1 with it in a refusing state. Log it and set 3 for this attempt.
  const resolve = new NativeFunction(at(0x0017b7e0), 'pointer', ['pointer']);
  const ctrl = resolve(globalThis.__summonId);
  if (!ctrl.isNull()) {
    const state = ctrl.add(0x8c).readU8();
    send({ family: 'summon-sign', tag: 'summon_test.state', ctrl: ctrl.toString(), state, b79: ctrl.add(0x79).readU8(), b7a: ctrl.add(0x7a).readU8() });
    if (![3, 4, 5, 6, 8].includes(state)) ctrl.add(0x8c).writeU8(3);
    // FUN_140415b20 passes gen+0x20 as the position (it is the 5th argument to FUN_140418d50 and
    // r9 of FUN_140418690; FUN_14040f9c0 stores the same +0x20 as the entry's position at +0x1c)
    // unless (gen+0x8e & 0xf0) == 0x30. Put it 3 m from the player.
    const me = GAME_MANAGER_IMP.readPointer().add(PLAYER_CTRL_OFFSET).readPointer().add(0x90);
    const before = [0, 4, 8].map((o) => ctrl.add(0x20 + o).readFloat().toFixed(2));
    ctrl.add(0x20).writeFloat(me.readFloat() + 3);
    ctrl.add(0x24).writeFloat(me.add(4).readFloat());
    ctrl.add(0x28).writeFloat(me.add(8).readFloat());
    send({ family: 'summon-sign', tag: 'summon_test.position', b8e: '0x' + ctrl.add(0x8e).readU8().toString(16), before,
      after: [0, 4, 8].map((o) => ctrl.add(0x20 + o).readFloat().toFixed(2)) });
  }
  globalThis.__summoning = true;
  const ret = generate(mgr, globalThis.__summonId, 1);
  globalThis.__summoning = false;
  if (globalThis.__b302) {
    globalThis.__b302.at.writeU8(globalThis.__b302.was);
    globalThis.__b302 = undefined;
  }
  send({ family: 'summon-sign', tag: 'summon_test', entity: '0x' + SUMMON_ENTITY.toString(16), ret: ret.toString(),
    at: ret.isNull() ? null : [0, 4, 8].map((o) => ret.add(0x90 + o).readFloat().toFixed(2)) });
  if (!ret.isNull()) {
    globalThis.__movedChr = { chr: ret, frames: 0 };
    // +0x30 is the team byte (1 player side, 2 hostile; measured by diffing live CharacterCtrls).
    if (SPAWN_TEAM !== null) {
      const was = ret.add(0x30).readU8();
      ret.add(0x30).writeU8(SPAWN_TEAM);
      send({ family: 'summon-sign', tag: 'summon_test.team', was, now: ret.add(0x30).readU8() });
    }
  }
}
// Read-only, once: every generator entity with its spawn row and the character it holds now, to
// find the entity whose spawn is an NPC PHANTOM. Measured 2026-11-02: Pate's 0x2824 (kind 1) gave a
// CharacterCtrl at his standing spot; 0x3024 (kind 3, chr 866001) gave a PlayerCtrl.
const ENTITY_DUMP = false;
let entityDumped = false;
const PATE_TEAM = null;
globalThis.__pateId = Memory.alloc(8);
function entityDump() {
  if (!ENTITY_DUMP || entityDumped || !granted) return;
  entityDumped = true;
  const rows = [];
  for (const c of listCharacters()) {
    if (c.error) continue;
    const r = { e: '0x' + c.entity.toString(16), kind: c.kind, t: c.chrType, s: c.chrType < 0x14 ? signTypeOf(c.chrType) : null,
      st: c.ctrl.add(0x8c).readU8(), b8e: c.ctrl.add(0x8e).readU8() };
    try {
      const desc = c.ctrl.add(0x60).readPointer();
      r.chr = desc.readU32();
      r.f = [desc.add(4).readU32(), desc.add(8).readU32()];
      r.mask = '0x' + desc.add(0xe).readU16().toString(16);
    } catch (e) { r.desc = e.message; }
    r.at = [0, 4, 8].map((o) => c.ctrl.add(0x20 + o).readFloat().toFixed(1));
    // The generator param row ([ctrl+0x58]): +0x84 is the kind; FUN_140418690 copies +0x85 and +8
    // into the spawn descriptor (+0x3d, +0x3c) beside it. Candidate team type.
    const row = c.ctrl.add(0x58).readPointer();
    r.row80 = Array.from(new Uint8Array(row.add(0x80).readByteArray(0x10))).map((x) => x.toString(16).padStart(2, '0')).join(' ');
    r.row00 = Array.from(new Uint8Array(row.readByteArray(0x10))).map((x) => x.toString(16).padStart(2, '0')).join(' ');
    try {
      const gen = c.ctrl.add(0x68).readPointer();
      if (!gen.isNull()) {
        const h = gen.add(0x18).readU32();
        r.h = '0x' + h.toString(16);
        const chr = grabChr(gen.add(0x18));
        if (!chr.isNull()) {
          r.vt = rel(chr.readPointer());
          r.bytes = Array.from(new Uint8Array(chr.readByteArray(0x600))).map((x) => x.toString(16).padStart(2, '0')).join('');
          // [chr+0xb0]+0x3c is the phantom type FUN_14014ed00 reads (and TeamTypeArray indexes).
          const cd = chr.add(0xb0).readPointer();
          if (!cd.isNull()) r.cd = Array.from(new Uint8Array(cd.readByteArray(0x100))).map((x) => x.toString(16).padStart(2, '0')).join('');
        }
      }
    } catch (e) { r.gen = e.message; }
    rows.push(r);
  }
  // Measured 2026-11-02 by diffing live CharacterCtrls: byte +0x30 is 1 on the player and on the
  // friendly Pate, 2 on all 13 hostile enemies. Candidate team type: write PATE_TEAM into Pate's.
  if (PATE_TEAM !== null) {
    const pate = rows.find((r) => r.e === '0x2824');
    globalThis.__pateId.writeU32(0x2824);
    const gen = new NativeFunction(at(0x0017b7e0), 'pointer', ['pointer'])(globalThis.__pateId).add(0x68).readPointer();
    const chr = grabChr(gen.add(0x18));
    if (!chr.isNull()) {
      globalThis.__movedChr = { chr, frames: 0 };
      const was = chr.add(0x30).readU32();
      chr.add(0x30).writeU8(PATE_TEAM);
      send({ family: 'summon-sign', tag: 'pate.team', chr: chr.toString(), was: '0x' + was.toString(16), now: chr.add(0x30).readU32(), found: !!pate });
    }
  }
  const pl = GAME_MANAGER_IMP.readPointer().add(PLAYER_CTRL_OFFSET).readPointer();
  send({ family: 'summon-sign', tag: 'entity.dump.player', vt: rel(pl.readPointer()),
    bytes: Array.from(new Uint8Array(pl.readByteArray(0x600))).map((x) => x.toString(16).padStart(2, '0')).join('') });
  for (let k = 0; k < rows.length; k += 8)
    send({ family: 'summon-sign', tag: 'entity.dump', from: k, total: rows.length, rows: rows.slice(k, k + 8) });
}
// THE PLAYER'S TOUCH: when the player touches the NPC sign, NetNpcPhantomDirection_HOST calls
// FUN_140415b20 for the sign's entity. Do for that call what summonTest does for its own: free the
// generator, allow its state, put the spawn 3 m from the player (gen+0x20), and mark the call as a
// summon so the forced gates and SPAWN_CHR_TYPE (16 = hostile invader, measured) apply to it.
const TOUCH_SUMMON = true;
function touchSummonEnter(ctx, args) {
  if (!TOUCH_SUMMON || globalThis.__summoning === true) return;
  const entity = args[1].readU32();
  if (entity !== SUMMON_ENTITY) return;
  const gm = GAME_MANAGER_IMP.readPointer();
  const ctrl = new NativeFunction(at(0x0017b7e0), 'pointer', ['pointer'])(args[1]);
  if (ctrl.isNull()) return;
  const entry = ctrl.add(0x68).readPointer();
  if (!entry.isNull() && !grabChr(entry.add(0x18)).isNull()) despawn(gm.add(0x40).readPointer(), entry);
  const state = ctrl.add(0x8c).readU8();
  if (![3, 4, 5, 6, 8].includes(state)) ctrl.add(0x8c).writeU8(3);
  const me = gm.add(PLAYER_CTRL_OFFSET).readPointer().add(0x90);
  ctrl.add(0x20).writeFloat(me.readFloat() + 3);
  ctrl.add(0x24).writeFloat(me.add(4).readFloat());
  ctrl.add(0x28).writeFloat(me.add(8).readFloat());
  ctx.touch = true;
  globalThis.__summoning = true;
  globalThis.__auraUntil = Date.now() + 10000;
  send({ family: 'summon-sign', tag: 'touch_summon', entity: '0x' + entity.toString(16), state, loaded: chrLoaded(gm.add(0x18).readPointer(), ctrl.add(0x60).readPointer().readU32()) });
}
// THE AURA (user 2026-11-02: blue, with his phantom type 16). ChrNetworkPhantomParam row 1 (white
// NPC) carries SpEffect 100015, rows 5/6/16 (red) 100020 -- so the glow may be an applied SpEffect.
// Log every applySpEffect (0x14014bec0(ctrl, struct)) and addSpEffectToList (0x1402206d0) for ~10 s
// after a touch, with the struct's first words.
for (const [name, rva] of [['apply_speffect', 0x0014bec0], ['add_speffect', 0x002206d0]]) {
  Interceptor.attach(at(rva), {
    onEnter(args) {
      if (!globalThis.__auraUntil || Date.now() > globalThis.__auraUntil) return;
      let words = null;
      try { words = [0, 4, 8, 12].map((o) => args[1].add(o).readS32()); } catch (e) { words = e.message; }
      send({ family: 'summon-sign', tag: 'aura.' + name, ctrl: args[0].toString(), words, from: rel(this.returnAddress) });
    },
  });
}
function touchSummonLeave(ctx, ret) {
  // ChrNetworkPhantomParam row 16's chr_phantom_param_id (red) in hostile mode. Measured
  // 2026-11-02: written after the spawn it changed nothing (still blue).
  const AURA_PHANTOM_PARAM = summonMode.aura;
  globalThis.__auraUntil = Date.now() + 10000;
  if (!ctx.touch) return;
  globalThis.__summoning = false;
  if (globalThis.__b302) {
    globalThis.__b302.at.writeU8(globalThis.__b302.was);
    globalThis.__b302 = undefined;
  }
  // The aura: ChrNetworkPhantomParam is looked up by [chr+0xb0]+0x3c (0x14016f510); row 16 names
  // the red-invader ChrPhantomParam rows (3; 4, 5, 28), row 1 the white NPC one (2). He came in
  // blue with chrType 16 forced at spawn (user, 2026-11-02): read what his phantom type really is.
  let phantomType = null;
  let spCtrl = null;
  try {
    if (!ret.isNull()) {
      phantomType = ret.add(0xb0).readPointer().add(0x3c).readU8();
      // His ChrSpEffectCtrl, as FUN_140418690 gets it (vtable +0x130), to match aura.* lines.
      spCtrl = new NativeFunction(ret.readPointer().add(0x130).readPointer(), 'pointer', ['pointer'])(ret).toString();
      // No SpEffect reached him (measured), so the glow is not one. setCharacterAppearence
      // (0x140202e40) writes ChrNetworkPhantomParam's chr_phantom_param_id (row 16: 3; rows 1/2:
      // 2/1) into character_data+0x38. Read it as int and float, then put the red row's id there.
      const cd = ret.add(0xb0).readPointer();
      globalThis.__auraCd = { u32: cd.add(0x38).readU32(), f32: cd.add(0x38).readFloat() };
      if (AURA_PHANTOM_PARAM !== null) {
        if (globalThis.__auraCd.f32 > 0.5 && globalThis.__auraCd.f32 < 1000) cd.add(0x38).writeFloat(AURA_PHANTOM_PARAM);
        else cd.add(0x38).writeU32(AURA_PHANTOM_PARAM);
      }
      globalThis.__auraCd.now = { u32: cd.add(0x38).readU32(), f32: cd.add(0x38).readFloat() };
    }
  } catch (e) { phantomType = e.message; }
  send({ family: 'summon-sign', tag: 'touch_summon.ret', ret: ret.toString(), phantomType, spCtrl, cd38: globalThis.__auraCd });
  // Copy it: the onLeave return value reads as null after the callback (measured: 0x168 faults).
  if (!ret.isNull()) globalThis.__movedChr = { chr: ptr(ret.toString()), frames: 0 };
}
let lastSignWords = '';
function watchSign() {
  const s = globalThis.__watchSign;
  if (s === undefined) return;
  const words = '0x' + (s.add(0x14).readU32() >>> 0).toString(16) + ' 0x' + (s.add(0x18).readU32() >>> 0).toString(16);
  if (words === lastSignWords) return;
  lastSignWords = words;
  send({ family: 'summon-sign', tag: 'sign.words', words });
}
{
}
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
// THE TRIGGER: the inventory's Use queues 0x140500c40(this, u16 handle, u16 count) whatever the
// item. When the handle resolves to a soapstone in SUMMON_MODES, that soapstone picks the mode and
// the next nav frame places Pate's sign under the player (npcSignTest).
Interceptor.attach(USE_ITEM, {
  onEnter(args) {
    const handle = args[1].toInt32() & 0xffff;
    let item;
    try {
      const x = GAME_MANAGER_IMP.readPointer().add(GAME_DATA_MANAGER_OFFSET).readPointer().add(ITEM_INVENTORY_OFFSET).readPointer().add(0x10).readPointer();
      item = itemOfHandle(x, handle);
    } catch (e) {
      item = e.message;
    }
    const picked = SUMMON_MODES[item];
    send({ family: 'summon-sign', tag: 'use.soapstone', handle: '0x' + handle.toString(16), item, mode: picked ? picked.name : null });
    if (!picked) return;
    summonMode = picked;
    signTested = false;
  },
});
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
    watchSlot();
  } catch (e) {
    send({ family: 'summon-sign', tag: 'phantom.slot_watch.error', error: e.message });
    globalThis.__npcSlot = undefined;
  }
  try {
    watchSign();
  } catch (e) {
    send({ family: 'summon-sign', tag: 'sign.words.error', error: e.message });
  }
  try {
    prepareThenSummon();
    reportSpawn();
    watchMoved();
  } catch (e) {
    send({ family: 'summon-sign', tag: 'summon_test.refused', why: 'threw ' + e.message });
  }
  try {
    npcSignTest();
  } catch (e) {
    send({ family: 'summon-sign', tag: 'npc_sign_test.refused', why: 'threw ' + e.message, stage: globalThis.__signStage });
  }
  try {
    entityDump();
  } catch (e) {
    send({ family: 'summon-sign', tag: 'entity.dump.error', error: e.message });
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
