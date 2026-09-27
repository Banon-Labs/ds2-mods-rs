// Estus sync prototype (bd ds2-mods-rs-w2im): raise MY Estus uses level (property 0, Estus Flask
// Shards) and effect level (property 1, Sublime Bone Dust) to the highest of any player in the
// session, each axis on its own, and only ever up.
//
// `python3 scripts/ds2-frida-up.py`, then attach a FROZEN COPY (bd ds2-frida-hot-reload-kills-game):
//   cp -f scripts/frida/estus-sync.js $SCRATCH/estus-sync-frozen.js
//   uv run --with frida python3 scripts/ds2-frida-watch.py --agent $SCRATCH/estus-sync-frozen.js
//
// Control file, re-read once a second: <Game>/estus-sync.json. Every key is optional.
//   {"apply": true}                       actually call ESTUS_SET_PROPERTY (default false: observe)
//   {"test_other": {"uses": 7, "effect": 3}}
//                                          a FAKE other player, logged as TEST, for a solo proof
//   {"carry": true}                       send my levels to the session as P2P packet 0x41 and read
//                                          theirs out of the 0x41 packets that arrive (see CARRY)
//   {"restore": true}                     put both levels back to what they were at attach
//
// WHY A CARRY AT ALL: a remote player's Estus levels are not in this process. Static evidence
// (darksoulsii-deobf.bin), recorded in bd ds2-mods-rs-w2im:
//   * The levels are two bytes on the flask's ItemInventory2 entry (+0x25 uses, +0x26 effect). There
//     is one ItemInventory2, GameManagerImp+0xa8 -> +0x10, and it is ours.
//   * The setter core 0x1401ae7c0 writes entry+0x25+prop and the save record [list+0x48]+i*0x10
//     (+0xa charges, +0xb/+0xc levels) and returns. No notifier, no packet. Compare the weapon path,
//     0x14037f890, which ends in packet 61 (0x140162c50).
//   * A remote PlayerCtrl's equipment comes from two places only. The join snapshot 0x14035a400 ->
//     0x140154a30 copies 46 ITEM IDS (+0x1c..+0xd0) into the 0x34 records and writes every record's
//     level byte as the constant 0. After that, the ChrEquip receiver 0x140162150 updates records
//     only for 0x3d weapon (level <= 10), 0x3e armour, 0x3f ring. Nothing carries a quick item's or
//     the flask's level.
// This agent also READS the record tables (mine and each remote's) for the flask id 60155000 and
// prints the four bytes at +0x0c, so the static claim is checked against the running game.
//
// CARRY: packet id 0x41. The receiver 0x140162150 claims ids 0x3a..0x42 and its jump table sends
// 0x41 straight to the common exit 0x140162748, and no sender of 0x41 was found in the image
// (`mov dl,0x41` before a `call [reg+0x78]`). So a peer without this agent drops it, and a peer with
// it sees it here first. Send path: the one packet 61 uses, session = [[0x141616cf8]],
// vtable[+0x78](session, u8 id, const void* buf, u32 len). Payload, 8 bytes:
//   'E' 'S' version(1) uses effect 0 0 0
'use strict';

const image = Process.getModuleByName('DarkSoulsII.exe');
const at = (rva) => image.base.add(rva);

// ds2-rva names, same values.
const GAME_MANAGER_IMP = at(0x016148f0);
const GAME_DATA_MANAGER_OFFSET = 0xa8;
const ITEM_INVENTORY_OFFSET = 0x10;
const GAME_MANAGER_PLAYER_CTRL_OFFSET = 0xd0;
const GAME_MANAGER_CHARACTER_MANAGER_OFFSET = 0x18;
const CHARACTER_MANAGER_ENTITY_BEGIN_OFFSET = 0x10;
const CHARACTER_MANAGER_ENTITY_END_OFFSET = 0x18;
const PLAYER_CTRL_VTABLE = at(0x010e4bb8);
const CHARACTER_CTRL_NAME_OFFSET = 0x118;
const CHARACTER_CTRL_PHANTOM_BLOCK_OFFSET = 0xb0;
const PHANTOM_BLOCK_PHANTOM_PARAM_OFFSET = 0x3c;
const REPLAY_PHANTOM_PARAM_IDS = [0x12, 0x13];
const CHARACTER_CTRL_CHR_ASM_CTRL_OFFSET = 0x378;
const CHR_ASM_CTRL_RECORD_TABLE_OFFSET = 0x20;
const EQUIP_RECORD_ARRAY_OFFSET = 0x08;
const EQUIP_RECORD_STRIDE = 0x14;
const EQUIP_RECORD_COUNT = 0x34;
const EQUIP_RECORD_ITEM_OFFSET = 0x04;
const ESTUS_FLASK_ITEM_ID = 60155000;

