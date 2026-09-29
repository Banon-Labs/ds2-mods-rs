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

**Open:** the unit of those heights. The `.flo` coordinates this repo has measured span a
1280x720 canvas, so the working assumption is 720p units scaled by `viewport_h / 720`. A runtime
read of the game's own text quad heights settles it before the tokens are frozen.

**Rule:** the fonts are loaded at run time from the player's own install, never shipped. The
loader turns the `.ccm` glyph records plus the decoded pages into an imgui font atlas
(`FontSource` custom glyphs, or a pre-built RGBA atlas with `add_custom_rect`). If the files are
missing, fall back to imgui's default font and log it once.

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
| `BLOOD` refusal / cannot-use | `rgb(181,44,16)` | the game's own X, see `DS2-ITEM-REQUIREMENTS.md` |
| `TEXT` | **open** | the font pages are white and tinted at draw time; read the tint from a `.flo` text record |

**Rule:** these replace the hand-copied sets in `ds2-save-file/src/picker.rs`,
`ds2-build-import/src/panel.rs` and `ds2-build-recommender-ui/src/panel.rs`. The off-palette
colours go: `FOLDER` blue, `CURRENT`/`GOOD` green, `SELECTION` blue, `CURSOR_EDGE` cyan. The game
marks focus with a lit frame and a warmer fill, not a hue.

## Shapes

| Rule | Value |
|---|---|
| Corner rounding | 0. The current 4 to 6 px rounding is a web look. |
| Panel frame | 1 px `BRONZE`. Later, the `waku_03` frame pieces drawn as a nine-slice. |
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
| Save picker | Title `"LOAD CHARACTER FROM FILE"`, eight all-caps prose hint lines, and refusals written as sentences, e.g. `"That path is missing, or it is not a file. Choose another."` | Title `Load Character` / `Save Character`. The path is a dim subtitle. Hint bar: `Open * Back * Page * Type Path`. Refusals become a `BLOOD` headline in Title Case (`Save Not Found`) plus at most one short line, and only where the headline is not enough (`Name Not Allowed` -> `\ / : * ? " < > \|`). Row markers: `[ empty ]` -> dim `Empty`; `[..]`/`[ back ]` -> one `..` row. |
| Load Build from URL | `"Paste or type a soulsplanner.com or mugenmonkey.com build link"`, plus two hint lines, one for the keyboard and one for the pad | Title `Load Build`. The field placeholder is `soulsplanner / mugenmonkey link`. Hint bar: `Load * Paste * Clear * Close`, with the device's glyph. Status headlines stay (`Fetching`, `Busy`). Details shrink to their noun: `build 123 from soulsplanner.com` stays; `a build is still being fetched -- try again in a moment` goes. |
| Build Recommender | A 35-word hint sentence. Checkbox labels such as `"the weapon, infusion, objective and grip above (two-handed: STR requirement halved\|one-handed: full STR requirement), at SL n"`. A stub warning sentence. Calibration prose. | Title `Recommender`. Mode tabs become `Weapons * Optimize * Minimum * Similar`. Delete the long checkbox explanation; the fields above already show that state. The stub warning becomes a `RUST` tag, `Placeholder data`. Calibration moves behind a hold-to-expand detail. Hint bar: `Move * Select * Step * Page * Back`. The confirm dialog keeps its single irreversible warning (`Soul memory rises. This cannot be undone.`) and drops the other two sentences into a detail list. |
| Music panel | A default imgui window with a title bar, a close X and the ProggyClean font | Restyle it to the `panel` helper: frame, fonts, no title-bar chrome. `"F10 or Esc closes this window."` goes into the hint bar. The `(playlist track\|the game's own track)` suffix becomes a small glyph. |
| Pause-menu rows and their live captions, item-warn X, HP gauge, voice-chat icon, dialog-skip greying | Already native | No change (pause rows signed off 2026-09-29). |
| Win32 fallback dialogs | Segoe UI system dialogs | No change. They are fallbacks, and the OS draws them. |

## Order of work

1. `ds2-ui-style` with the palette and shape tokens, then move the three hand-copied palettes onto it.
   This is a pure refactor, so screenshots show the palette shift and nothing else.
2. The FeFont loader. Make it the default overlay font and settle the **open** unit question.
3. `hint_bar` with bracketed names, then cut the text in every panel per the table.
4. The status strip replaces the four corner glyphs.
5. Find the game's pad-glyph atlas and swap `[A]` for real glyphs.
6. The `waku_03` nine-slice frame.
