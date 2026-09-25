// Can the floating damage number over another character be resized without resizing its HP bar?
//
// THE CLAIM UNDER TEST. `FeSceneEnemyHpGuage::update` (RVA 0x66c60) places each of its 20 floating
// gauges with two separate calls, both passed the same scale (render height / 720.0):
//
//   0x663b0  bar  -> 0x66290 -> element 0x113e10 (frame), 0x113e11, 0x113e12 (fills)
//   0x669c0  text -> element 0x5f5b9f2 only (the one init hands an empty wide string)
//
// So multiplying the 4th argument of 0x669c0 alone should grow the number and leave the bar as it
// was. Both functions are replaced here with pass-throughs so the scale each one actually receives
// is logged side by side; only the text call's scale is changed.
//
// The scale is a float in the 4th Win64 slot (xmm3). Frida's CpuContext does not expose xmm
// registers, so this goes through Interceptor.replace with a NativeFunction whose signature puts a
// 'float' there, which is what makes the ABI carry it in xmm3 both ways.
//
// NO TIMER (`frida-js-thread-wedges-in-ds2`). Counters are plain integers; one message per new
// gauge slot seen and one every REPORT_EVERY text calls.

'use strict';

Interceptor.detachAll();

const TEXT_TRANSFORM = 0x669c0; // FeSceneEnemyHpGuage: set transform of element 0x5f5b9f2
const BAR_TRANSFORM = 0x663b0; // FeSceneEnemyHpGuage: bar frame + fills transform
const config = globalThis.__ER_FRIDA_CONFIG || {};
//: How much bigger the number gets than the game would draw it. 2.5 is large enough that nobody
//: has to squint to decide whether it changed.
//:
//: "Normal" on screen is NOT the game's scale argument. The game passes height/720 (2.0 at 1440p)
//: but its own flag-1 setter masks the scale off, so the number it draws is effectively scale
//: 1.0. Once the mask is forced to 7 the argument is honoured, so 2.5x normal is an applied
//: scale of 2.5 -- a factor of 1.25 on the game's 2.0.
//:
//: Every tunable lives in `tune` and is RE-READ LIVE from `hp-text-scale.json` beside
//: DarkSoulsII.exe, so adjusting a number is a file save, not an agent hot reload -- a hot reload
//: on 2026-09-25 hung Frida's transport and only a game restart cleared it. Offsets are in 720p
//: pixels and are multiplied by the game's own height/720 scale, so they are the same share of
//: the screen at any resolution. Screen y grows DOWNWARD.
//:
//:   textFactor  multiplier on the game's scale for the damage number (1.25 -> 2.5x normal)
//:   textLift    how far to raise the number; it scales about its centre, so growing it pushes
//:               its lower half into the bar
//:   barFactor   multiplier on the HP bar's scale (frame 0x113e10, fills 0x113e11/0x113e12)
//:   barDx/barDy where to move the bar's pivot; the bar grows away from its pivot, so a bigger
//:               bar drifts right and needs pulling back left to stay centred on the target
const tune = {
  textFactor: typeof config.textFactor === 'number' ? config.textFactor : 1.25,
  textLift: typeof config.textLift === 'number' ? config.textLift : 12,
  //: Horizontal nudge for the number, negative = left. The layout gives the number no fixed
  //: width (it is 1-5 digits of a font glyph run), so percentages of "its width" are estimates.
  textDx: 0,
  //: Width of one digit of the damage number in layout units, multiplied by the applied text
  //: scale. The text element's pivot is the LEFT edge of its block (seen live 2026-09-25), so
  //: centring it shifts left by half of digits x this. The glyph advance is not in the layout;
  //: 8 is an estimate to be calibrated by eye.
  textDigitAdvance: 8,
  barFactor: typeof config.barFactor === 'number' ? config.barFactor : 2.25,
  barDx: -20,
  barDy: 6,
};
const REPORT_EVERY = 600;
//: Re-read the tuning file every this many bar calls (~20 per second per visible bar).
const TUNE_EVERY = 60;