const ESTUS_SET_PROPERTY = at(0x001ac5a0);
const ESTUS_GET_LEVEL = at(0x001abc40);
const ESTUS_GET_CHARGES = at(0x001abc60);
const ESTUS_IS_MAX = at(0x001ac1d0);
const ESTUS_PROPERTY_INDEX = at(0x001ad140);
const ESTUS_THUNK_PROLOGUE = [0x48, 0x8b, 0x49, 0x10, 0xe9];
const ESTUS_PROPERTY_INDEX_PROLOGUE = [0x4c, 0x8d, 0x05, 0xe9, 0xde, 0x3b, 0x01];
const ESTUS_PROPERTY_NOT_FOUND = 0xff;
const ESTUS_PROPERTY_USES = 0;
const ESTUS_PROPERTY_EFFECT = 1;

const NAV_UPDATE = at(0x00baeb20); // NvNavigationSystem::Update, game thread, once a frame
const CHR_EQUIP_RECEIVER = at(0x00162150);
const NET_SESSION_HOLDER = at(0x01616cf8);
const NET_SESSION_SEND_SLOT = 0x78;
const CARRY_PACKET_ID = 0x41;
const CARRY_MAGIC = [0x45, 0x53]; // 'E' 'S'
const CARRY_VERSION = 1;
const CARRY_LEN = 8;

const WORK_EVERY_FRAMES = 30;
const SEND_EVERY_FRAMES = 120;

const TAG = '[estus-sync]';
const say = (t) => { console.log(TAG + ' ' + t); send({ kind: 'log', text: t }); };

const controlPath = image.path.replace(/[^\\\/]*$/, '') + 'estus-sync.json';

function p(base, off) {
  try { const v = base.add(off).readPointer(); return v.isNull() ? null : v; } catch (e) { return null; }
}
function bytesAt(a, n) {
  try { return Array.from(new Uint8Array(a.readByteArray(n))); } catch (e) { return null; }
}
function hex(bs) { return bs === null ? 'unreadable' : bs.map((b) => b.toString(16).padStart(2, '0')).join(' '); }
function same(a, b) { return a !== null && a.length === b.length && a.every((x, i) => x === b[i]); }

function rttiName(obj) {
  try {
    const vt = obj.readPointer();
    const col = vt.sub(8).readPointer();
    const td = image.base.add(col.add(12).readU32());
    return td.add(16).readCString();
  } catch (e) { return '?'; }
}

// ---- prologue checks, once, before anything is called ------------------------------------------
const checks = [
  ['ESTUS_SET_PROPERTY', ESTUS_SET_PROPERTY, ESTUS_THUNK_PROLOGUE],
  ['ESTUS_GET_LEVEL', ESTUS_GET_LEVEL, ESTUS_THUNK_PROLOGUE],
  ['ESTUS_GET_CHARGES', ESTUS_GET_CHARGES, ESTUS_THUNK_PROLOGUE],
  ['ESTUS_IS_MAX', ESTUS_IS_MAX, ESTUS_THUNK_PROLOGUE],
  ['ESTUS_PROPERTY_INDEX', ESTUS_PROPERTY_INDEX, ESTUS_PROPERTY_INDEX_PROLOGUE],
];
let prologuesOk = true;
for (const [name, site, want] of checks) {
  const got = bytesAt(site, want.length);
  const ok = same(got, want);
  if (!ok) prologuesOk = false;
  say('prologue ' + name + ' va=' + site + ' ' + (ok ? 'ok' : 'MISMATCH saw=' + hex(got) + ' want=' + hex(want)));
}

