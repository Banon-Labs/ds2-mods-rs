// Watch the effect point light's update and report the lights it runs for.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/stone-light-update.js`
//
// Question (oar): does the Prism Stone copy's spliced 11001 light exist in the running game, with
// the candle values written into it? `0x140c0ba70` is the light's per-frame update; it reads the
// flicker period min/max and floor from `this+0x74/+0x78/+0x7c`, and the colour it writes lives
// behind `this+0x30`. Prints one line per distinct light object for the first ones seen, then a
// sample of the colour it wrote on later hits, which is what shows the flicker moving.

'use strict';

const image = Process.getModuleByName('DarkSoulsII.exe');
const update = image.base.add(0xc0ba70);
const seen = new Map();
let hits = 0;

Interceptor.attach(update, {
  onEnter(args) {
    this.self = args[0];
  },
  onLeave() {
    hits += 1;
    const self = this.self;
    const key = self.toString();
    const count = (seen.get(key) || 0) + 1;
    seen.set(key, count);
    if (seen.size > 12 && count === 1) return;
    if (count === 1 || count === 30 || count === 60 || count === 90) {
      let colour = '?';
      try {
        const light = self.add(0x30).readPointer();
        const c = [];
        for (let i = 0; i < 8; i++) c.push(light.add(i * 4).readFloat().toFixed(3));
        colour = c.join(',');
      } catch (e) {
        colour = 'unreadable';
      }
      console.log('[stone-light-update] light=' + key + ' hit=' + count +
        ' p6=' + self.add(0x74).readS32() + ' p7=' + self.add(0x78).readS32() +
        ' p8=' + self.add(0x7c).readFloat().toFixed(3) + ' colour@[+0x30]=' + colour +
        ' total-hits=' + hits);
    }
  },
});
console.log('[stone-light-update] attached at ' + update);
