//! DARK SOULS II's own bitmap fonts, read out of the player's install.
//!
//! `Game/font/English/FeFont_{Big,Small}.fontbnd.dcx` are the faces the game's menus draw with.
//! Nothing is shipped: the panels load them from the install at run time, and fall back to
//! imgui's default font if they are missing. `scripts/ds2-fefont.py check` proves every layout
//! claim below against the real files; the host tests here re-read the same files when they are
//! present.
//!
//! # Layout
//!
//! A `DCX`/`DFLT` zlib stream (payload at 0x4c, inflated size big-endian at 0x1c) around a
//! `BND4` with single-byte names. It holds `FeFont_X.ccm` and one `.tpf` per texture page: page 0
//! is `FeFont_X_0000_win64.tpf` (512x512, CJK symbols only), page n is `FeFont_X_000n.tpf`
//! (256x256; DXT5 in Big, DXT3 in Small).
//!
//! The `.ccm` (version 0x20000): `+0x08 u16` line height (41 / 28), `+0x0e u16` region count,
//! `+0x10 u16` glyph count, `+0x14 u32` region table, `+0x18 u32` glyph table. A region is
//! `u16 x1, y1, x2, y2` (end-exclusive). A glyph is 24 bytes: `u32 code, u32 region byte offset,
//! i16 page, i16 pre_space, i16 width, i16 advance`, then eight zero bytes. The ink goes
//! `pre_space` right of the pen and the pen then moves `pre_space + advance`: the digits share
//! one step that way in both faces, and not by `advance` alone.
//!
//! The ink is a light grey fill inside a one-pixel near-black outline, with alpha over both. It
//! is copied as it is: imgui multiplies it by the text colour, which is how the game tints it.

use std::path::Path;

/// One of the two faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaceName {
    /// Titles. Line height 41.
    Big,
    /// Everything else. Line height 28.
    Small,
}

impl FaceName {
    fn stem(self) -> &'static str {
        match self {
            Self::Big => "FeFont_Big",
            Self::Small => "FeFont_Small",
        }
    }
}

/// One drawable glyph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyph {
    /// Unicode code point.
    pub code: u32,
    /// Texture page index into [`Face::pages`].
    pub page: usize,
    /// `x1, y1, x2, y2` in the page's pixels, end-exclusive.
    pub rect: [u16; 4],
    /// Pixels from the pen to the ink's left edge.
    pub pre_space: i16,
    /// Pixels from the ink's left edge to the next pen position.
    pub advance: i16,
}

impl Glyph {
    /// Ink width in pixels.
    #[must_use]
    pub fn width(&self) -> u16 {
        self.rect[2] - self.rect[0]
    }

    /// Ink height in pixels.
    #[must_use]
    pub fn height(&self) -> u16 {
        self.rect[3] - self.rect[1]
    }

    /// How far the pen moves past this glyph.
    #[must_use]
    pub fn pen_step(&self) -> i32 {
        i32::from(self.pre_space) + i32::from(self.advance)
    }
}

/// A decoded texture page, RGBA8 row-major.
#[derive(Clone, Debug)]
pub struct Page {
    /// Width in pixels.
    pub width: usize,
    /// Height in pixels.
    pub height: usize,
    /// `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

/// A whole face: its line height, its glyphs sorted by code, and its pages.
#[derive(Clone, Debug)]
pub struct Face {
    /// Line height in pixels (41 for Big, 28 for Small).
    pub line_height: u16,
    /// Every well-formed glyph, sorted by code.
    pub glyphs: Vec<Glyph>,
    /// Decoded pages, indexed by [`Glyph::page`]. Pages no glyph asked for are left empty.
    pub pages: Vec<Page>,
}

/// Why a face could not be read.
#[derive(Debug)]
pub enum Error {
    /// The file could not be opened or read.
    Io(std::io::Error),
    /// The bytes are not what the layout above says; the text says which part.
    Format(&'static str),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Format(what) => write!(f, "{what}"),
        }
    }
}

type Result<T> = core::result::Result<T, Error>;

fn u16_le(b: &[u8], at: usize) -> Result<u16> {
    b.get(at..at + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or(Error::Format("short read (u16)"))
}

fn i16_le(b: &[u8], at: usize) -> Result<i16> {
    u16_le(b, at).map(|v| v as i16)
}

fn u32_le(b: &[u8], at: usize) -> Result<u32> {
    b.get(at..at + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or(Error::Format("short read (u32)"))
}

fn u64_le(b: &[u8], at: usize) -> Result<u64> {
    b.get(at..at + 8)
        .map(|s| u64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]))
        .ok_or(Error::Format("short read (u64)"))
}

