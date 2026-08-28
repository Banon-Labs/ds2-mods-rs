//! Lengthen the quit tab's banner, on the field that actually draws it.
//!
//! # What the three failed stretches were missing
//!
//! Scaling the panel's record did nothing at 8%, at 17%, and at a deliberately unmissable 2.0,
//! while the log proved every write landed. The tree walk then found why: the panel holds exactly
//! one drawable, a `FeComponentTextureShape`, and
//! [`ds2_rva::FE_TEXTURE_SHAPE_INIT`] shows such a shape is sized by its **own quad**, copied in at
//! build time from the shape table and never re-derived from an ancestor.
//!
//! So the quad is the field. For shape `0x0220` there is exactly one, reading
//! `(914.20, 1.10, 972.00, 342.35)` — `57.80 x 341.25`, which is the number every panel measurement
//! in `ds2-rva` was built on. Three row slots and `25.65` of margin under the last one; one more row
//! is one more `48.00` of pitch, so `y1` goes from `342.35` to `390.35` and the margin is preserved.
//!
//! # Why the LIVE component and not the file
//!
//! Shape `0x0220` is shared: all three menu tabs' panels instantiate it. Editing the loaded `.flo`
//! would lengthen every tab's banner, including the two that have nothing extra to cover. Editing
//! the one component built for the quit tab touches only the quit tab.
//!
//! # Destination, not source
//!
//! The initialiser fills `+0x50` and `+0x58` with the SAME four floats, so nothing in it
//! distinguishes them. The DRAW does: `0x140b6f200` passes both to `FUN_140b521c0` and, when a
//! texture is bound, substitutes a local `{0, 0, texWidth, texHeight}` for the fourth argument. A
//! rect the texture's own pixel size can stand in for is the source.
//!
//! Growing both is what put nearly-transparent art on the added row -- the destination made room
//! and the source reached below the banner into empty atlas. Only the destination grows now.

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::LOG_PREFIX;
use crate::install::log;

/// The DESTINATION rect, and only that one.
///
/// Growing both put art on the added row that was almost entirely transparent: the destination made
/// room and the source pulled in whatever sits below the banner in the atlas, which is nothing. The
/// draw (`0x140b6f200`) is what separates them -- it passes `[this+0x50]` and `[this+0x58]` to
/// `FUN_140b521c0` and substitutes the texture's own `{0, 0, w, h}` for the fourth argument when a
/// texture is bound, which is what a SOURCE rect is.
///
/// With the source left alone the shipped art stretches to fill the taller destination.
const RECTS: [usize; 1] = [ds2_rva::FE_TEXTURE_SHAPE_DEST_RECT_OFFSET];

/// Byte offset of `y1` inside a `(x0, y0, x1, y1)` rect.
const Y1: usize = 0x0c;

static LENGTHENED: AtomicUsize = AtomicUsize::new(0);
static REFUSED: AtomicUsize = AtomicUsize::new(0);

/// Find the texture shape under a panel component.
///
/// The panel is a `FeComponentObject`, so its child is reached through the `+0x38` link; that child
/// is the `FeComponentSprite` holding the display list, and the shape is the entry filed under
/// [`ds2_rva::FE_TEXTURE_SHAPE_DISPLAY_KEY`].
///
/// # Safety
///
/// `panel` must be the live `FeComponentObject` for the quit tab's panel, and `base` the module
/// base. Every pointer read is checked before it is followed.
unsafe fn texture_shape_of(panel: *const u8, base: usize) -> *mut u8 {
    let sane = |p: *const u8| -> bool {
        !p.is_null() && (p as usize) >= 0x1_0000 && (p as usize).is_multiple_of(8)
    };
    if !sane(panel) {
        return std::ptr::null_mut();
    }
    // SAFETY: `panel` is a validated `FeComponentObject`, whose `+0x38` is the child link.
    let sprite = unsafe {
        panel
            .add(ds2_rva::FE_COMPONENT_FIRST_CHILD_OFFSET)
            .cast::<*const u8>()
            .read()
    };
    if !sane(sprite) {
        return std::ptr::null_mut();
    }
    // SAFETY: validated pointer; its first qword is its vtable.
    let vtable = unsafe { sprite.cast::<usize>().read() };
    if vtable != base + ds2_rva::FE_COMPONENT_SPRITE_VTABLE as usize {
        return std::ptr::null_mut();
    }
    // SAFETY: the class is established, so these are its own display list and live count.
    let (list, count) = unsafe {
        (
            sprite
                .add(ds2_rva::FE_COMPONENT_DISPLAY_LIST_OFFSET)
                .cast::<*const u8>()
                .read(),
            sprite
                .add(ds2_rva::FE_COMPONENT_DISPLAY_COUNT_OFFSET)
                .cast::<u16>()
                .read() as usize,
        )
    };
    if !sane(list) || count > 64 {
        return std::ptr::null_mut();
    }
    for i in 0..count {
        // SAFETY: `i < count`, at the stride the game's own search uses.
        let (child, key) = unsafe {
            let entry = list.add(i * ds2_rva::FE_COMPONENT_DISPLAY_ENTRY_STRIDE);
            (
                entry
                    .add(ds2_rva::FE_COMPONENT_DISPLAY_ENTRY_CHILD_OFFSET)
                    .cast::<*mut u8>()
                    .read(),
                entry
                    .add(ds2_rva::FE_COMPONENT_DISPLAY_ENTRY_KEY_OFFSET)
                    .cast::<u32>()
                    .read(),
            )
        };
        if key != ds2_rva::FE_TEXTURE_SHAPE_DISPLAY_KEY || !sane(child) {
            continue;
        }
        // SAFETY: validated pointer.
        if unsafe { child.cast::<usize>().read() }
            == base + ds2_rva::FE_COMPONENT_TEXTURE_SHAPE_VTABLE as usize
        {
            return child;
        }
    }
    std::ptr::null_mut()
}

