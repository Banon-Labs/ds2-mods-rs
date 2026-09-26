// Read the live first bytes of the boot-phase functions ds2-boot-timeline wraps, once.
//
// `python3 scripts/ds2-frida-up.py`, then
// `uv run --with frida python3 scripts/ds2-frida-watch.py --agent scripts/frida/boot-phase-prologues.js`
//
// Question (a8b): are these entries in the running image the clean prologues the deobfuscated
// image shows, or has Arxan redirected any of them with a jmp? A MinHook on a redirected entry
// would hook the stub instead of the function.

'use strict';

const image = Process.getModuleByName('DarkSoulsII.exe');
const sites = [
  ['win-main', 0x2eb630, '48895c2410555657'],
  ['app-setup', 0xaef500, '555657415448'],
  ['archive-mounts', 0x2ef410, '40534883ec20'],
  ['input-devices', 0x2ec290, '4889542410'],
  ['graphics-init', 0xaead80, '48895c2418555657'],
  ['sound-init', 0xb049f0, '48895c2408'],
  ['katana-init', 0x2eed00, '48895c2410'],
  ['app-frame', 0xaeeed0, '4053'],
];
const hex = (p, n) => Array.from(new Uint8Array(p.readByteArray(n)))
  .map((b) => b.toString(16).padStart(2, '0')).join('');
for (const [name, rva, want] of sites) {
  const live = hex(image.base.add(rva), 12);
  console.log('[boot-phase-prologues] ' + name + ' rva=0x' + rva.toString(16) + ' live=' + live +
    ' ' + (live.startsWith(want) ? 'MATCH' : 'DIFFERS want=' + want));
}