fn usize_of(v: u64) -> Result<usize> {
    usize::try_from(v).map_err(|_| Error::Format("offset out of range"))
}

/// Unwrap a `DCX`/`DFLT` container.
pub(crate) fn dcx(bytes: &[u8]) -> Result<Vec<u8>> {
    if bytes.get(..4) != Some(b"DCX\0") || bytes.get(0x28..0x2c) != Some(b"DFLT") {
        return Err(Error::Format("not a DCX/DFLT container"));
    }
    let size = bytes
        .get(0x1c..0x20)
        .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or(Error::Format("short DCX header"))?;
    let payload = bytes.get(0x4c..).ok_or(Error::Format("short DCX"))?;
    let data = miniz_oxide::inflate::decompress_to_vec_zlib(payload)
        .map_err(|_| Error::Format("DCX payload does not inflate"))?;
    if data.len() != size as usize {
        return Err(Error::Format("DCX inflated size mismatch"));
    }
    Ok(data)
}

/// The members of a single-byte-name `BND4`, as `(name, bytes)`.
pub(crate) fn bnd4(data: &[u8]) -> Result<Vec<(String, &[u8])>> {
    if data.get(..4) != Some(b"BND4") || data.get(0x30) != Some(&0) {
        return Err(Error::Format("not a single-byte-name BND4"));
    }
    let count = u32_le(data, 0x0c)? as usize;
    let header = usize_of(u64_le(data, 0x10)?)?;
    let entry = usize_of(u64_le(data, 0x20)?)?;
    let mut members = Vec::with_capacity(count);
    for index in 0..count {
        let at = header + index * entry + 8;
        let size = usize_of(u64_le(data, at)?)?;
        let unpacked = usize_of(u64_le(data, at + 8)?)?;
        let offset = u32_le(data, at + 16)? as usize;
        let name_at = u32_le(data, at + 24)? as usize;
        if size != unpacked {
            return Err(Error::Format("compressed BND4 member"));
        }
        let name_end = data
            .get(name_at..)
            .and_then(|s| s.iter().position(|&c| c == 0))
            .ok_or(Error::Format("unterminated BND4 name"))?;
        let name = String::from_utf8_lossy(&data[name_at..name_at + name_end]).into_owned();
        let body = data
            .get(offset..offset + size)
            .ok_or(Error::Format("BND4 member out of range"))?;
        members.push((name, body));
    }
    Ok(members)
}

/// The glyph table of a `.ccm`, and its line height.
fn ccm(b: &[u8]) -> Result<(u16, Vec<Glyph>)> {
    if u32_le(b, 0)? != 0x2_0000 {
        return Err(Error::Format("not a version 0x20000 .ccm"));
    }
    let line = u16_le(b, 0x08)?;
    let count = u16_le(b, 0x10)? as usize;
    let table = u32_le(b, 0x18)? as usize;
    let mut glyphs = Vec::with_capacity(count);
    for index in 0..count {
        let at = table + index * 24;
        let code = u32_le(b, at)?;
        let region = u32_le(b, at + 4)? as usize;
        let page = i16_le(b, at + 8)?;
        let rect = [
            u16_le(b, region)?,
            u16_le(b, region + 2)?,
            u16_le(b, region + 4)?,
            u16_le(b, region + 6)?,
        ];
        let Ok(page) = usize::try_from(page) else {
            continue;
        };
        // One Big glyph (U+2465) carries a broken region with y1 = 65531; skip anything malformed.
        if rect[2] <= rect[0] || rect[3] <= rect[1] {
            continue;
        }
        let glyph = Glyph {
            code,
            page,
            rect,
            pre_space: i16_le(b, at + 10)?,
            advance: i16_le(b, at + 14)?,
        };
        glyphs.push(glyph);
    }
    Ok((line, glyphs))
}

/// The DDS inside a single-texture `.tpf`: `(width, height, fourcc, block data)`.
pub(crate) fn tpf(b: &[u8]) -> Result<(usize, usize, [u8; 4], &[u8])> {
    if b.get(..4) != Some(b"TPF\0") {
        return Err(Error::Format("not a TPF"));
    }
    let offset = u32_le(b, 0x10)? as usize;
    let size = u32_le(b, 0x14)? as usize;
    let dds = b
        .get(offset..offset + size)
        .ok_or(Error::Format("TPF texture out of range"))?;
    if dds.get(..4) != Some(b"DDS ") {
        return Err(Error::Format("TPF texture is not a DDS"));
    }
    let height = u32_le(dds, 12)? as usize;
    let width = u32_le(dds, 16)? as usize;
    let fourcc: [u8; 4] = dds
        .get(84..88)
        .and_then(|s| s.try_into().ok())
        .ok_or(Error::Format("short DDS header"))?;
    let blocks = dds.get(128..).ok_or(Error::Format("short DDS"))?;
    Ok((width, height, fourcc, blocks))
}