const propertyIndex = new NativeFunction(ESTUS_PROPERTY_INDEX, 'uint8', ['uint32']);
const getLevel = new NativeFunction(ESTUS_GET_LEVEL, 'uint8', ['pointer', 'pointer']);
const getCharges = new NativeFunction(ESTUS_GET_CHARGES, 'uint8', ['pointer']);
const isMax = new NativeFunction(ESTUS_IS_MAX, 'uint8', ['pointer', 'pointer']);
const setProperty = new NativeFunction(ESTUS_SET_PROPERTY, 'uint8', ['pointer', 'pointer', 'int32']);

// The property bytes the setter/getter dereference. They must outlive every call, so they are
// allocated once. Filled from the game's own key->index lookup, which reads no game state.
const propUses = Memory.alloc(8);
const propEffect = Memory.alloc(8);
let indexOk = false;
if (prologuesOk) {
  const iu = propertyIndex(ESTUS_PROPERTY_USES);
  const ie = propertyIndex(ESTUS_PROPERTY_EFFECT);
  indexOk = iu !== ESTUS_PROPERTY_NOT_FOUND && ie !== ESTUS_PROPERTY_NOT_FOUND;
  propUses.writeU8(iu);
  propEffect.writeU8(ie);
  say('property index uses=' + iu + ' effect=' + ie + (indexOk ? '' : ' REFUSED (0xff)'));
}
const callable = prologuesOk && indexOk;
if (!callable) say('NOT ARMED: a prologue or property index check failed; nothing will be called');

// ---- chains -------------------------------------------------------------------------------------
function manager() { return p(GAME_MANAGER_IMP, 0); }
function inventory() {
  const gm = manager();
  const gdm = gm && p(gm, GAME_DATA_MANAGER_OFFSET);
  return gdm && p(gdm, ITEM_INVENTORY_OFFSET);
}
function localPlayer() { const gm = manager(); return gm && p(gm, GAME_MANAGER_PLAYER_CTRL_OFFSET); }

function charName(chr) {
  try {
    const s = chr.add(CHARACTER_CTRL_NAME_OFFSET);
    const len = s.add(0x10).readU64().toNumber();
    const cap = s.add(0x18).readU64().toNumber();
    if (len === 0 || len > 64 || len > cap) return '';
    return (cap > 7 ? s.readPointer() : s).readUtf16String(len);
  } catch (e) { return ''; }
}

function isPerson(chr) {
  if (!charName(chr).startsWith('NetworkPlayer')) return false;
  const block = p(chr, CHARACTER_CTRL_PHANTOM_BLOCK_OFFSET);
  if (!block) return false;
  try { return !REPLAY_PHANTOM_PARAM_IDS.includes(block.add(PHANTOM_BLOCK_PHANTOM_PARAM_OFFSET).readU8()); } catch (e) { return false; }
}

function remotes(local) {
  const out = [];
  const gm = manager();
  const cm = gm && p(gm, GAME_MANAGER_CHARACTER_MANAGER_OFFSET);
  const b = cm && p(cm, CHARACTER_MANAGER_ENTITY_BEGIN_OFFSET);
  const e = cm && p(cm, CHARACTER_MANAGER_ENTITY_END_OFFSET);
  if (!b || !e) return out;
  const n = Math.min(e.sub(b).toInt32() / 8, 512);
  for (let k = 0; k < n; k++) {
    const c = p(b, k * 8);
    if (!c || (local && c.equals(local))) continue;
    let vt; try { vt = c.readPointer(); } catch (x) { continue; }
    if (!vt.equals(PLAYER_CTRL_VTABLE) || !isPerson(c)) continue;
    out.push(c);
  }
  return out;
}

// Every record in a character's equipment table whose item id is the flask: index and +0x0c..+0x0f.
function flaskRecords(chr) {
  const asm = p(chr, CHARACTER_CTRL_CHR_ASM_CTRL_OFFSET);
  const table = asm && p(asm, CHR_ASM_CTRL_RECORD_TABLE_OFFSET);
  if (!table) return 'no-record-table';
  const hits = [];
  for (let r = 0; r < EQUIP_RECORD_COUNT; r++) {
    const rec = table.add(EQUIP_RECORD_ARRAY_OFFSET + r * EQUIP_RECORD_STRIDE);
    let id; try { id = rec.add(EQUIP_RECORD_ITEM_OFFSET).readU32(); } catch (e) { return 'unreadable'; }
    if (id === ESTUS_FLASK_ITEM_ID) hits.push('rec' + r + '{+0c..0f=' + hex(bytesAt(rec.add(0x0c), 4)) + '}');
  }
  return hits.length ? hits.join(' ') : 'flask-not-in-records';
}

