// The creator opened from the bonfire's Change Appearance row: no Class & gift tab, and the three
// tabs left (Body, Face, Advanced settings) each a third of the bar instead of a quarter.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/chara-tabs-thirds.js`
//
// Arms on openCharaMakerWindow (0x1401986c0), which the DLL's bonfire row calls; a prototype, so it
// does not tell that call apart from any other.
//
// * The tab list: FeGroupCharaMakingTop's builder 0x1400eb270 appends four FexTextTabSpecs with
//   0x1400d2a40. The first append (Class & gift) is not made. The builder unrefs its own copy of
//   the spec's functor after each append, so nothing leaks.
// * The layout: the bind 0x140b00d20(resource, bytes, len) copies l09_01_chara_make.flo (0x765a0)
//   and parses the copy. The bytes it is handed are swapped for a copy with eight transform blocks
//   rewritten: tabs 0x5f5c1c0..1c3 (def 0x00b8) and labels 0x5f5c420..423 (def 0x00ae). Each record
//   is checked (id at rec+0x1c, transform offset at rec+0x08, original floats) before anything is
//   written; one mismatch and the original bytes go through untouched.
//
// Geometry, from scripts/ds2-flo.py over the extracted file: a tab is def 0x00b7, whose art is shape
// 0x00b5, 287.2 wide, drawn 4.65 right of the tab's x (child 0.35 + quad 4.3). The four sit on a 271
// pitch, so neighbours overlap by 16.2 -- that overlap is the divider line. The labels sit 110.5 left
// of their tab's centre.

'use strict';

const exe = Process.getModuleByName('DarkSoulsII.exe').base;
const at = (rva) => exe.add(rva);
const say = (text) => console.log('[chara-tabs] ' + text);

const FLO_LEN = 0x765a0;
const ART_LEFT = 4.65;
const ART_WIDTH = 287.2;
const OVERLAP = ART_WIDTH - 271.0;
const LABEL_FROM_CENTRE = 110.5;
const OFFSCREEN_X = -4000.0;

// The bar the four tabs' art covers, kept for three.
const SPAN_LEFT = 86.4 + ART_LEFT;
const SPAN_RIGHT = 899.6 + ART_LEFT + ART_WIDTH;
const WIDTH3 = (SPAN_RIGHT - SPAN_LEFT + 2 * OVERLAP) / 3;
const SCALE3 = WIDTH3 / ART_WIDTH;
const PITCH3 = WIDTH3 - OVERLAP;
const tabX = (k) => SPAN_LEFT - ART_LEFT * SCALE3 + PITCH3 * k;
const labelX = (k) => tabX(k) + (ART_LEFT + ART_WIDTH / 2) * SCALE3 - LABEL_FROM_CENTRE;

// [record, id, transform, original x, y, new x, new scale x]
const EDITS = [
  [0xa020, 0x5f5c1c0, 0x594d8, 86.4, 67.05, OFFSCREEN_X, 1.0],
  [0xa048, 0x5f5c1c1, 0x59508, 357.4, 67.05, tabX(0), SCALE3],
  [0xa070, 0x5f5c1c2, 0x59538, 628.4, 67.05, tabX(1), SCALE3],
  [0xa098, 0x5f5c1c3, 0x59568, 899.6, 67.05, tabX(2), SCALE3],
  [0x9eb8, 0x5f5c420, 0x584e8, 124.15, 72.35, OFFSCREEN_X, 1.0],
  [0x9e90, 0x5f5c421, 0x584b8, 395.15, 72.35, labelX(0), 1.0],
  [0x9e68, 0x5f5c422, 0x58488, 666.15, 72.35, labelX(1), 1.0],
  [0x9e40, 0x5f5c423, 0x58458, 937.15, 72.35, labelX(2), 1.0],
];

let armed = false;
let inTabs = false;
let skipped = false;
const keep = [];

Interceptor.attach(at(0x1986c0), {
  onEnter() {
    armed = true;
    say('armed by openCharaMakerWindow');
  },
});

function retab(buf) {
  const near = (a, b) => Math.abs(a - b) < 0.01;
  for (const [rec, id, xf, x, y] of EDITS) {
    const gotId = buf.add(rec + 0x1c).readU32();
    const gotXf = buf.add(rec + 0x08).readU64().toNumber();
    const f = [0, 4, 8, 12].map((o) => buf.add(xf + o).readFloat());
    if (gotId !== id || gotXf !== xf || !near(f[0], x) || !near(f[1], y) || !near(f[2], 1) || !near(f[3], 1)) {
      return 'record 0x' + rec.toString(16) + ' id 0x' + gotId.toString(16) + ' xform 0x' + gotXf.toString(16) +
        ' floats ' + f.map((v) => v.toFixed(2)).join(',');
    }
  }
  for (const [, , xf, , , nx, sx] of EDITS) {
    buf.add(xf).writeFloat(nx);
    buf.add(xf + 8).writeFloat(sx);
  }
  return null;
}

Interceptor.attach(at(0xb00d20), {
  onEnter(args) {
    const len = args[2].toInt32();
    this.len = len;
    if (len !== FLO_LEN) return;
    say('chara_make.flo bind, armed=' + armed);
    if (!armed) return;
    const copy = Memory.alloc(len);
    Memory.copy(copy, args[1], len);
    const refused = retab(copy);
    if (refused) {
      say('layout left alone: ' + refused);
      return;
    }
    keep.push(copy);
    args[1] = copy;
    say('layout rewritten: tabs x ' + [0, 1, 2].map((k) => tabX(k).toFixed(2)).join('/') +
      ' scale ' + SCALE3.toFixed(4) + ', labels x ' + [0, 1, 2].map((k) => labelX(k).toFixed(2)).join('/'));
  },
  onLeave(ret) {
    if (this.len === FLO_LEN) say('bind returned ' + (ret.toInt32() & 0xff));
  },
});

Interceptor.attach(at(0xeb270), {
  onEnter() {
    inTabs = armed;
    skipped = false;
  },
  onLeave(ret) {
    if (inTabs) {
      const list = ret;
      const n = list.add(8).readPointer().sub(list.readPointer()).toInt32() / 0x90;
      say('top tabs built: ' + n + ' specs, class & gift skipped=' + skipped);
    }
    inTabs = false;
  },
});

const append = new NativeFunction(at(0xd2a40), 'void', ['pointer', 'pointer']);
Interceptor.replace(at(0xd2a40), new NativeCallback((list, spec) => {
  if (inTabs && !skipped) {
    skipped = true;
    return;
  }
  append(list, spec);
}, 'void', ['pointer', 'pointer']));

say('loaded');