/// Lengthen the banner's quad, if it is the one this expects.
///
/// # Safety
///
/// `panel` must be the live panel component for the quit tab, and `base` the module base.
pub unsafe fn lengthen(panel: *const u8, base: usize) {
    let refuse = |why: std::fmt::Arguments<'_>| {
        let n = REFUSED.fetch_add(1, Ordering::Relaxed) + 1;
        log(format_args!(
            "{LOG_PREFIX} banner REFUSED {why} refusals={n} -- the banner is the shipped one"
        ));
    };
    // SAFETY: the caller guarantees the panel; the walk validates everything it follows.
    let shape = unsafe { texture_shape_of(panel, base) };
    if shape.is_null() {
        refuse(format_args!("no texture shape under the panel"));
        return;
    }
    // SAFETY: `shape` is a validated `FeComponentTextureShape`, and this is the field its own
    // initialiser reads the quad count from.
    let entry = unsafe {
        shape
            .add(ds2_rva::FE_TEXTURE_SHAPE_ENTRY_OFFSET)
            .cast::<*const u8>()
            .read()
    };
    if entry.is_null() {
        refuse(format_args!("the shape has no table entry"));
        return;
    }
    // SAFETY: a shape table entry, still mapped in the loaded document.
    let quads = unsafe {
        entry
            .add(ds2_rva::FE_SHAPE_ENTRY_COUNT_OFFSET)
            .cast::<u16>()
            .read()
    } as usize;
    // THE CHECK. Shape 0x0220 has exactly one quad. A panel with a different count is a different
    // shape, and writing into it would be a guess wearing a measurement's clothes.
    if quads != 1 {
        refuse(format_args!("quads={quads}, expected 1"));
        return;
    }

    for offset in RECTS {
        // SAFETY: the arrays are `quads * 0x10` bytes and `quads` is 1, so element 0 is inside.
        let rect = unsafe { shape.add(offset).cast::<*mut u8>().read() };
        if rect.is_null() {
            refuse(format_args!("rect array at {offset:#x} is null"));
            return;
        }
        // SAFETY: a live four-float rect.
        let before = unsafe { std::slice::from_raw_parts(rect.cast::<f32>(), 4) };
        let y1 = before[3];
        // ONE ROW'S PITCH PER REGISTERED ROW. The shipped quad ends `25.65` below the last row,
        // and that margin is what the growth preserves -- so two rows want two pitches, not a
        // second look at a constant sized for one.
        let rows = crate::api::rows_for(crate::api::Tab::Quit).len().max(1);
        let grown = ds2_rva::FE_BANNER_QUAD_SHIPPED_Y1 + ds2_rva::FLO_ROW_PITCH * rows as f32;
        if (y1 - ds2_rva::FE_BANNER_QUAD_SHIPPED_Y1).abs() > 0.5 {
            refuse(format_args!(
                "rect at {offset:#x} reads y1={y1}, expected {}",
                ds2_rva::FE_BANNER_QUAD_SHIPPED_Y1
            ));
            return;
        }
        let quad = format!("{} {} {} {}", before[0], before[1], before[2], before[3]);
        // SAFETY: element 0 of a `quads`-element array of four floats.
        unsafe {
            rect.add(Y1).cast::<f32>().write(grown);
        }
        let n = LENGTHENED.fetch_add(1, Ordering::Relaxed) + 1;
        // First couple of opens only; the sink calls `sync_all` per line and the pause menu is
        // opened over and over.
        if n > 2 {
            continue;
        }
        log(format_args!(
            "{LOG_PREFIX} banner rect={offset:#x} at=0x{:016x} before=[{quad}] y1={y1}->{} \
             writes={n}",
            rect as usize, grown
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The new quad must be TALLER, and by about one row pitch -- a value that is neither is a
    /// number somebody typed rather than the arithmetic in `ds2-rva`.
    #[test]
    fn the_new_quad_is_one_row_taller() {
        let grew = ds2_rva::FE_BANNER_QUAD_Y1 - ds2_rva::FE_BANNER_QUAD_SHIPPED_Y1;
        assert!(grew > 0.0);
        assert!(
            (grew - 48.0).abs() < 0.5,
            "grew by {grew}, which is not one row pitch"
        );
    }

    /// The SOURCE rect must be left alone: growing it samples atlas the banner does not occupy,
    /// which is the transparent-art result this replaced.
    #[test]
    fn only_the_destination_rect_is_written() {
        assert_eq!(RECTS, [ds2_rva::FE_TEXTURE_SHAPE_DEST_RECT_OFFSET]);
        assert!(!RECTS.contains(&ds2_rva::FE_TEXTURE_SHAPE_SOURCE_RECT_OFFSET));
        assert_eq!(Y1 + 4, ds2_rva::FE_TEXTURE_SHAPE_RECT_STRIDE);
    }
}
