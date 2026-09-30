# One look for everything we draw

Every mod UI in this repo was designed on its own. Seven imgui panels, one raw D3D11 overlay and a
set of native `.flo` edits share no theme module, use imgui's default ProggyClean at 16.25 px, and
three of them hand-copy the save picker's palette. The result reads as a debug tool bolted onto
DARK SOULS II. This document is the target: one type system, one palette and one set of rules,
all taken from the game's own files, and a per-element list of what changes.

Tags: **measured** = read out of the game's files in this repo's tooling; **rule** = a design
decision; **open** = not yet established.

## Rules

1. **Native first.** If a feature can live inside the game's own `.flo` UI (a menu row, a tab, a
   badge from an existing atlas), it goes there. `ds2-menu-row` and `ds2-item-warn` already prove the
   path. imgui is for what the game has no screen for: file pickers, the recommender, pasting URLs.
2. **The game's font, the game's sizes.** No ProggyClean, no Segoe UI, no third size.
3. **Icons and button glyphs before words.** A hint line of prose is a missing glyph.
4. **Say it once, in the game's voice.** DS2 labels are Title Case nouns, two or three words
   ("Load Character", "Equipment"). No sentences in labels. No `--`, no parenthetical asides, no
   explanations of how the code works.
5. **Show only what the player needs now.** Status glyphs appear when a toggle changes, then fade;
   they do not sit on screen for the whole session.
6. **Controller first.** Every panel is fully drivable by pad; hints show the pad's glyph.
7. **Colour never carries meaning alone.** Pair it with a glyph or a word.

## Type

**Measured** from `Game/font/English/FeFont_{Big,Small}.fontbnd.dcx` (DCX `DFLT` zlib over a
`BND4`). Each bundle holds one `.ccm` glyph table (version `0x20000`) and `.tpf` pages, each a
512x512 DXT5 atlas: 14 pages for Big, 8 for Small.

| Face | Line height (`.ccm` +0x08) | Use |
|---|---|---|
| `FeFont_Big` | 41 | Panel titles, and the one headline of a status banner |
| `FeFont_Small` | 28 | Everything else |

**Measured 2026-09-29** (`scripts/frida/ui-scale.js`): the float at `0x1410ae458`, which
`docs/DS2-HP-GAUGE.md` names as the UI scale's source, holds `720.0`. It is the divisor, so the
game's UI is authored on a 720-pixel-tall canvas and scaled by back-buffer height / 720. That run's
back buffer was 3840x2160 in a 1920x1080 client, so the scale comes from the back buffer, not the
window. The `.ccm` heights are the glyph atlas's own pixels. The panels draw them unscaled, since
a bitmap face blurs when it is scaled. Whether the game draws its menu text at 1x or at the full
canvas scale is **open**.

**Built** (`crates/ds2-overlay/src/fefont.rs`, `panels::install_game_fonts`): the fonts are loaded
at run time from the player's own install, never shipped. Each `.ccm` glyph becomes an imgui
custom-rect glyph, and the decoded DXT pixels are copied into the atlas before hudhook uploads it.
`FeFont_Small` is every panel's default font. `panels::title` draws in `FeFont_Big`. If the files
are missing or do not parse, the panels fall back to imgui's default font and log why.
`scripts/ds2-fefont.py check` verifies the layout against the files.

## Palette

**Measured:** median-cut over the opaque pixels of `In-game_01` (1024x1024) and `waku_03`
(1024x256), extracted with `scripts/ds2-tpf.py extract`.

| Token | Value | Source |
|---|---|---|
| `INK_0` panel fill | `#100f0e` | In-game_01, 2nd largest cluster |
| `INK_1` raised fill | `#1f1e1c` | In-game_01 |
| `INK_2` row focus fill | `#342f2b` | In-game_01, largest cluster |
| `BRONZE` frame, focus edge | `#a79171` | waku_03 frame art |
| `BRONZE_DIM` secondary text, dividers | `#816f56` | In-game_01 |
| `ASH` disabled | `#60574c` | waku_03 |
| `SLATE` field / input fill | `#202d33` | waku_03 |
| `RUST` warning | `#5c3623` | waku_03 |
| `BLOOD` refusal / cannot-use | `rgb(181,44,16)` | the game's own X, see `DS2-ITEM-REQUIREMENTS.md`. 3.04:1 on `INK_0`: glyph only, never text |
| `TEXT` / `TEXT_DIM` / `TEXT_DISABLED` | `#ffffff` / `#a0a0a0` / `#969696` tints | **measured**: the menu `.flo` text records (`FeComponentTextField`, colour at `+0x6c`). Tints multiply `FeFont`'s ~200 grey fill, so white text lands at about `#c8c8c8`. Grey sub-labels only on the panel fill: they measure 4.07:1 on `INK_1` |

**Rule:** these replace the hand-copied sets in `ds2-save-file/src/picker.rs`,
`ds2-build-import/src/panel.rs` and `ds2-build-recommender-ui/src/panel.rs`. The off-palette
colours go: `FOLDER` blue, `CURRENT`/`GOOD` green, `SELECTION` blue, `CURSOR_EDGE` cyan. The game
marks focus with a lit frame and a warmer fill, not a hue.

## Shapes

| Rule | Value |
|---|---|
| Corner rounding | 0. The current 4 to 6 px rounding is a web look. |
| Panel frame | **built**: the game's own `waku` atlas (512x128), the corner pieces the pause menu window (`l02_01_In-Game.flo` shape `0x008b`) is made of, at the game's scale, mirrored for the right, with the flat line between them stretched. 1 px `BRONZE` is the fallback |
| Dim cover | Black at 0.55 |
| Focus | `INK_2` fill and a `BRONZE` left rule. No glow and no second colour. |
| Spacing | 4-unit grid in 720p units: 4, 8, 12, 16, 24 |
| Text shadow | 1 unit, black at 0.85, on every overlay string not inside a panel |