const module = Process.findModuleByName('DarkSoulsII.exe');
if (module === null) {
  send({ error: 'DarkSoulsII.exe is not loaded' });
} else {
  const textAt = module.base.add(TEXT_TRANSFORM);
  const barAt = module.base.add(BAR_TRANSFORM);
  const sig = ['pointer', 'uint32', 'pointer', 'float', 'pointer'];
  const textOriginal = new NativeFunction(textAt, 'void', sig);
  const barOriginal = new NativeFunction(barAt, 'void', sig);
  // A hot reload does not dispose the previous load, and replacing an already-replaced target
  // throws. Revert both first; reverting one that was never replaced is harmless.
  Interceptor.revert(textAt);
  Interceptor.revert(barAt);

  // SECOND QUESTION, after a live run at scale 50 looked unchanged to the user: does the element's
  // own transform setter (vtable +0x120) keep the scale at all? Resolve the element the same way
  // the game does (FUN_140afda00(scene, 0x5f5b9f2, 0x5f5b9f2, 0, ...)), name its class from RTTI,
  // and snapshot its first ELEMENT_WINDOW bytes either side of the original call so the float
  // words that changed -- and whether any of them is the scale -- can be read off the diff.
  const FIND_ELEMENT = 0xafda00;
  const TEXT_ELEMENT_ID = 0x5f5b9f2;
  const ELEMENT_WINDOW = 0x200;
  const findElement = new NativeFunction(module.base.add(FIND_ELEMENT), 'pointer', [
    'pointer', 'uint32', 'uint32', 'uint32', 'uint64', 'uint64', 'uint64', 'uint64', 'uint64',
    'uint64', 'uint64',
  ]);
  const inspected = new Array(20).fill(false);
  //: Re-submit the text's matrix with the bar's flag (0) so its scale is applied. false = the
  //: game's own behaviour, for an A/B on the same session.
  const FULL_MATRIX = config.fullMatrix !== false;
  //: FeComponentObject's primary vtable (RVA), read live from the text element on 2026-09-25.
  const componentObjectVtable = module.base.add(0x11ddfa8);
  //: 16-byte aligned: the game reads the pivot with MOVSS only, but alignment costs nothing.
  const posScratch = Memory.alloc(0x20).add(0xf).and(ptr('0xfffffffffffffff0'));
  const barPosScratch = Memory.alloc(0x20).add(0xf).and(ptr('0xfffffffffffffff0'));
  //: Width of the bar frame in layout units: shape 0x0006's atlas rect x 153.1..252.7.
  const BAR_WIDTH_UNITS = 99.6;
  const barCenterX = new Array(20).fill(NaN);

  // How many digits each gauge's number shows. FUN_140066190(self, scene, value) is the game's
  // only writer of the number's text: value < 1 clears it, otherwise it is clamped to 99999 and
  // formatted in decimal. It runs only when the number changes, so a map keyed by the slot's
  // scene pointer costs nothing per frame.
  const SET_NUMBER = 0x66190;
  const digitsByScene = {};
  Interceptor.attach(module.base.add(SET_NUMBER), {
    onEnter: function (args) {
      const value = args[2].toInt32();
      digitsByScene[args[1].toString()] = value < 1 ? 0 : String(Math.min(value, 99999)).length;
    },
  });
  const tunePath = module.path.replace(/[^\\\/]*$/, '') + 'hp-text-scale.json';
  let tuneText = null;

  function retune() {
    let text;
    try {
      text = File.readAllText(tunePath);
    } catch (_) {
      return; // no file: keep the current values
    }
    if (text === tuneText) {
      return;
    }
    tuneText = text;
    try {
      const next = JSON.parse(text);
      for (const key of Object.keys(tune)) {
        if (typeof next[key] === 'number' && isFinite(next[key])) {
          tune[key] = next[key];
        }
      }
      send({ event: 'retuned', path: tunePath, tune: tune });
    } catch (e) {
      send({ event: 'retune-failed', path: tunePath, error: String(e) });
    }
  }
  retune();

  function rva(p) {
    return '0x' + p.sub(module.base).toString(16);
  }

  function className(vtable) {
    try {
      const col = vtable.sub(8).readPointer();
      const td = module.base.add(col.add(0xc).readU32());
      return td.add(0x10).readCString();
    } catch (_) {
      return null;
    }
  }

  function floatDiff(before, after) {
    const a = new Float32Array(before);
    const b = new Float32Array(after);
    const out = [];
    for (let i = 0; i < a.length; i += 1) {
      const same = a[i] === b[i] || (a[i] !== a[i] && b[i] !== b[i]);
      if (!same) {
        out.push({ off: '0x' + (i * 4).toString(16), before: a[i], after: b[i] });
      }
    }
    return out;
  }

  let textCalls = 0;
  let barCalls = 0;
  let lastBarScale = 0;
  const slotsSeen = new Array(20).fill(false);

  Interceptor.replace(
    barAt,
    new NativeCallback(
      function (self, slot, pos, scale, offset) {
        barCalls += 1;
        lastBarScale = scale;
        // The bar's own setter passes flag 0 (full matrix), so its scale is honoured as given --
        // unlike the text's, there is no mask to fix.
        if (barCalls % TUNE_EVERY === 0) {
          retune();
        }
        let barPos = pos;
        try {
          Memory.copy(barPosScratch, pos, 16); // the caller's live stack local; always mapped
          barPosScratch.writeFloat(barPosScratch.readFloat() + tune.barDx * scale);
          barPosScratch.add(4).writeFloat(barPosScratch.add(4).readFloat() + tune.barDy * scale);
          barPos = barPosScratch;
        } catch (_) {
          barPos = pos;
        }
        // Remember where this slot's bar is centred so the text call that follows it in the same
        // update pass (0x66c60 calls 0x663b0, then 0x669c0, per slot) can centre on it. The frame
        // (l01_13_hp_enemy.flo shape 0x0006) spans BAR_WIDTH_UNITS to the right of the pivot, and
        // the setter scales about the pivot, so the centre is pivot.x + width/2 * applied scale.
        if (slot < 20) {
          try {
            barCenterX[slot] = barPos.readFloat() + (BAR_WIDTH_UNITS / 2) * scale * tune.barFactor;
          } catch (_) {
            barCenterX[slot] = NaN;
          }
        }
        barOriginal(self, slot, barPos, scale * tune.barFactor, offset);
      },
      'void',
      sig,
    ),
  );

  Interceptor.replace(
    textAt,
    new NativeCallback(
      function (self, slot, pos, scale, offset) {
        textCalls += 1;
        const applied = scale * tune.textFactor;
        if (slot < 20 && !slotsSeen[slot]) {
          slotsSeen[slot] = true;
          send({
            event: 'gauge-slot-live',
            slot: slot,
            textScaleGame: scale,
            textScaleApplied: applied,
            barScaleGame: lastBarScale,
            barCalls: barCalls,
            textCalls: textCalls,
          });
        } else if (textCalls % REPORT_EVERY === 0) {
          send({
            event: 'running',
            textCalls: textCalls,
            barCalls: barCalls,
            textScaleGame: scale,
            textScaleApplied: applied,
            barScaleGame: lastBarScale,
          });
        }
        // `pos` is the caller's screen-space pivot (x, y, z, w floats; y grows downward). Lift a
        // private copy rather than writing through the caller's stack temporary.
        let liftedPos = pos;
        try {
          Memory.copy(posScratch, pos, 16); // pos is the caller's live stack local; always mapped
          // Centre on this slot's bar when it was drawn this pass; the number scales about its own
          // centre, so putting the pivot at the bar's midpoint centres it whatever its digit count.
          const centre = slot < 20 ? barCenterX[slot] : NaN;
          let baseX = isFinite(centre) ? centre : posScratch.readFloat();
          if (isFinite(centre)) {
            const digits = digitsByScene[self.add(0x18 + slot * 8).readPointer().toString()] || 0;
            baseX -= (digits * tune.textDigitAdvance * applied) / 2;
          }
          posScratch.writeFloat(baseX + tune.textDx * scale);
          posScratch.add(4).writeFloat(posScratch.add(4).readFloat() - tune.textLift * scale);
          liftedPos = posScratch;
        } catch (_) {
          liftedPos = pos;
        }
        textOriginal(self, slot, liftedPos, applied, offset);
        // THE REASON SCALE 50 LOOKED UNCHANGED. The text call's setter (vtable +0x120,
        // FeComponentBase 0x140b6aa10) forwards to FeComponentObject::setMatrix (+0x118,
        // 0x140b6aa70) with flag 1. That stores the whole 3x4 at *(el+0x58), writes the flag to
        // el+0x9a, and sets the apply-mask el+0x30 to 1; the bar's calls pass 0, giving mask 7.
        // The scaled matrix is therefore ALREADY stored when the original returns -- only the
        // mask and flag differ. Write exactly those two bytes, and only on an object whose vtable
        // is FeComponentObject's.
        //
        // An earlier version re-submitted the matrix by copying *(el+0x58) with Memory.copy, which
        // is not fault-guarded; that pointer read back 0x3f800000 and the copy took the game down
        // (ds2-crash-latest.txt: AV at 0x3f800020 inside frida-agent.dll). No raw pointer is
        // dereferenced here.
        if (slot < 20 && FULL_MATRIX) {
          try {
            const scene = self.add(0x18 + slot * 8).readPointer();
            if (!scene.isNull()) {
              const el = findElement(scene, TEXT_ELEMENT_ID, TEXT_ELEMENT_ID, 0, 0, 0, 0, 0, 0, 0, 0);
              if (!el.isNull() && el.readPointer().equals(componentObjectVtable)) {
                const maskBefore = el.add(0x30).readU8();
                const flagBefore = el.add(0x9a).readU8();
                el.add(0x30).writeU8(7);
                el.add(0x9a).writeU8(0);
                if (!inspected[slot]) {
                  inspected[slot] = true;
                  send({
                    event: 'text-full-matrix',
                    slot: slot,
                    maskBefore: maskBefore,
                    flagBefore: flagBefore,
                    maskAfter: el.add(0x30).readU8(),
                  });
                }
              } else if (!inspected[slot]) {
                inspected[slot] = true;
                send({ event: 'text-full-matrix', slot: slot, skipped: 'not FeComponentObject' });
              }
            }
          } catch (e) {
            if (!inspected[slot]) {
              inspected[slot] = true;
              send({ event: 'text-full-matrix', slot: slot, error: String(e) });
            }
          }
        }
      },
      'void',
      sig,
    ),
  );

  console.log(
    '[hp-text-scale] text x' + tune.textFactor + ' at ' + textAt + '; bar pass-through at ' + barAt,
  );
}

recv('drain', function () {
  send({ event: 'drain' });
});
