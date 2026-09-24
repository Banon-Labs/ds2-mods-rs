// Where does every Prism Stone actually get spawned, and how often on the same spot?
//
// THE QUESTION. The route decodes with zero repeated points, yet the player sees the trail end in
// a clump of duplicated stones short of the character. So the duplication is not in the polyline;
// it is in what `ds2-invasion-path` asks the engine to spawn. This records the ask.
//
// `KatanaSfxSystem`'s spawn (RVA 0x00beb670) is called as
//   spawn(system, ctrl_block, sfx_id, pose*, ...)
// and `crate::sfx::spawn` builds `pose` as the position followed by a unit direction, so the
// first three floats behind arg 3 are where the stone is going.
//
// Positions are bucketed at a tenth of a metre. A site that is asked for once is the trail being
// laid; a site asked for ten times is the trail being laid on the same ground ten times, and the
// count is the thing the screen cannot tell you.
//
// NO TIMER (`frida-js-thread-wedges-in-ds2`). A plain integer gate runs before any argument is
// touched, and the report is sent once the sample cap is reached or the host drains it.

'use strict';

Interceptor.detachAll();

const KATANA_SFX_SPAWN = 0x00beb670;
//: Prism Stone ids, ds2_rva::PRISM_STONE_SFX_IDS. Everything else spawning in the map is the
//: game's own and is counted separately rather than mixed in.
const PRISM_FIRST = 833;
const PRISM_LAST = 839;
//: Enough to cover several whole trails without letting the sample grow unbounded.
const SAMPLE_LIMIT = 600;
//: Two asks within a tenth of a metre are the same ground.
const BUCKET = 10.0;

let seen = 0;
let others = 0;
let reported = false;
const sites = {};

const module = Process.findModuleByName('DarkSoulsII.exe');
if (module === null) {
  send({ error: 'DarkSoulsII.exe is not loaded' });
} else {
  const spawn = module.base.add(KATANA_SFX_SPAWN);
  console.log('[stone-sites] hooking KatanaSfxSystem spawn at ' + spawn);
  Interceptor.attach(spawn, {
    onEnter: function (args) {
      // Integer gate FIRST -- materialising a NativePointer per call is what starved the JS
      // thread in four earlier agents on this target.
      if (seen >= SAMPLE_LIMIT) {
        return;
      }
      const id = args[2].toInt32();
      if (id < PRISM_FIRST || id > PRISM_LAST) {
        others += 1;
        return;
      }
      let at;
      try {
        at = new Float32Array(args[3].readByteArray(12));
      } catch (_) {
        return;
      }
      if (!isFinite(at[0]) || !isFinite(at[1]) || !isFinite(at[2])) {
        return;
      }
      seen += 1;
      const key =
        Math.round(at[0] * BUCKET) +
        ',' +
        Math.round(at[1] * BUCKET) +
        ',' +
        Math.round(at[2] * BUCKET);
      const found = sites[key];
      if (found === undefined) {
        sites[key] = { at: [at[0], at[1], at[2]], id: id, count: 1 };
      } else {
        found.count += 1;
      }
      if (seen >= SAMPLE_LIMIT && !reported) {
        reported = true;
        send({ sites: sites, spawns: seen, others: others });
      }
    },
  });
  console.log('[stone-sites] armed; reporting after ' + SAMPLE_LIMIT + ' Prism Stone spawn(s).');
}

recv('drain', function () {
  send({ sites: sites, spawns: seen, others: others, partial: seen < SAMPLE_LIMIT });
});
