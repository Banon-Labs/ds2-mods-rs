//! DARK SOULS II's look, as the constants every mod panel draws with.
//!
//! The save picker, Load Build and the recommender each carried a hand-copied palette of web-ish
//! colours with rounded corners. This is the one copy, and `docs/DS2-UI-DESIGN.md` is its reason.
//!
//! # Where the numbers come from
//!
//! The palette is a median-cut over the opaque pixels of the game's own menu atlases,
//! `In-game_01` (1024x1024) and `waku_03` (1024x256), pulled with
//! `scripts/ds2-tpf.py extract <name>.tpf`. [`BLOOD`] is the mean colour of the game's own
//! "cannot use" X in `waku_03` (`docs/DS2-ITEM-REQUIREMENTS.md`). Colours not taken from an atlas
//! say so where they are defined.
//!
//! Colours are `[f32; 4]` RGBA in 0..=1, the shape imgui's draw list takes. Everything here is
//! constants and arithmetic, so the contrast rules at the bottom run on the host.

/// An opaque colour from a `0xRRGGBB` literal.
#[must_use]
pub const fn rgb(hex: u32) -> [f32; 4] {
    rgba(hex, 1.0)
}

/// A colour from a `0xRRGGBB` literal with an alpha.
#[must_use]
pub const fn rgba(hex: u32, alpha: f32) -> [f32; 4] {
    [
        ((hex >> 16) & 0xff) as f32 / 255.0,
        ((hex >> 8) & 0xff) as f32 / 255.0,
        (hex & 0xff) as f32 / 255.0,
        alpha,
    ]
}

/// `c` with its alpha replaced.
#[must_use]
pub const fn with_alpha(c: [f32; 4], alpha: f32) -> [f32; 4] {
    [c[0], c[1], c[2], alpha]
}

// ---- measured: the game's own atlases ----

/// Panel fill. `In-game_01`'s second-largest cluster.
pub const INK_0: [f32; 4] = rgb(0x10_0f_0e);
/// Raised fill: cells, buttons, a hovered row. `In-game_01`.
pub const INK_1: [f32; 4] = rgb(0x1f_1e_1c);
/// Focus fill: the row or cell the cursor is on. `In-game_01`'s largest cluster.
pub const INK_2: [f32; 4] = rgb(0x34_2f_2b);
/// Frames, focus edges and secondary text -- except on [`INK_2`], where it only reaches 4.37:1
/// and secondary text is drawn in [`TEXT`] instead. `waku_03`'s frame art.
pub const BRONZE: [f32; 4] = rgb(0xa7_91_71);
/// Dividers and rules. `In-game_01`.
pub const BRONZE_DIM: [f32; 4] = rgb(0x81_6f_56);
/// Disabled text. `waku_03`.
pub const ASH: [f32; 4] = rgb(0x60_57_4c);
/// Text-field fill. `waku_03`.
pub const SLATE: [f32; 4] = rgb(0x20_2d_33);
/// Warning fill: a tag's background, never text. `waku_03`.
pub const RUST: [f32; 4] = rgb(0x5c_36_23);
/// The game's own X. For a glyph beside a refusal, never for text: 3.04:1 on [`INK_0`].
pub const BLOOD: [f32; 4] = rgb(0xb5_2c_10);

// ---- measured: the game's text tints ----
//
// FeFont's glyphs are a light grey fill (about 200) inside a dark outline, and a text component
// multiplies them by its tint (`FeComponentTextField`, colour packed at `+0x6c` by
// `0x140b6daa0`). These are the tints the menu `.flo` text records carry; what reaches the screen
// is the tint times the fill, [`on_screen`].

/// Body text: every menu text record's tint is white, so it is drawn the grey of the glyph fill.
pub const TEXT: [f32; 4] = rgb(0xff_ff_ff);
/// Secondary labels: the Inventory and Equipment sub-label records (`0x003e`, `0x0039`).
pub const TEXT_DIM: [f32; 4] = rgb(0xa0_a0_a0);
/// A row that cannot be used: the pause menu's second-state records (`0x0051`, `0x0059`).
pub const TEXT_DISABLED: [f32; 4] = rgb(0x96_96_96);

// ---- not measured ----

/// Warning text: [`RUST`] lightened until it reads on every ink fill. Derived, not measured.
pub const WARN_TEXT: [f32; 4] = rgb(0xe0_94_6c);
/// A text field while it is being typed in: [`SLATE`] one step lighter. Derived.
pub const SLATE_EDIT: [f32; 4] = rgb(0x2b_3c_44);

// ---- composites ----

/// Panel fill as drawn: the scene barely shows through.
pub const PANEL_BG: [f32; 4] = with_alpha(INK_0, 0.96);
/// The full-screen cover behind a modal panel.
pub const DIM_COVER: [f32; 4] = rgba(0x00_00_00, 0.55);
/// Selected text inside a field.
pub const TEXT_SELECTION: [f32; 4] = with_alpha(BRONZE_DIM, 0.55);

// ---- shapes ----

/// Corner rounding for every panel, cell and field. DS2's frames are square.
pub const ROUNDING: f32 = 0.0;
/// Frame stroke.
pub const FRAME_PX: f32 = 1.0;

/// What a text `tint` looks like on screen: the tint times the `FeFont` glyph fill (about 200 of 255).
#[must_use]
pub fn on_screen(tint: [f32; 4]) -> [f32; 4] {
    const FILL: f32 = 200.0 / 255.0;
    [tint[0] * FILL, tint[1] * FILL, tint[2] * FILL, tint[3]]
}

/// WCAG relative luminance of an sRGB colour.
#[must_use]
pub fn luminance(c: [f32; 4]) -> f32 {
    fn lin(v: f32) -> f32 {
        if v <= 0.039_28 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}

/// WCAG contrast ratio between two opaque colours.
#[must_use]
pub fn contrast(a: [f32; 4], b: [f32; 4]) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Body-size text has to reach 4.5:1 on every fill it is drawn over, as it lands on screen:
    /// the tint times the glyph fill.
    #[test]
    fn text_reads_on_every_fill() {
        for (fill_name, fill) in [
            ("INK_0", INK_0),
            ("INK_1", INK_1),
            ("INK_2", INK_2),
            ("SLATE", SLATE),
            ("SLATE_EDIT", SLATE_EDIT),
            ("RUST", RUST),
        ] {
            let ratio = contrast(on_screen(TEXT), fill);
            assert!(ratio >= 4.5, "TEXT on {fill_name} is {ratio:.2}:1");
        }
        for (name, fg) in [("TEXT_DIM", TEXT_DIM), ("WARN_TEXT", WARN_TEXT)] {
            let ratio = contrast(on_screen(fg), INK_0);
            assert!(ratio >= 4.5, "{name} on INK_0 is {ratio:.2}:1");
        }
    }

    /// Secondary text on a hovered or focused row is drawn in TEXT: the game's own grey sub-label
    /// tint measures 4.07:1 on the raised fill and less on the focus fill.
    #[test]
    fn dim_text_is_only_for_the_panel_fill() {
        assert!(contrast(on_screen(TEXT_DIM), INK_1) < 4.5);
        assert!(contrast(on_screen(TEXT_DIM), INK_2) < 4.5);
    }

    /// Measured 3.04:1: enough for the X glyph, not for body text.
    #[test]
    fn blood_is_not_a_text_colour() {
        assert!(contrast(BLOOD, INK_0) < 4.5);
    }

    #[test]
    fn hex_unpacks_in_order() {
        assert_eq!(rgb(0xff_80_00), [1.0, 128.0 / 255.0, 0.0, 1.0]);
    }
}
