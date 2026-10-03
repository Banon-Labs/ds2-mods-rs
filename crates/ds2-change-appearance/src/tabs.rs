//! The creator's tab bar without Class & gift: three tabs, each a third of the bar.
//!
//! The bar lives in `l09_01_chara_make.flo`. Tabs `0x5f5c1c0..1c3` are `def 0x00b8`'s children and
//! their labels `0x5f5c420..423` are `def 0x00ae`'s; each record's transform starts with
//! `x, y, scale_x, scale_y`. A tab's art is shape `0x00b5`, 287.2 wide, drawn 4.65 right of the
//! tab's x. The four sit on a 271 pitch, so neighbours overlap by 16.2, and that overlap is the
//! divider line. Labels sit 110.5 left of their tab's centre. (`scripts/ds2-flo.py` over the
//! extracted file.)
//!
//! Three tabs keep the span the four covered and the same overlap. Class & gift's tab and label are
//! moved off-screen. Proven with `scripts/frida/chara-tabs-thirds.js` on 2026-10-03.
//!
//! The parsed layout is cached for the process, so the same bytes serve every later creator,
//! including New Game's. Hence [`Layout`]: the edit is applied for our creator and undone after.

/// Where a tab's art starts, right of the tab's own x: child record 0.35 plus quad offset 4.3.
const ART_LEFT: f32 = 4.65;
/// Shape `0x00b5`'s width.
const ART_WIDTH: f32 = 287.2;
/// How far neighbouring tabs' art overlaps with four tabs on a 271 pitch.
const OVERLAP: f32 = ART_WIDTH - 271.0;
/// A label's x, left of its tab's centre.
const LABEL_FROM_CENTRE: f32 = 110.5;
/// Far enough left that neither the tab nor the label reaches the screen.
const OFFSCREEN_X: f32 = -4000.0;

/// The span the four tabs' art covers, which three tabs keep.
const SPAN_LEFT: f32 = 86.4 + ART_LEFT;
const SPAN_RIGHT: f32 = 899.6 + ART_LEFT + ART_WIDTH;
const WIDTH3: f32 = (SPAN_RIGHT - SPAN_LEFT + 2.0 * OVERLAP) / 3.0;
/// Horizontal scale of each of the three tabs.
pub const SCALE3: f32 = WIDTH3 / ART_WIDTH;
const PITCH3: f32 = WIDTH3 - OVERLAP;

const fn tab_x(k: u8) -> f32 {
    SPAN_LEFT - ART_LEFT * SCALE3 + PITCH3 * k as f32
}

const fn label_x(k: u8) -> f32 {
    tab_x(k) + (ART_LEFT + ART_WIDTH / 2.0) * SCALE3 - LABEL_FROM_CENTRE
}

/// One record the edit touches.
#[derive(Clone, Copy, Debug)]
pub struct Edit {
    /// Offset of the record in the file; its element id is at `+0x1c`.
    pub record: usize,
    /// The element id the record must carry.
    pub id: u32,
    /// Offset of the record's transform block in the file.
    pub transform: usize,
    /// `x, y, scale_x, scale_y` as shipped.
    pub original: [f32; 4],
    /// The same four for three tabs.
    pub thirds: [f32; 4],
}

const fn edit(record: usize, id: u32, transform: usize, x: f32, y: f32, nx: f32, sx: f32) -> Edit {
    Edit {
        record,
        id,
        transform,
        original: [x, y, 1.0, 1.0],
        thirds: [nx, y, sx, 1.0],
    }
}

/// Offset of a record's element id.
pub const RECORD_ID_OFFSET: usize = 0x1c;

/// The eight records: four tabs, then four labels.
pub const EDITS: [Edit; 8] = [
    edit(0xa020, 0x05f5_c1c0, 0x594d8, 86.4, 67.05, OFFSCREEN_X, 1.0),
    edit(0xa048, 0x05f5_c1c1, 0x59508, 357.4, 67.05, tab_x(0), SCALE3),
    edit(0xa070, 0x05f5_c1c2, 0x59538, 628.4, 67.05, tab_x(1), SCALE3),
    edit(0xa098, 0x05f5_c1c3, 0x59568, 899.6, 67.05, tab_x(2), SCALE3),
    edit(
        0x9eb8,
        0x05f5_c420,
        0x584e8,
        124.15,
        72.35,
        OFFSCREEN_X,
        1.0,
    ),
    edit(0x9e90, 0x05f5_c421, 0x584b8, 395.15, 72.35, label_x(0), 1.0),
    edit(0x9e68, 0x05f5_c422, 0x58488, 666.15, 72.35, label_x(1), 1.0),
    edit(0x9e40, 0x05f5_c423, 0x58458, 937.15, 72.35, label_x(2), 1.0),
];