fn rgb565(v: u16) -> [u8; 3] {
    let r = ((v >> 11) & 31) as u32 * 255 / 31;
    let g = ((v >> 5) & 63) as u32 * 255 / 63;
    let b = (v & 31) as u32 * 255 / 31;
    [r as u8, g as u8, b as u8]
}

/// Decode a DXT3 or DXT5 surface to RGBA8.
pub(crate) fn decode(width: usize, height: usize, fourcc: [u8; 4], blocks: &[u8]) -> Result<Page> {
    let dxt5 = match &fourcc {
        b"DXT5" => true,
        b"DXT3" => false,
        _ => return Err(Error::Format("page is neither DXT3 nor DXT5")),
    };
    let blocks_needed = width.div_ceil(4) * height.div_ceil(4) * 16;
    if blocks.len() < blocks_needed {
        return Err(Error::Format("DDS block data is short"));
    }
    let mut rgba = vec![0u8; width * height * 4];
    let mut at = 0;
    for by in (0..height).step_by(4) {
        for bx in (0..width).step_by(4) {
            let block = &blocks[at..at + 16];
            at += 16;
            let mut alpha = [0u8; 16];
            if dxt5 {
                let (a0, a1) = (u32::from(block[0]), u32::from(block[1]));
                let mut table = [0u8; 8];
                table[0] = a0 as u8;
                table[1] = a1 as u8;
                if a0 > a1 {
                    for i in 0..6 {
                        table[i + 2] = (((6 - i as u32) * a0 + (i as u32 + 1) * a1) / 7) as u8;
                    }
                } else {
                    for i in 0..4 {
                        table[i + 2] = (((4 - i as u32) * a0 + (i as u32 + 1) * a1) / 5) as u8;
                    }
                    table[6] = 0;
                    table[7] = 255;
                }
                let mut bits = 0u64;
                for (i, byte) in block[2..8].iter().enumerate() {
                    bits |= u64::from(*byte) << (8 * i);
                }
                for (i, a) in alpha.iter_mut().enumerate() {
                    *a = table[((bits >> (3 * i)) & 7) as usize];
                }
            } else {
                let mut bits = 0u64;
                for (i, byte) in block[..8].iter().enumerate() {
                    bits |= u64::from(*byte) << (8 * i);
                }
                for (i, a) in alpha.iter_mut().enumerate() {
                    *a = (((bits >> (4 * i)) & 15) * 17) as u8;
                }
            }
            let c0 = u16::from_le_bytes([block[8], block[9]]);
            let c1 = u16::from_le_bytes([block[10], block[11]]);
            let idx = u32::from_le_bytes([block[12], block[13], block[14], block[15]]);
            let (p0, p1) = (rgb565(c0), rgb565(c1));
            let mix = |wa: u32, wb: u32| -> [u8; 3] {
                core::array::from_fn(|k| {
                    ((wa * u32::from(p0[k]) + wb * u32::from(p1[k])) / (wa + wb)) as u8
                })
            };
            let palette = [p0, p1, mix(2, 1), mix(1, 2)];
            for i in 0..16 {
                let (x, y) = (bx + i % 4, by + i / 4);
                if x >= width || y >= height {
                    continue;
                }
                let colour = palette[((idx >> (2 * i)) & 3) as usize];
                let o = (y * width + x) * 4;
                rgba[o..o + 3].copy_from_slice(&colour);
                rgba[o + 3] = alpha[i];
            }
        }
    }
    Ok(Page {
        width,
        height,
        rgba,
    })
}