## One crate: `ds2-ui-style`

**Rule:** one crate owns the tokens, the font loader and draw helpers. Every panel imports these
helpers and none keeps its own constants.

- `panel(title)`
- `row(label, state)`
- `hint_bar(&[(Button, verb)])`
- `status_toast(glyph)`
- `badge(glyph)`

`hint_bar` draws the pad glyph for the live device (the game's glyph atlas is **open**; until it
is found, it draws a bracketed short name such as `[A]`) followed by a one-word verb.

## Per element

| Element | Today | Target |
|---|---|---|
| Net Effects selector | `"DS2 NET EFFECTS [-]/[+] n kept"`. Rows like `"> [K] * 123456789 name"`. Hint: `"Up/Down row Left/Right page Alt+' apply Alt+M keep/unkeep Alt+9 collapse"` | Title `Effects`, with the kept count as a number on the right. Rows show the name only: a kept mark glyph, an "on you" dot and the id dimmed on the right. Hint bar: `Move * Page * Apply * Keep * Close`. |
| Net Effects sparkle, weapon/armor signs, invasion-path glyph | Each sits in its own corner, some permanently, drawn with different shapes and golds. The invasion glyph sits on top of the game's HP bars. | One status strip in one safe corner, not over the HP bars. The glyphs are `BRONZE` on `INK_0`, all the same size. A toggle shows its glyph and its name in Small for 2 s, then only the glyph stays (the net-effects sparkle included). The option to hide persistent glyphs entirely goes in the TOML. |
| Save picker | Title `"LOAD CHARACTER FROM FILE"`, eight all-caps prose hint lines, and refusals written as sentences, e.g. `"That path is missing, or it is not a file. Choose another."` | Title `Load Character` / `Save Game` (built as `Save Game`: the row copies the whole container, all ten slots, so `Save Character` would be wrong). The path is a dim subtitle. Hint bar: `Open * Back * Page * Type Path`. Refusals become a `BLOOD` headline in Title Case (`Save Not Found`) plus at most one short line, and only where the headline is not enough (`Name Not Allowed` -> `\ / : * ? " < > \|`). Row markers: `[ empty ]` -> dim `Empty`; `[..]`/`[ back ]` -> one `..` row. |
| Load Build from URL | `"Paste or type a soulsplanner.com or mugenmonkey.com build link"`, plus two hint lines, one for the keyboard and one for the pad | Title `Load Build`. The field placeholder is `soulsplanner / mugenmonkey link`. Hint bar: `Load * Paste * Clear * Close`, with the device's glyph. Status headlines stay (`Fetching`, `Busy`). Details shrink to their noun: `build 123 from soulsplanner.com` stays; `a build is still being fetched -- try again in a moment` goes. |
| Build Recommender | A 35-word hint sentence. Checkbox labels such as `"the weapon, infusion, objective and grip above (two-handed: STR requirement halved\|one-handed: full STR requirement), at SL n"`. A stub warning sentence. Calibration prose. | Title `Recommender`. Mode tabs become `Weapons * Optimize * Minimum * Similar`. Delete the long checkbox explanation; the fields above already show that state. The stub warning becomes a `RUST` tag, `Placeholder data`. Calibration moves behind a hold-to-expand detail. Hint bar: `Move * Select * Step * Page * Back`. The confirm dialog keeps its single irreversible warning (`Soul memory rises. This cannot be undone.`) and drops the other two sentences into a detail list. **Built 2026-09-30, from the Build Recommender Layout canvas:** the design's spacing around every section, Run as a press (X, R on the keyboard) instead of a button, a hint bar of `Move * Select * Step 1 * Run * Page * Back`, and the weapon list as a picker window whose rows carry the game's own item icon, the requirements with the game's X on each unmet one, the weight, a `2H only` tag or `Can't wield`, and the attack by type. **Refined the same day:** a row is laid out against its icon. A weapon icon is a 128x256 texture whose art lies in rows 29 to 219 (`scripts/ds2-item-icons.py measure`), so the row shows that 128x192 band at the texture's own size in a frame that fits it, 224 tall with its padding; the name is level with the art's top, the stats grid ends level with its bottom, and each attack line sits on a line of that grid. |
| Music panel | A default imgui window with a title bar, a close X and the ProggyClean font | Restyle it to the `panel` helper: frame, fonts, no title-bar chrome. `"F10 or Esc closes this window."` goes into the hint bar. The `(playlist track\|the game's own track)` suffix becomes a small glyph. |
| Pause-menu rows and their live captions, item-warn X, HP gauge, voice-chat icon, dialog-skip greying | Already native | No change (pause rows signed off 2026-09-29). |
| Win32 fallback dialogs | Segoe UI system dialogs | No change. They are fallbacks, and the OS draws them. |

## Order of work

1. `ds2-ui-style` with the palette and shape tokens, then move the three hand-copied palettes onto it.
   This is a pure refactor, so screenshots show the palette shift and nothing else.
2. The FeFont loader. Make it the default overlay font and settle the **open** unit question.
3. `hint_bar` with bracketed names, then cut the text in every panel per the table.
4. The status strip replaces the four corner glyphs.
5. **Built**: pad buttons are `FeFont` page-0 characters (U+2460..U+2473, the circled numbers the game's key-guide strings use), so the hint bar writes the real A/B/X/Y glyphs as text. Which bumper is left is not established.
6. **Built**: the `waku` frame (not `waku_03`, which holds bars and gauges).
