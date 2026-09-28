// Does two-handing change the STR the game scales attack rating from? Read-only.
//
// 0x14038d790 builds the per-stat attack bonus block (docs/DS2-DPS-MECHANICS.md, "Attack rating:
// stat scaling"): rdx is the stat block it reads (11 u16 words, STR = id 5 -> +0x08, DEX -> +0x0a),
// rcx the output, where the STR bonus (PhysicalStatsPerLevelStatValuesParam.physicalAttackByStrength
// of the clamped STR) lands at +0x24 (store at 0x14038da44) and DEX's at +0x28.
//
// Every distinct (caller, STR word, DEX word, STR bonus, DEX bonus) is sent once with a count, then
// a summary every two seconds. Toggle the grip (Y) while it runs: if two-handing raises the STR the
// builder is handed, a new tuple with STR ~1.5x appears on the same caller.

'use strict';

const BUILDER = Process.getModuleByName('DarkSoulsII.exe').base.add(0x0038d790);
const base = Process.getModuleByName('DarkSoulsII.exe').base;
const seen = {};
let calls = 0;

Interceptor.attach(BUILDER, {
  onEnter(args) {
    this.out = args[0];
    this.block = args[1];
    this.caller = this.returnAddress.sub(base);
  },
  onLeave() {
    calls += 1;
    const str = this.block.add(0x08).readU16();
    const dex = this.block.add(0x0a).readU16();
    const strBonus = this.out.add(0x24).readS32();
    const dexBonus = this.out.add(0x28).readS32();
    const key = 'caller=0x' + this.caller.toString(16) + ' STR=' + str + ' DEX=' + dex +
      ' strBonus=' + strBonus + ' dexBonus=' + dexBonus;
    if (!(key in seen)) {
      seen[key] = 0;
      send({ kind: 'new', key, at_call: calls });
    }
    seen[key] += 1;
  },
});

// The last block and output the builder was handed, re-read between frames: a grip toggle that
// changes STR without re-running the builder still shows up here.
let lastOut = null;
let lastBlock = null;
Interceptor.attach(BUILDER, {
  onEnter(args) {
    lastOut = args[0];
    lastBlock = args[1];
  },
});
let lastSample = '';
setInterval(() => {
  if (lastOut === null) return;
  const sample = 'block STR=' + lastBlock.add(0x08).readU16() + ' out strBonus=' +
    lastOut.add(0x24).readS32();
  if (sample !== lastSample) {
    lastSample = sample;
    send({ kind: 'sample', sample });
  }
}, 100);

setInterval(() => send({ kind: 'summary', calls, seen }), 2000);
send({ kind: 'attached', builder: BUILDER.toString() });