/// Read one face out of a `FeFont_X.fontbnd.dcx`'s bytes, keeping only glyphs `keep` accepts.
///
/// # Errors
///
/// [`Error::Format`] naming the first part of the file that does not match the layout.
pub fn parse(name: FaceName, bytes: &[u8], keep: impl Fn(u32) -> bool) -> Result<Face> {
    let data = dcx(bytes)?;
    let members = bnd4(&data)?;
    let stem = name.stem();
    let member = |wanted: &str| {
        members
            .iter()
            .find(|(n, _)| n == wanted)
            .map(|(_, b)| *b)
            .ok_or(Error::Format("a FeFont member is missing"))
    };
    let (line_height, all) = ccm(member(&format!("{stem}.ccm"))?)?;
    let glyphs: Vec<Glyph> = all.into_iter().filter(|g| keep(g.code)).collect();
    let page_count = glyphs.iter().map(|g| g.page + 1).max().unwrap_or(0);
    let mut pages = Vec::with_capacity(page_count);
    for page in 0..page_count {
        if !glyphs.iter().any(|g| g.page == page) {
            pages.push(Page {
                width: 0,
                height: 0,
                rgba: Vec::new(),
            });
            continue;
        }
        let suffix = if page == 0 { "_win64" } else { "" };
        let (w, h, fourcc, blocks) = tpf(member(&format!("{stem}_{page:04}{suffix}.tpf"))?)?;
        pages.push(decode(w, h, fourcc, blocks)?);
    }
    for g in &glyphs {
        let p = &pages[g.page];
        if usize::from(g.rect[2]) > p.width || usize::from(g.rect[3]) > p.height {
            return Err(Error::Format("a glyph region is outside its page"));
        }
    }
    Ok(Face {
        line_height,
        glyphs,
        pages,
    })
}

/// Read one face from the game directory `game` (the folder holding `DarkSoulsII.exe`).
///
/// # Errors
///
/// [`Error::Io`] when the file cannot be read, [`Error::Format`] when it does not parse.
pub fn load(game: &Path, name: FaceName, keep: impl Fn(u32) -> bool) -> Result<Face> {
    let path = game
        .join("font")
        .join("English")
        .join(format!("{}.fontbnd.dcx", name.stem()));
    let bytes = std::fs::read(path).map_err(Error::Io)?;
    parse(name, &bytes, keep)
}

/// The glyphs the panels ask for: Basic Latin and Latin-1, which is all imgui's default range is.
#[must_use]
pub fn latin(code: u32) -> bool {
    (0x20..=0xff).contains(&code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// The install on the machine the tests run on, if there is one.
    fn game() -> Option<PathBuf> {
        let home = std::env::var_os("HOME")?;
        let dir = PathBuf::from(home).join(
            ".local/share/Steam/steamapps/common/Dark Souls II Scholar of the First Sin/Game",
        );
        dir.join("font/English/FeFont_Small.fontbnd.dcx")
            .is_file()
            .then_some(dir)
    }

    #[test]
    fn both_faces_read_with_their_line_heights_and_every_printable_ascii() {
        let Some(game) = game() else {
            eprintln!("no DARK SOULS II install here; skipped");
            return;
        };
        for (name, line) in [(FaceName::Big, 41), (FaceName::Small, 28)] {
            let face = load(&game, name, latin).expect("face parses");
            assert_eq!(face.line_height, line, "{name:?}");
            let codes: Vec<u32> = face.glyphs.iter().map(|g| g.code).collect();
            for c in 0x20..0x7f {
                assert!(codes.contains(&c), "{name:?} lacks {c:#x}");
            }
            // Text cells are the full line tall.
            let a = face
                .glyphs
                .iter()
                .find(|g| g.code == u32::from('A'))
                .unwrap();
            assert_eq!(a.height(), line, "{name:?} 'A' {a:?}");
            // The digits share one pen step.
            let steps: Vec<i32> = ('0'..='9')
                .map(|d| {
                    face.glyphs
                        .iter()
                        .find(|g| g.code == u32::from(d))
                        .unwrap()
                        .pen_step()
                })
                .collect();
            assert!(steps.windows(2).all(|w| w[0] == w[1]), "{name:?} {steps:?}");
        }
    }

    #[test]
    fn a_letter_has_light_fill_and_dark_outline() {
        let Some(game) = game() else {
            return;
        };
        let face = load(&game, FaceName::Small, latin).unwrap();
        let a = face
            .glyphs
            .iter()
            .find(|g| g.code == u32::from('A'))
            .unwrap();
        let page = &face.pages[a.page];
        let (mut light, mut dark) = (0, 0);
        for y in usize::from(a.rect[1])..usize::from(a.rect[3]) {
            for x in usize::from(a.rect[0])..usize::from(a.rect[2]) {
                let o = (y * page.width + x) * 4;
                if page.rgba[o + 3] > 128 {
                    if page.rgba[o] > 150 {
                        light += 1;
                    } else if page.rgba[o] < 40 {
                        dark += 1;
                    }
                }
            }
        }
        assert!(light > 20 && dark > 20, "light {light} dark {dark}");
    }

    #[test]
    fn a_file_that_is_not_a_dcx_is_refused() {
        assert!(matches!(
            parse(FaceName::Small, b"not a font", latin),
            Err(Error::Format(_))
        ));
    }
}
