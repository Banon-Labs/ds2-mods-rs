//! The Prism Stone without the sparkles that bleed off it, built from the game's own file.
//!
//! # What the stock stone is made of
//!
//! `f0000833.ffx` .. `f0000839.ffx` in `Game/sfx/sfx9999.ffxbnd.dcx` are `DLsE` effect
//! definitions (`SoulsFormats`' `FFXDLSE`; `scripts/ds2-ffx.py tree --id 833` prints one). Each is a
//! root that spawns child effect `2101`: ONE billboard (action 59, texture 132) -- the glow on the
//! ground -- which in turn spawns child effect [`SPARKLE_CHILD_EFFECT`], a particle cluster
//! (action 15000, texture 133) thrown out of a four-sided-pyramid emitter and pushed by an
//! acceleration. That cluster is the sparkles.
//!
//! Zeroing the one `Param type 37` that names `2032` leaves the glow and removes the sparkles.
//! Shipped files already use `0` for an unused child slot (seven of the eight slots beside it are
//! `0`), so this is a value the engine is known to accept. Watched in game on 2026-09-25: a stock
//! stone beside a stripped one, the glow on both, the sparkles on only the stock one.
//!
//! # Why derived at runtime rather than shipped
//!
//! The stripped bytes are a copy of the game's own asset with a few bytes changed. Reading them
//! out of the player's own install keeps game data out of this repository and out of the
//! download, and it covers all seven colours with one rule instead of seven checked-in blobs.
//!
//! Everything here is bytes in, bytes out, and host-tested. `crate::sfx::register_effect` is the
//! half that hands the result to the engine.

/// The child effect id that is the sparkle cluster in all seven Prism Stone effects.
pub const SPARKLE_CHILD_EFFECT: i32 = 2032;

/// Added to a stock Prism Stone id to name its stripped copy: `833 -> 25833`.
///
/// Above 9999, so the spawn skips the remaster `+20000`/`+10000` probe; below 30000, so the
/// per-bundle "already tried" bitset stops every spawn from rescanning every bundle; and no
/// `f00258xx.ffx` exists in any shipped `Game/sfx` bundle (checked 2026-09-25).
pub const STRIPPED_ID_OFFSET: u32 = 25_000;

/// The id a stripped copy of `stock` is registered under.
pub const fn stripped_id(stock: u32) -> u32 {
    STRIPPED_ID_OFFSET + stock
}

/// Where the effect bundle lives, relative to the directory holding `DarkSoulsII.exe`.
pub const BUNDLE_RELATIVE_PATH: &str = "sfx/sfx9999.ffxbnd.dcx";