// ---- state shared between the timer (JS thread) and the frame hook (game thread) ----------------
const control = { apply: false, test: null, carry: false, restore: false };
const carried = new Map(); // peer key -> {uses, effect, at}
let original = null;       // my levels at the first read
let restored = false;
let lastSeen = '';
let lastPlan = '';

function readControl() {
  let text = '';
  try { text = File.readAllText(controlPath); } catch (e) { text = ''; }
  let c = {};
  try { c = text.trim() ? JSON.parse(text) : {}; } catch (e) { say('control unparseable, ignored: ' + e); return; }
  const next = {
    apply: c.apply === true,
    test: c.test_other && Number.isInteger(c.test_other.uses) && Number.isInteger(c.test_other.effect)
      ? { uses: c.test_other.uses, effect: c.test_other.effect } : null,
    carry: c.carry === true,
    restore: c.restore === true,
  };
  const was = JSON.stringify(control), now = JSON.stringify(next);
  Object.assign(control, next);
  if (was !== now) say('control ' + controlPath + ' ' + now);
  if (control.carry && !receiverHook) armReceiver();
}

// ---- carry: receive -----------------------------------------------------------------------------
let receiverHook = null;
function armReceiver() {
  const got = bytesAt(CHR_EQUIP_RECEIVER, 3);
  if (!same(got, [0x4c, 0x8b, 0xdc])) { say('carry receiver prologue MISMATCH ' + hex(got) + '; not hooked'); return; }
  receiverHook = Interceptor.attach(CHR_EQUIP_RECEIVER, {
    onEnter(args) {
      if ((args[1].toUInt32() & 0xff) !== CARRY_PACKET_ID) return;
      const len = args[3].toUInt32();
      const buf = args[2];
      const peer = args[4];
      const bs = len >= CARRY_LEN ? bytesAt(buf, CARRY_LEN) : null;
      if (!bs || bs[0] !== CARRY_MAGIC[0] || bs[1] !== CARRY_MAGIC[1] || bs[2] !== CARRY_VERSION) {
        say('carry recv id=0x41 not ours len=' + len + ' bytes=' + hex(bs) + ' peer=' + peer);
        return;
      }
      const key = peer.toString();
      const prev = carried.get(key);
      carried.set(key, { uses: bs[3], effect: bs[4], at: Date.now() });
      if (!prev || prev.uses !== bs[3] || prev.effect !== bs[4]) {
        say('carry recv peer=' + key + ' uses=' + bs[3] + ' effect=' + bs[4]);
      }
    },
  });
  say('carry receiver hooked at ' + CHR_EQUIP_RECEIVER + ' (packet id 0x41)');
}

// ---- carry: send (game thread) ------------------------------------------------------------------
const sendBuf = Memory.alloc(16);
let sendFn = null;
let sendLogged = '';
function carrySend(uses, effect) {
  const holder = p(NET_SESSION_HOLDER, 0);
  const session = holder && p(holder, 0);
  if (!session) { if (sendLogged !== 'none') { say('carry send: no net session'); sendLogged = 'none'; } return; }
  const slot = p(session.readPointer(), NET_SESSION_SEND_SLOT);
  if (!slot) return;
  if (sendFn === null || !sendFn.equals(slot)) {
    sendFn = slot;
    say('carry send via session=' + session + ' class=' + rttiName(session) + ' vtable+0x78=' + slot +
      ' (rva 0x' + slot.sub(image.base).toString(16) + ')');
  }
  sendBuf.writeByteArray([CARRY_MAGIC[0], CARRY_MAGIC[1], CARRY_VERSION, uses, effect, 0, 0, 0]);
  const fn = new NativeFunction(slot, 'void', ['pointer', 'uint8', 'pointer', 'uint32']);
  fn(session, CARRY_PACKET_ID, sendBuf, CARRY_LEN);
  const note = uses + '/' + effect;
  if (sendLogged !== note) { say('carry send id=0x41 uses=' + uses + ' effect=' + effect); sendLogged = note; }
}