/// Which of the two layouts a buffer holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// As shipped: four tabs.
    Original,
    /// Three tabs, Class & gift off-screen.
    Thirds,
}

impl Layout {
    fn values(self, edit: &Edit) -> [f32; 4] {
        match self {
            Self::Original => edit.original,
            Self::Thirds => edit.thirds,
        }
    }
}

/// Which layout `read` sees, or `None` when it is neither: a different file, a freed block, or one
/// half-written. `read(offset, out)` fills `out` from the layout and returns false when it cannot.
pub fn layout_of(read: impl Fn(usize, &mut [u8]) -> bool) -> Option<Layout> {
    let floats = |at: usize| {
        let mut raw = [0u8; 16];
        read(at, &mut raw).then(|| {
            let mut out = [0f32; 4];
            for (v, chunk) in out.iter_mut().zip(raw.as_chunks::<4>().0) {
                *v = f32::from_le_bytes(*chunk);
            }
            out
        })
    };
    let near = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01);
    let mut seen = None;
    for edit in &EDITS {
        let mut id = [0u8; 4];
        if !read(edit.record + RECORD_ID_OFFSET, &mut id) || u32::from_le_bytes(id) != edit.id {
            return None;
        }
        let now = floats(edit.transform)?;
        let this = [Layout::Original, Layout::Thirds]
            .into_iter()
            .find(|layout| near(now, layout.values(edit)))?;
        if seen.is_some_and(|s| s != this) {
            return None;
        }
        seen = Some(this);
    }
    seen
}

/// The writes that turn either layout into `to`: `(offset, little-endian x, y, scale_x, scale_y)`.
pub fn writes(to: Layout) -> impl Iterator<Item = (usize, [u8; 16])> {
    EDITS.into_iter().map(move |edit| {
        let mut out = [0u8; 16];
        for (chunk, v) in out.as_chunks_mut::<4>().0.iter_mut().zip(to.values(&edit)) {
            *chunk = v.to_le_bytes();
        }
        (edit.transform, out)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shipped() -> Vec<u8> {
        let mut buf = vec![0u8; 0x60000];
        for edit in &EDITS {
            buf[edit.record + RECORD_ID_OFFSET..][..4].copy_from_slice(&edit.id.to_le_bytes());
        }
        for (at, bytes) in writes(Layout::Original) {
            buf[at..][..16].copy_from_slice(&bytes);
        }
        buf
    }

    fn reader(buf: &[u8]) -> impl Fn(usize, &mut [u8]) -> bool + '_ {
        |at, out| match buf.get(at..at + out.len()) {
            Some(src) => {
                out.copy_from_slice(src);
                true
            }
            None => false,
        }
    }

    #[test]
    fn toggles_both_ways() {
        let mut buf = shipped();
        assert_eq!(layout_of(reader(&buf)), Some(Layout::Original));
        for (at, bytes) in writes(Layout::Thirds) {
            buf[at..][..16].copy_from_slice(&bytes);
        }
        assert_eq!(layout_of(reader(&buf)), Some(Layout::Thirds));
        for (at, bytes) in writes(Layout::Original) {
            buf[at..][..16].copy_from_slice(&bytes);
        }
        assert_eq!(buf, shipped());
    }

    #[test]
    fn refuses_anything_else() {
        let mut buf = shipped();
        buf[EDITS[3].record + RECORD_ID_OFFSET] ^= 1;
        assert_eq!(layout_of(reader(&buf)), None, "a different id");

        let mut buf = shipped();
        let (at, bytes) = writes(Layout::Thirds).next().unwrap();
        buf[at..][..16].copy_from_slice(&bytes);
        assert_eq!(layout_of(reader(&buf)), None, "half written");

        assert_eq!(layout_of(|_, _| false), None, "unreadable");
    }

    #[test]
    fn three_tabs_keep_the_span_and_centre() {
        let art = |k: u8| {
            let x = tab_x(k) + ART_LEFT * SCALE3;
            (x, x + ART_WIDTH * SCALE3)
        };
        assert!((art(0).0 - SPAN_LEFT).abs() < 0.01);
        assert!((art(2).1 - SPAN_RIGHT).abs() < 0.01);
        assert!((art(0).1 - art(1).0 - OVERLAP).abs() < 0.01);
        // The middle tab centres where the four did: halfway between the outer two's centres.
        let centre = |x: f32| x + ART_LEFT + ART_WIDTH / 2.0;
        let mid = (art(1).0 + art(1).1) / 2.0;
        assert!((mid - (centre(86.4) + centre(899.6)) / 2.0).abs() < 0.01);
        // And each label keeps its distance from its tab's centre.
        for k in 0..3 {
            let c = (art(k).0 + art(k).1) / 2.0;
            assert!((c - label_x(k) - LABEL_FROM_CENTRE).abs() < 0.01);
        }
    }
}