const DCX_PAYLOAD_OFFSET: usize = 0x4c;
const BND4_HEADER_SIZE: u64 = 0x40;
const BND4_ENTRY_SIZE: u64 = 36;
const PARAM_CLASS: &str = "FXSerializableParam";
const PARAM_VERSION: i32 = 2;
const CHILD_EFFECT_PARAM_TYPE: i32 = 37;
/// `class u16 + version i32 + length i32`.
const OBJECT_HEADER: usize = 10;

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn i32_at(bytes: &[u8], at: usize) -> Option<i32> {
    Some(i32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

fn inflate(bytes: &[u8], expected: usize) -> Result<Vec<u8>, String> {
    let out = miniz_oxide::inflate::decompress_to_vec_zlib(bytes)
        .map_err(|error| format!("zlib stream did not inflate: {error:?}"))?;
    if out.len() != expected {
        return Err(format!(
            "inflated to {} bytes, header says {expected}",
            out.len()
        ));
    }
    Ok(out)
}

/// The stock `.ffx` for `stock_id`, out of the bytes of `sfx9999.ffxbnd.dcx`.
///
/// DS2 `DCX`/`DFLT` around a `BND4` with 36-byte entries whose members are bare zlib streams
/// when their two size fields differ. The member is found by NAME (`...\f0000833.ffx`), which is
/// how the engine itself matches a bundle member to an id -- the BND4 entry id is not used.
///
/// # Errors
///
/// When the bytes are not a DS2 `DCX`/`BND4` of this layout, a zlib stream does not inflate to
/// its declared size, or no member is named for `stock_id`.
pub fn member_from_bundle(dcx: &[u8], stock_id: u32) -> Result<Vec<u8>, String> {
    if dcx.get(..4) != Some(b"DCX\0") {
        return Err("not a DCX file".to_owned());
    }
    let declared = dcx
        .get(0x1c..0x20)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize)
        .ok_or("DCX header is truncated")?;
    let bnd = inflate(
        dcx.get(DCX_PAYLOAD_OFFSET..).ok_or("DCX has no payload")?,
        declared,
    )?;
    if bnd.get(..4) != Some(b"BND4") {
        return Err("DCX payload is not a BND4".to_owned());
    }
    let count = u32_at(&bnd, 0x0c).ok_or("BND4 header is truncated")?;
    if u64_at(&bnd, 0x10) != Some(BND4_HEADER_SIZE) || u64_at(&bnd, 0x20) != Some(BND4_ENTRY_SIZE) {
        return Err("BND4 header/entry sizes are not the 0x40/36 this reads".to_owned());
    }
    let wanted = format!("f{stock_id:07}.ffx");
    for index in 0..count as usize {
        let at = BND4_HEADER_SIZE as usize + index * BND4_ENTRY_SIZE as usize;
        let stored = u64_at(&bnd, at + 8).ok_or("BND4 entry is truncated")? as usize;
        let inflated = u64_at(&bnd, at + 16).ok_or("BND4 entry is truncated")? as usize;
        let offset = u32_at(&bnd, at + 24).ok_or("BND4 entry is truncated")? as usize;
        let name_at = u32_at(&bnd, at + 32).ok_or("BND4 entry is truncated")? as usize;
        let name = utf16z(&bnd, name_at).ok_or("BND4 name is unreadable")?;
        if !name.to_ascii_lowercase().ends_with(&wanted) {
            continue;
        }
        let data = bnd
            .get(offset..offset + stored)
            .ok_or("BND4 member points outside the file")?;
        return if stored == inflated {
            Ok(data.to_vec())
        } else {
            inflate(data, inflated)
        };
    }
    Err(format!("{wanted} is not in the bundle"))
}

fn utf16z(bytes: &[u8], at: usize) -> Option<String> {
    let mut units = Vec::new();
    let mut here = at;
    loop {
        let unit = u16_at(bytes, here)?;
        if unit == 0 {
            break;
        }
        units.push(unit);
        here += 2;
    }
    String::from_utf16(&units).ok()
}

/// Where the root object starts, and the one-based index of [`PARAM_CLASS`] in the class table.
fn layout(ffx: &[u8]) -> Result<(usize, u16), String> {
    if ffx.get(..6) != Some(b"DLsE\x01\x03") {
        return Err("not a DS2 DLsE effect".to_owned());
    }
    // magic 4, version 4, two zero i32s, a zero byte, i32 1 -- then the i16 class count.
    let mut at = 4 + 4 + 8 + 1 + 4;
    let count = u16_at(ffx, at).ok_or("class table is truncated")?;
    at += 2;
    let mut param_class = None;
    for index in 0..count {
        let length = u32_at(ffx, at).ok_or("class table is truncated")? as usize;
        let name = ffx
            .get(at + 4..at + 4 + length)
            .ok_or("class table is truncated")?;
        if name == PARAM_CLASS.as_bytes() {
            param_class = Some(index + 1);
        }
        at += 4 + length;
    }
    Ok((at, param_class.ok_or("no FXSerializableParam class")?))
}

/// The stock stone with its sparkle child cut and its own id set to `new_id`.
///
/// Refuses unless exactly one `Param type 37` names [`SPARKLE_CHILD_EFFECT`], so a different
/// layout -- another game version, another effect id -- is an error rather than a guess. The
/// effect's own id has to match the id it is registered under: with the two different, the
/// engine resolves the resource and then builds nothing (measured in game).
///
/// # Errors
///
/// When `ffx` is not a DS2 `DLsE` effect, or it does not hold exactly one child-effect param
/// naming [`SPARKLE_CHILD_EFFECT`].
pub fn strip_sparkles(ffx: &[u8], new_id: u32) -> Result<Vec<u8>, String> {
    let (root, param_class) = layout(ffx)?;
    let mut hits = Vec::new();
    for at in root..=ffx.len().saturating_sub(OBJECT_HEADER + 8) {
        if u16_at(ffx, at) == Some(param_class)
            && i32_at(ffx, at + 2) == Some(PARAM_VERSION)
            && i32_at(ffx, at + OBJECT_HEADER) == Some(CHILD_EFFECT_PARAM_TYPE)
            && i32_at(ffx, at + OBJECT_HEADER + 4) == Some(SPARKLE_CHILD_EFFECT)
        {
            let length = i32_at(ffx, at + 6).unwrap_or(0);
            if length > OBJECT_HEADER as i32 && at + length as usize <= ffx.len() {
                hits.push(at);
            }
        }
    }
    let [hit] = hits[..] else {
        return Err(format!(
            "expected one child-effect param naming {SPARKLE_CHILD_EFFECT}, found {}",
            hits.len()
        ));
    };
    let mut out = ffx.to_vec();
    out[hit + OBJECT_HEADER + 4..hit + OBJECT_HEADER + 8].copy_from_slice(&0i32.to_le_bytes());
    // Root object: header, then `i32 0; i32 id`.
    let id_at = root + OBJECT_HEADER + 4;
    out.get_mut(id_at..id_at + 4)
        .ok_or("root object is truncated")?
        .copy_from_slice(&new_id.to_le_bytes());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal `DLsE`: three classes, a root Effect whose body holds child-effect params.
    fn synthetic(child: i32, copies: usize) -> Vec<u8> {
        let mut out = b"DLsE\x01\x03\0\0".to_vec();
        out.extend_from_slice(&[0; 9]);
        out.extend_from_slice(&1i32.to_le_bytes());
        let classes = ["FXSerializableEffect", "DLVector", PARAM_CLASS];
        out.extend_from_slice(&(classes.len() as u16).to_le_bytes());
        for name in classes {
            out.extend_from_slice(&(name.len() as u32).to_le_bytes());
            out.extend_from_slice(name.as_bytes());
        }
        let mut body = Vec::new();
        body.extend_from_slice(&0i32.to_le_bytes());
        body.extend_from_slice(&833i32.to_le_bytes());
        for _ in 0..copies {
            body.extend_from_slice(&3u16.to_le_bytes());
            body.extend_from_slice(&PARAM_VERSION.to_le_bytes());
            body.extend_from_slice(&18i32.to_le_bytes());
            body.extend_from_slice(&CHILD_EFFECT_PARAM_TYPE.to_le_bytes());
            body.extend_from_slice(&child.to_le_bytes());
        }
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&5i32.to_le_bytes());
        out.extend_from_slice(&((OBJECT_HEADER + body.len()) as i32).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    #[test]
    fn cuts_the_sparkle_child_and_renumbers_the_effect() {
        let stock = synthetic(SPARKLE_CHILD_EFFECT, 1);
        let stripped = strip_sparkles(&stock, stripped_id(833)).unwrap();
        let (root, _) = layout(&stripped).unwrap();
        assert_eq!(i32_at(&stripped, root + OBJECT_HEADER + 4), Some(25_833));
        assert_eq!(i32_at(&stripped, stripped.len() - 4), Some(0));
        let changed = stock.iter().zip(&stripped).filter(|(a, b)| a != b).count();
        assert!(
            changed <= 8,
            "only the id and the child reference may change, {changed} did"
        );
    }

    #[test]
    fn refuses_a_file_without_exactly_one_sparkle_child() {
        assert!(strip_sparkles(&synthetic(2101, 1), 25_833).is_err());
        assert!(strip_sparkles(&synthetic(SPARKLE_CHILD_EFFECT, 2), 25_833).is_err());
        assert!(strip_sparkles(b"not an effect", 25_833).is_err());
    }

    #[test]
    fn stripped_ids_stay_in_the_window_the_engine_resolves_directly() {
        for stock in ds2_rva::PRISM_STONE_SFX_IDS {
            let id = stripped_id(stock);
            assert!(id > 9_999 && id < 30_000, "{id}");
        }
    }

    /// Against the real install, when this machine has one: the name lookup, the member inflate
    /// and the sparkle cut on all seven colours, with the bytes the game actually loads.
    #[test]
    fn strips_all_seven_colours_from_the_installed_bundle() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let bundle = std::path::Path::new(&home)
            .join(".local/share/Steam/steamapps/common/Dark Souls II Scholar of the First Sin/Game")
            .join(BUNDLE_RELATIVE_PATH);
        let Ok(dcx) = std::fs::read(&bundle) else {
            return;
        };
        for stock in ds2_rva::PRISM_STONE_SFX_IDS {
            let ffx = member_from_bundle(&dcx, stock).unwrap();
            let stripped = strip_sparkles(&ffx, stripped_id(stock)).unwrap();
            assert_eq!(stripped.len(), ffx.len());
            let changed = ffx.iter().zip(&stripped).filter(|(a, b)| a != b).count();
            assert!(changed <= 8, "{stock}: {changed} bytes changed");
        }
    }
}