// ---- the work, on the game thread ---------------------------------------------------------------
let frame = 0;
function work() {
  const inv = inventory();
  const me = localPlayer();
  if (!inv || !me) { const s = 'no character (inventory=' + inv + ' player=' + me + ')'; if (s !== lastSeen) { say(s); lastSeen = s; } return; }

  const mine = { uses: getLevel(inv, propUses), effect: getLevel(inv, propEffect), charges: getCharges(inv) };
  if (mine.uses === 0) { if (lastSeen !== 'noflask') { say('no Estus Flask (level 0)'); lastSeen = 'noflask'; } return; }
  if (original === null) { original = { uses: mine.uses, effect: mine.effect }; say('original mine uses=' + original.uses + ' effect=' + original.effect); }

  if (control.restore && !restored) {
    const bu = setProperty(inv, propUses, original.uses);
    const be = setProperty(inv, propEffect, original.effect);
    say('RESTORE set uses=' + original.uses + ' ret=' + bu + ' effect=' + original.effect + ' ret=' + be +
      ' -> readback uses=' + getLevel(inv, propUses) + ' effect=' + getLevel(inv, propEffect));
    restored = true;
    return;
  }
  if (!control.restore) restored = false;

  const people = remotes(me);
  if (control.carry && people.length > 0 && frame % SEND_EVERY_FRAMES === 0) carrySend(mine.uses, mine.effect);

  // What each other player contributes: carried values, never anything else -- the game holds none.
  const others = [];
  for (const c of people) others.push({ who: charName(c) + '@' + c, source: 'records', flask: flaskRecords(c) });
  for (const [peer, v] of carried) others.push({ who: 'peer ' + peer, source: 'carried', uses: v.uses, effect: v.effect });
  if (control.test) others.push({ who: 'TEST-OTHER-PLAYER', source: 'test', uses: control.test.uses, effect: control.test.effect });

  const seen = 'mine uses=' + mine.uses + ' effect=' + mine.effect + ' charges=' + mine.charges +
    ' myRecords=' + flaskRecords(me) + ' people=' + people.length +
    others.map((o) => '\n  other ' + o.source + ' ' + o.who + (o.source === 'records'
      ? ' estus=NOT-REPLICATED flaskRecords=' + o.flask
      : ' uses=' + o.uses + ' effect=' + o.effect)).join('');
  if (seen !== lastSeen) { say(seen); lastSeen = seen; }

  // Per-axis max over me and every source that carries a number.
  let tu = mine.uses, te = mine.effect, fromU = 'me', fromE = 'me';
  for (const o of others) {
    if (o.source === 'records') continue;
    if (o.uses > tu) { tu = o.uses; fromU = o.who; }
    if (o.effect > te) { te = o.effect; fromE = o.who; }
  }
  const plan = 'max uses=' + tu + ' (from ' + fromU + ') effect=' + te + ' (from ' + fromE + ')' +
    (tu > mine.uses || te > mine.effect ? ' RAISE' : ' nothing to raise') + ' apply=' + control.apply;
  if (plan !== lastPlan) { say(plan); lastPlan = plan; }
  if (!control.apply || !callable) return;

  for (const [axis, prop, target, now] of [['uses', propUses, tu, mine.uses], ['effect', propEffect, te, mine.effect]]) {
    if (target <= now) continue; // only ever up
    const ret = setProperty(inv, prop, target);
    const back = getLevel(inv, prop);
    say('SET ESTUS_SET_PROPERTY(inv=' + inv + ', prop=' + prop.readU8() + ' [' + axis + '], ' + target + ') ret=' + ret +
      ' readback ESTUS_GET_LEVEL=' + back + ' was=' + now + ' atMax=' + isMax(inv, prop) +
      ' charges=' + getCharges(inv) + (back > now ? ' RAISED' : ' NOT RAISED'));
  }
}

const nav = Interceptor.attach(NAV_UPDATE, {
  onEnter() {
    frame++;
    if (frame % WORK_EVERY_FRAMES !== 0) return;
    try { work(); } catch (e) { say('work threw: ' + e.stack); }
  },
});

readControl();
const timer = setInterval(readControl, 1000);
say('attached; control=' + controlPath + ' armed=' + callable + ' thread-work=NvNavigationSystem::Update onEnter');

rpc.exports = {
  dispose() {
    clearInterval(timer);
    nav.detach();
    if (receiverHook) receiverHook.detach();
  },
};
