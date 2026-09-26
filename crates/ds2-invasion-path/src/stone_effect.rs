//! The Prism Stone as a candle: no sparkles, a flickering point light, a pulsing glow. Built from
//! the game's own files.
//!
//! # What the stock stone is made of
//!
//! `f0000833.ffx` .. `f0000839.ffx` in `Game/sfx/sfx9999.ffxbnd.dcx` are `DLsE` effect
//! definitions (`SoulsFormats`' `FFXDLSE`; `scripts/ds2-ffx.py tree --id 833` prints one). Each is a
//! root that spawns child effect [`GLOW_CHILD_EFFECT`]: ONE billboard (action 59, texture 132) --
//! the glow on the ground -- which in turn spawns child effect [`SPARKLE_CHILD_EFFECT`], a particle
//! cluster (action 15000, texture 133). That cluster is the sparkles.
//!
//! # What the copy changes
//!
//! Two splices, each a whole object swapped for another of a different length, with every
//! enclosing object's length field moved by the difference (a `ParamList`'s two ints count params,
//! not bytes, and the file header holds no total size):
//!
//! - **The sparkle child becomes a light.** Effect [`LIGHT_DONOR_EFFECT`] (181) is a point light
//!   and nothing else: one `Param type 37` naming [`GLOW_CHILD_EFFECT`] whose appearance argument
//!   is action [`POINT_LIGHT_ACTION`] (11001). That object is copied byte for byte over the stone's
//!   one `Param type 37` naming [`SPARKLE_CHILD_EFFECT`], with its colours set to the stone's
//!   own and its radius and flicker set to [`CANDLE`]. The glow's action 14 spawns every
//!   non-empty child slot, so the light lives as long as the glow and sits at its origin. The
//!   donor's class table has to equal the stone's -- the copied objects name their classes by
//!   index -- and a mismatch is refused, not assumed away.
//! - **The glow's colour loops.** The billboard's colour argument (action 59 arg 8) is a type-19
//!   one-key colour curve. It becomes type 20 -- the same layout with a cyclic loop -- with the
//!   five keys of [`GLOW_FLICKER`], each the stock colour scaled. A cyclic curve needs two or more
//!   keys to compile as a loop at all.
//!
//! `DS2-STONE-LIGHT-FLICKER.md` in `docs/` (PR #142) has the static RE behind both, the table of
//! donor values and the run that has to show them; `docs/ffx-actions.md` section 7 has the curve
//! types and the 11001 light.
//!
//! # Why derived at runtime rather than shipped
//!
//! The result is a copy of the game's own assets with a few objects moved. Reading them out of
//! the player's own install keeps game data out of this repository and out of the download, and
//! it covers all seven colours with one rule instead of seven checked-in blobs.
//!
//! Everything here is bytes in, bytes out, and host-tested. `crate::sfx::register_effect` is the
//! half that hands the result to the engine.

/// The child effect id that is the sparkle cluster in all seven Prism Stone effects.
pub const SPARKLE_CHILD_EFFECT: i32 = 2032;

/// The child effect template that is the stone's glow, and also 181's light: the template takes
/// its appearance (billboard or light) as an argument.
pub const GLOW_CHILD_EFFECT: i32 = 2101;

/// The stock effect whose one child is a point light and nothing else. Its light object is what
/// replaces the sparkles.
pub const LIGHT_DONOR_EFFECT: u32 = 181;

/// `SfxFxDrawEntityHostPointLight`, the action id of a point light.
pub const POINT_LIGHT_ACTION: i32 = 11001;

/// The billboard action: the stone's glow.
pub const BILLBOARD_ACTION: i32 = 59;

/// Added to a stock Prism Stone id to name its derived copy: `833 -> 25833`.
///
/// Above 9999, so the spawn skips the remaster `+20000`/`+10000` probe; below 30000, so the
/// per-bundle "already tried" bitset stops every spawn from rescanning every bundle; and no
/// `f00258xx.ffx` exists in any shipped `Game/sfx` bundle (checked 2026-09-25).
pub const STRIPPED_ID_OFFSET: u32 = 25_000;

/// The id a derived copy of `stock` is registered under.
pub const fn stripped_id(stock: u32) -> u32 {
    STRIPPED_ID_OFFSET + stock
}

/// Where the effect bundle lives, relative to the directory holding `DarkSoulsII.exe`.
pub const BUNDLE_RELATIVE_PATH: &str = "sfx/sfx9999.ffxbnd.dcx";

/// The light's radius and flicker, written over 181's values. A starting point, not measured.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candle {
    /// Arg 2, the radius curve's one key (181: 10.0).
    pub radius: f32,
    /// Arg 6, the shortest flicker period in 1/30 s ticks (181: 5).
    pub period_min: i32,
    /// Arg 7, the longest flicker period (181: 10).
    pub period_max: i32,
    /// Arg 8, the brightness floor each flicker dips to (181: 0.8).
    pub floor: f32,
}

/// What the derived stones' lights use.
pub const CANDLE: Candle = Candle {
    radius: 4.0,
    period_min: 3,
    period_max: 9,
    floor: 0.6,
};

/// The glow's looping colour: `(time in seconds, factor on the stock colour)`. The last key repeats
/// the first so the loop has no seam.
pub const GLOW_FLICKER: [(f32, f32); 5] = [
    (0.0, 1.0),
    (4.0 / 30.0, 0.8),
    (6.0 / 30.0, 0.95),
    (10.0 / 30.0, 0.75),
    (14.0 / 30.0, 1.0),
];

const DCX_PAYLOAD_OFFSET: usize = 0x4c;
const BND4_HEADER_SIZE: u64 = 0x40;
const BND4_ENTRY_SIZE: u64 = 36;
/// `class u16 + version i32 + length i32`.
const OBJECT_HEADER: usize = 10;
/// Where the class table's `u16` count sits: magic 4, version 4, two zero i32s, a zero byte, i32 1.
const CLASS_TABLE_AT: usize = 4 + 4 + 8 + 1 + 4;
/// `FXSerializableEffect`'s fixed body before its first child: `i32 0, id, 0, 0, 2; i16 0`.
const EFFECT_BODY_SKIP: usize = 0x14 + 2;
const CHILD_EFFECT_PARAM_TYPE: i32 = 37;
const ACTION_PARAM_TYPE: i32 = 38;
const INT_PARAM_TYPE: i32 = 1;
const FLOAT_PARAM_TYPE: i32 = 7;
const FLOAT_CURVE_PARAM_TYPE: i32 = 11;
/// A vec4 linear curve, `LoopCategoryNone`.
const COLOUR_CURVE_PARAM_TYPE: i32 = 19;
/// The same layout as [`COLOUR_CURVE_PARAM_TYPE`], `LoopCategoryCyclic`.
const LOOPING_COLOUR_CURVE_PARAM_TYPE: i32 = 20;
/// The billboard's colour argument.
const GLOW_COLOUR_ARG: usize = 8;
/// A one-key colour curve: header, type, count, one `FXTick`, one `FXColorRGBA`.
const ONE_KEY_COLOUR_CURVE_LEN: usize = OBJECT_HEADER + 8 + TICK_LEN + COLOUR_LEN;
const TICK_LEN: usize = OBJECT_HEADER + 4;
const COLOUR_LEN: usize = OBJECT_HEADER + 16;

const EFFECT_CLASS: &str = "FXSerializableEffect";
const VECTOR_CLASS: &str = "DLVector";
const PARAM_LIST_CLASS: &str = "FXSerializableParamList";
const PARAM_CLASS: &str = "FXSerializableParam";
const ACTION_CLASS: &str = "FXSerializableAction";
const PRIMITIVE_PREFIX: &str = "FXSerializablePrimitive<";
const TICK_CLASS: &str = "FXSerializablePrimitive<FXTick>";
const COLOUR_CLASS: &str = "FXSerializablePrimitive<FXColorRGBA>";
const FLOAT_CLASS: &str = "FXSerializablePrimitive<dl_float32>";
const INT_CLASS: &str = "FXSerializablePrimitive<dl_int32>";

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

fn f32_at(bytes: &[u8], at: usize) -> Option<f32> {
    Some(f32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn put(bytes: &mut [u8], at: usize, value: [u8; 4]) -> Result<(), String> {
    bytes
        .get_mut(at..at + 4)
        .ok_or_else(|| format!("write at {at:#x} is outside the object"))?
        .copy_from_slice(&value);
    Ok(())
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

/// One serialized object: `{u16 class_index + 1; i32 version; i32 length_including_header}`.
#[derive(Clone, Debug)]
struct Node {
    at: usize,
    len: usize,
    class: String,
    parent: Option<usize>,
    /// A `Param`'s or `Action`'s first `i32` (its type or id).
    kind: Option<i32>,
    /// The first raw `i32` after that: a child effect id (37), an action id (38), a key count.
    first: Option<i32>,
}

/// Every object in a `DLsE` file, in file order, the same walk as `scripts/ds2-ffx.py`'s `Tree`.
///
/// Inside a body, raw `i32`s and child objects interleave; a child is recognised by a class
/// index in range, a version 1..5 and a length that fits the parent. A `DLVector` is
/// `{u16 class; i32 count; i32 x count}` with no length field, so it is walked over and not
/// recorded: it can never be an ancestor.
struct Tree {
    classes: Vec<String>,
    root: usize,
    nodes: Vec<Node>,
}

/// The class names, and where the class table ends (which is where the root object starts).
fn class_table(ffx: &[u8]) -> Result<(Vec<String>, usize), String> {
    if ffx.get(..6) != Some(b"DLsE\x01\x03") {
        return Err("not a DS2 DLsE effect".to_owned());
    }
    let mut at = CLASS_TABLE_AT;
    let count = u16_at(ffx, at).ok_or("class table is truncated")?;
    at += 2;
    let mut classes = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        let length = u32_at(ffx, at).ok_or("class table is truncated")? as usize;
        let name = ffx
            .get(at + 4..at + 4 + length)
            .ok_or("class table is truncated")?;
        classes.push(String::from_utf8_lossy(name).into_owned());
        at += 4 + length;
    }
    Ok((classes, at))
}

impl Tree {
    fn parse(ffx: &[u8]) -> Result<Self, String> {
        let (classes, at) = class_table(ffx)?;
        let mut tree = Self {
            classes,
            root: at,
            nodes: Vec::new(),
        };
        let end = tree.walk(ffx, at, ffx.len(), None)?;
        if end != ffx.len() {
            return Err(format!(
                "the root object ends at {end:#x}, the file at {:#x}",
                ffx.len()
            ));
        }
        if tree.nodes.first().map(|n| n.class.as_str()) != Some(EFFECT_CLASS) {
            return Err("the root object is not an Effect".to_owned());
        }
        Ok(tree)
    }

    fn header(&self, ffx: &[u8], at: usize, end: usize) -> Option<(usize, usize)> {
        if at + OBJECT_HEADER > end {
            return None;
        }
        let class = usize::from(u16_at(ffx, at)?);
        let version = i32_at(ffx, at + 2)?;
        let length = usize::try_from(i32_at(ffx, at + 6)?).ok()?;
        if (1..=self.classes.len()).contains(&class)
            && (1..=5).contains(&version)
            && (OBJECT_HEADER..=end - at).contains(&length)
        {
            Some((class - 1, length))
        } else {
            None
        }
    }

    fn vector_end(&self, ffx: &[u8], at: usize, end: usize) -> Option<usize> {
        let class = self.classes.iter().position(|c| c == VECTOR_CLASS)? + 1;
        if at + 6 > end || usize::from(u16_at(ffx, at)?) != class {
            return None;
        }
        let count = usize::try_from(i32_at(ffx, at + 2)?).ok()?;
        let stop = at + 6 + 4 * count;
        (stop <= end).then_some(stop)
    }

    fn walk(
        &mut self,
        ffx: &[u8],
        at: usize,
        end: usize,
        parent: Option<usize>,
    ) -> Result<usize, String> {
        if let Some(stop) = self.vector_end(ffx, at, end) {
            return Ok(stop);
        }
        let (class, len) = self
            .header(ffx, at, end)
            .ok_or_else(|| format!("no object at {at:#x}"))?;
        let class = self.classes[class].clone();
        let index = self.nodes.len();
        let stop = at + len;
        let mut body = at + OBJECT_HEADER;
        let mut kind = None;
        if class == PARAM_CLASS || class == ACTION_CLASS {
            kind = Some(i32_at(ffx, body).ok_or("param is truncated")?);
            body += 4;
        }
        self.nodes.push(Node {
            at,
            len,
            class: class.clone(),
            parent,
            kind,
            first: None,
        });
        if class == EFFECT_CLASS {
            body += EFFECT_BODY_SKIP;
            body = self.walk(ffx, body, stop, Some(index))?;
            while body + 1 < stop {
                body = self.walk(ffx, body, stop, Some(index))?;
            }
            return Ok(stop);
        }
        if class.starts_with(PRIMITIVE_PREFIX) {
            return Ok(stop);
        }
        let mut raw_seen = false;
        while body < stop {
            if self.header(ffx, body, stop).is_some() || self.vector_end(ffx, body, stop).is_some()
            {
                body = self.walk(ffx, body, stop, Some(index))?;
                raw_seen = true;
                continue;
            }
            if stop - body >= 4 {
                if !raw_seen {
                    self.nodes[index].first = i32_at(ffx, body);
                    raw_seen = true;
                }
                body += 4;
            } else {
                body = stop;
            }
        }
        Ok(stop)
    }

    fn children(&self, parent: usize) -> impl Iterator<Item = usize> + '_ {
        (0..self.nodes.len()).filter(move |&i| self.nodes[i].parent == Some(parent))
    }

    fn is_descendant(&self, mut node: usize, of: usize) -> bool {
        while let Some(parent) = self.nodes[node].parent {
            if parent == of {
                return true;
            }
            node = parent;
        }
        false
    }

    /// Every `Param` of this `kind` whose first raw `i32` is `first`.
    fn params(&self, kind: i32, first: i32) -> Vec<usize> {
        (0..self.nodes.len())
            .filter(|&i| {
                let n = &self.nodes[i];
                n.class == PARAM_CLASS && n.kind == Some(kind) && n.first == Some(first)
            })
            .collect()
    }

    /// The `index`-th param of the argument list of the action param `action`.
    fn arg(&self, action: usize, index: usize) -> Result<usize, String> {
        let list = self
            .children(action)
            .find(|&c| self.nodes[c].class == PARAM_LIST_CLASS)
            .ok_or("action param has no ParamList")?;
        self.children(list)
            .filter(|&c| self.nodes[c].class == PARAM_CLASS)
            .nth(index)
            .ok_or_else(|| format!("action param has no arg {index}"))
    }

    /// The one primitive of `class` directly under `param`, checking the param's type.
    fn primitive(&self, param: usize, kind: i32, class: &str) -> Result<usize, String> {
        if self.nodes[param].kind != Some(kind) {
            return Err(format!(
                "param at {:#x} is type {:?}, not {kind}",
                self.nodes[param].at, self.nodes[param].kind
            ));
        }
        let found: Vec<usize> = self
            .children(param)
            .filter(|&c| self.nodes[c].class == class)
            .collect();
        let [one] = found[..] else {
            return Err(format!(
                "param at {:#x} holds {} {class}, not one",
                self.nodes[param].at,
                found.len()
            ));
        };
        Ok(one)
    }

    /// Every object that strictly contains `[at, at + len)`: the ones whose lengths move.
    fn ancestors(&self, at: usize, len: usize) -> Vec<usize> {
        (0..self.nodes.len())
            .filter(|&i| {
                let n = &self.nodes[i];
                n.at < at && at + len <= n.at + n.len
            })
            .collect()
    }
}

/// `ffx` with the object at `at` (`old_len` bytes, which must be an object the walk found) replaced
/// by `new`, and every enclosing object's length moved by the difference.
///
/// The result is re-walked end to end before it is returned.
fn splice(ffx: &[u8], at: usize, old_len: usize, new: &[u8]) -> Result<Vec<u8>, String> {
    let tree = Tree::parse(ffx)?;
    if !tree.nodes.iter().any(|n| n.at == at && n.len == old_len) {
        return Err(format!("no {old_len:#x}-byte object at {at:#x} to splice"));
    }
    let delta = i64::try_from(new.len()).map_err(|e| e.to_string())?
        - i64::try_from(old_len).map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(ffx.len() + new.len());
    out.extend_from_slice(&ffx[..at]);
    out.extend_from_slice(new);
    out.extend_from_slice(&ffx[at + old_len..]);
    for ancestor in tree.ancestors(at, old_len) {
        let node = &tree.nodes[ancestor];
        let grown = i32::try_from(i64::try_from(node.len).map_err(|e| e.to_string())? + delta)
            .map_err(|e| e.to_string())?;
        // An ancestor starts before `at`, so its header did not move.
        put(&mut out, node.at + 6, grown.to_le_bytes())?;
    }
    Tree::parse(&out).map_err(|why| format!("spliced file does not re-walk: {why}"))?;
    Ok(out)
}

/// The one object in `tree` that matches, or an error naming how many did.
fn one(found: &[usize], what: &str) -> Result<usize, String> {
    match found {
        [single] => Ok(*single),
        _ => Err(format!("expected one {what}, found {}", found.len())),
    }
}

/// The stock stone's billboard colour: the glow's action-59 arg 8, a one-key type-19 curve.
/// Returns the curve node and its one key's `(tick object, colour object)` offsets.
fn glow_colour(tree: &Tree, ffx: &[u8]) -> Result<(usize, usize, usize), String> {
    let glows = tree.params(CHILD_EFFECT_PARAM_TYPE, GLOW_CHILD_EFFECT);
    let billboards: Vec<usize> = tree
        .params(ACTION_PARAM_TYPE, BILLBOARD_ACTION)
        .into_iter()
        .filter(|&b| glows.iter().any(|&g| tree.is_descendant(b, g)))
        .collect();
    let billboard = one(&billboards, "billboard (action 59) inside the glow")?;
    let curve = tree.arg(billboard, GLOW_COLOUR_ARG)?;
    let node = &tree.nodes[curve];
    if node.kind != Some(COLOUR_CURVE_PARAM_TYPE)
        || node.first != Some(1)
        || node.len != ONE_KEY_COLOUR_CURVE_LEN
    {
        return Err(format!(
            "glow colour at {:#x} is type {:?} with {:?} keys and {:#x} bytes, not a one-key type \
             {COLOUR_CURVE_PARAM_TYPE}",
            node.at, node.kind, node.first, node.len
        ));
    }
    let tick = tree.primitive(curve, COLOUR_CURVE_PARAM_TYPE, TICK_CLASS)?;
    let colour = tree.primitive(curve, COLOUR_CURVE_PARAM_TYPE, COLOUR_CLASS)?;
    let (tick, colour) = (tree.nodes[tick].at, tree.nodes[colour].at);
    if tick != node.at + OBJECT_HEADER + 8 || colour != tick + TICK_LEN {
        return Err(format!(
            "glow colour at {:#x} is not laid out as one key",
            node.at
        ));
    }
    if f32_at(ffx, colour + OBJECT_HEADER + 12).is_none() {
        return Err("glow colour is truncated".to_owned());
    }
    Ok((curve, tick, colour))
}

/// The glow's looping colour curve: the stock curve's header, type 20, the keys of
/// [`GLOW_FLICKER`], each key's `FXTick` and `FXColorRGBA` objects copied from the stock key.
fn looping_glow(ffx: &[u8], curve_at: usize, tick: usize, colour: usize) -> Vec<u8> {
    let rgba: Vec<f32> = (0..4)
        .map(|c| f32_at(ffx, colour + OBJECT_HEADER + 4 * c).unwrap_or(0.0))
        .collect();
    let len = OBJECT_HEADER + 8 + GLOW_FLICKER.len() * (TICK_LEN + COLOUR_LEN);
    let mut out = Vec::with_capacity(len);
    out.extend_from_slice(&ffx[curve_at..curve_at + 6]);
    out.extend_from_slice(&(len as i32).to_le_bytes());
    out.extend_from_slice(&LOOPING_COLOUR_CURVE_PARAM_TYPE.to_le_bytes());
    out.extend_from_slice(&(GLOW_FLICKER.len() as i32).to_le_bytes());
    for (time, factor) in GLOW_FLICKER {
        out.extend_from_slice(&ffx[tick..tick + OBJECT_HEADER]);
        out.extend_from_slice(&time.to_le_bytes());
        out.extend_from_slice(&ffx[colour..colour + OBJECT_HEADER]);
        for channel in &rgba {
            out.extend_from_slice(&(channel * factor).to_le_bytes());
        }
    }
    out
}

/// 181's light object, copied, with the stone's colour and [`CANDLE`] written in.
fn candle_light(donor: &[u8], rgb: [f32; 3]) -> Result<Vec<u8>, String> {
    let tree = Tree::parse(donor)?;
    let children = tree.params(CHILD_EFFECT_PARAM_TYPE, GLOW_CHILD_EFFECT);
    let lights = tree.params(ACTION_PARAM_TYPE, POINT_LIGHT_ACTION);
    let holders: Vec<usize> = children
        .into_iter()
        .filter(|&c| lights.iter().any(|&l| tree.is_descendant(l, c)))
        .collect();
    let holder = one(&holders, "child effect 2101 carrying an 11001 light")?;
    let light = one(
        &lights
            .into_iter()
            .filter(|&l| tree.is_descendant(l, holder))
            .collect::<Vec<_>>(),
        "11001 light inside that child",
    )?;
    let base = tree.nodes[holder].at;
    let mut out = donor[base..base + tree.nodes[holder].len].to_vec();
    let value = |arg: usize, kind: i32, class: &str| -> Result<usize, String> {
        let primitive = tree.primitive(tree.arg(light, arg)?, kind, class)?;
        Ok(tree.nodes[primitive].at + OBJECT_HEADER - base)
    };
    // Args 0 and 1, the two colours: the stone's rgb, 181's own alpha (.251 and .059).
    for arg in [0, 1] {
        let at = value(arg, COLOUR_CURVE_PARAM_TYPE, COLOUR_CLASS)?;
        for (channel, component) in rgb.iter().enumerate() {
            put(&mut out, at + 4 * channel, component.to_le_bytes())?;
        }
    }
    let radius = value(2, FLOAT_CURVE_PARAM_TYPE, FLOAT_CLASS)?;
    put(&mut out, radius, CANDLE.radius.to_le_bytes())?;
    let period_min = value(6, INT_PARAM_TYPE, INT_CLASS)?;
    put(&mut out, period_min, CANDLE.period_min.to_le_bytes())?;
    let period_max = value(7, INT_PARAM_TYPE, INT_CLASS)?;
    put(&mut out, period_max, CANDLE.period_max.to_le_bytes())?;
    let floor = value(8, FLOAT_PARAM_TYPE, FLOAT_CLASS)?;
    put(&mut out, floor, CANDLE.floor.to_le_bytes())?;
    Ok(out)
}

/// The stock stone `ffx` with its sparkle child replaced by `donor`'s point light in the stone's
/// colour, its glow's colour turned into a looping curve, and its own id set to `new_id`.
///
/// `donor` is effect [`LIGHT_DONOR_EFFECT`] out of the same bundle. Refuses unless the two class
/// tables are identical, exactly one `Param type 37` names [`SPARKLE_CHILD_EFFECT`], exactly one
/// billboard sits in the glow with a one-key type-19 colour, and the donor holds exactly one
/// light child -- so a different layout is an error rather than a guess. The effect's own id has
/// to match the id it is registered under: with the two different, the engine resolves the
/// resource and then builds nothing (measured in game).
///
/// # Errors
///
/// When either file is not a DS2 `DLsE` effect that walks end to end, or any of the refusals
/// above applies.
pub fn light_stone(ffx: &[u8], donor: &[u8], new_id: u32) -> Result<Vec<u8>, String> {
    // Compared before the donor is walked: its objects name classes by index into this table.
    if class_table(ffx)?.0 != class_table(donor)?.0 {
        return Err("the light donor's class table is not the stone's".to_owned());
    }
    let tree = Tree::parse(ffx)?;
    let sparkle = one(
        &tree.params(CHILD_EFFECT_PARAM_TYPE, SPARKLE_CHILD_EFFECT),
        "child-effect param naming 2032",
    )?;
    let (curve, tick, colour) = glow_colour(&tree, ffx)?;
    let rgb = [0, 1, 2].map(|c| f32_at(ffx, colour + OBJECT_HEADER + 4 * c).unwrap_or(0.0));
    let (sparkle_at, sparkle_len) = (tree.nodes[sparkle].at, tree.nodes[sparkle].len);
    let (curve_at, curve_len) = (tree.nodes[curve].at, tree.nodes[curve].len);
    // The later object first, so the earlier one's offset still holds for the second splice.
    if curve_at + curve_len > sparkle_at {
        return Err("the glow colour does not sit before the sparkle child".to_owned());
    }
    let glow = looping_glow(ffx, curve_at, tick, colour);
    let light = candle_light(donor, rgb)?;
    let lit = splice(ffx, sparkle_at, sparkle_len, &light)?;
    let mut out = splice(&lit, curve_at, curve_len, &glow)?;
    // Root object: header, then `i32 0; i32 id`.
    put(
        &mut out,
        tree.root + OBJECT_HEADER + 4,
        new_id.to_le_bytes(),
    )?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(class: u16, body: &[u8]) -> Vec<u8> {
        let mut out = class.to_le_bytes().to_vec();
        out.extend_from_slice(&1i32.to_le_bytes());
        out.extend_from_slice(&((OBJECT_HEADER + body.len()) as i32).to_le_bytes());
        out.extend_from_slice(body);
        out
    }

    fn ints(values: &[i32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    /// A minimal `DLsE`: the real class order, a root Effect holding one `ParamList` with one
    /// child-effect param naming `child`.
    fn synthetic(child: i32) -> Vec<u8> {
        let mut out = b"DLsE\x01\x03\0\0".to_vec();
        out.extend_from_slice(&[0; 9]);
        out.extend_from_slice(&1i32.to_le_bytes());
        let classes = [EFFECT_CLASS, VECTOR_CLASS, PARAM_LIST_CLASS, PARAM_CLASS];
        out.extend_from_slice(&(classes.len() as u16).to_le_bytes());
        for name in classes {
            out.extend_from_slice(&(name.len() as u32).to_le_bytes());
            out.extend_from_slice(name.as_bytes());
        }
        let args = object(3, &ints(&[0, 0]));
        let mut param = ints(&[CHILD_EFFECT_PARAM_TYPE, child]);
        param.extend_from_slice(&args);
        let mut list = ints(&[1, 1]);
        list.extend_from_slice(&object(4, &param));
        let mut effect = ints(&[0, 833, 0, 0, 2]);
        effect.extend_from_slice(&0i16.to_le_bytes());
        effect.extend_from_slice(&2u16.to_le_bytes());
        effect.extend_from_slice(&0i32.to_le_bytes());
        effect.extend_from_slice(&object(3, &list));
        out.extend_from_slice(&object(1, &effect));
        out
    }

    #[test]
    fn a_splice_moves_every_enclosing_length_and_nothing_else() {
        let stock = synthetic(SPARKLE_CHILD_EFFECT);
        let tree = Tree::parse(&stock).unwrap();
        let param = one(
            &tree.params(CHILD_EFFECT_PARAM_TYPE, SPARKLE_CHILD_EFFECT),
            "sparkle",
        )
        .unwrap();
        let (at, len) = (tree.nodes[param].at, tree.nodes[param].len);
        let mut bigger = ints(&[CHILD_EFFECT_PARAM_TYPE, 2101, 7, 7, 7]);
        bigger.extend_from_slice(&object(3, &ints(&[0, 0])));
        let new = object(4, &bigger);
        let out = splice(&stock, at, len, &new).unwrap();
        assert_eq!(out.len(), stock.len() + 12);
        let after = Tree::parse(&out).unwrap();
        let root = &after.nodes[0];
        assert_eq!(root.at + root.len, out.len());
        // The ParamList around the param grew by the same twelve bytes; the param's own
        // ParamList (a child, not an ancestor) did not.
        let list = after.nodes[param - 1].clone();
        assert_eq!(list.len, tree.nodes[param - 1].len + 12);
        assert_eq!(after.nodes[param + 1].len, tree.nodes[param + 1].len);
        assert_eq!(after.params(CHILD_EFFECT_PARAM_TYPE, 2101).len(), 1);
        assert!(splice(&stock, at + 1, len, &new).is_err());
    }

    #[test]
    fn refuses_a_donor_with_another_class_table_or_no_sparkle() {
        let stock = synthetic(SPARKLE_CHILD_EFFECT);
        let mut donor = stock.clone();
        // `DLVector` -> `DLVectoR`: same length, different class name.
        let at = donor.windows(8).position(|w| w == b"DLVector").unwrap();
        donor[at + 7] = b'R';
        let error = light_stone(&stock, &donor, 25_833).unwrap_err();
        assert!(error.contains("class table"), "{error}");
        let error = light_stone(&synthetic(2101), &synthetic(2101), 25_833).unwrap_err();
        assert!(error.contains("2032"), "{error}");
        assert!(light_stone(b"not an effect", &stock, 25_833).is_err());
    }

    #[test]
    fn stripped_ids_stay_in_the_window_the_engine_resolves_directly() {
        for stock in ds2_rva::PRISM_STONE_SFX_IDS {
            let id = stripped_id(stock);
            assert!(id > 9_999 && id < 30_000, "{id}");
        }
    }

    fn installed_bundle() -> Option<Vec<u8>> {
        let home = std::env::var_os("HOME")?;
        let bundle = std::path::Path::new(&home)
            .join(".local/share/Steam/steamapps/common/Dark Souls II Scholar of the First Sin/Game")
            .join(BUNDLE_RELATIVE_PATH);
        std::fs::read(bundle).ok()
    }

    /// Where the value of `param`'s one `class` primitive sits.
    fn value(tree: &Tree, param: usize, kind: i32, class: &str) -> usize {
        tree.nodes[tree.primitive(param, kind, class).unwrap()].at + OBJECT_HEADER
    }

    /// Against the real install, when this machine has one: the name lookup, the member inflate
    /// and both splices on all seven colours, with the bytes the game actually loads.
    #[test]
    fn lights_all_seven_colours_from_the_installed_bundle() {
        let Some(dcx) = installed_bundle() else {
            return;
        };
        let donor = member_from_bundle(&dcx, LIGHT_DONOR_EFFECT).unwrap();
        for stock in ds2_rva::PRISM_STONE_SFX_IDS {
            let ffx = member_from_bundle(&dcx, stock).unwrap();
            let stock_tree = Tree::parse(&ffx).unwrap();
            let (_, _, stock_colour) = glow_colour(&stock_tree, &ffx).unwrap();
            let base: Vec<f32> = (0..4)
                .map(|c| f32_at(&ffx, stock_colour + OBJECT_HEADER + 4 * c).unwrap())
                .collect();

            // 839's sparkle object is 0xd1e bytes, the other six 0xce6.
            let sparkle = one(
                &stock_tree.params(CHILD_EFFECT_PARAM_TYPE, SPARKLE_CHILD_EFFECT),
                "2032",
            )
            .unwrap();
            let sparkle_len = stock_tree.nodes[sparkle].len;
            assert_eq!(sparkle_len, if stock == 839 { 0xd1e } else { 0xce6 });

            let out = light_stone(&ffx, &donor, stripped_id(stock)).unwrap();
            // Re-walks to its end (`parse` refuses otherwise), sized by the two splices: the
            // 0x37a-byte light over the sparkles, the glow colour 0x3a -> 0xda.
            let tree = Tree::parse(&out).unwrap();
            assert_eq!(out.len(), ffx.len() + 0x37a - sparkle_len + 0xa0, "{stock}");
            if stock == 833 {
                assert_eq!(out.len(), 0x124b);
            }
            // The light object is 181's, byte for byte, but for the ten floats and ints written
            // into it (two rgb triples, radius, two periods, floor). It sits where the sparkles
            // were, moved on by the glow colour's growth.
            let spliced = &out[stock_tree.nodes[sparkle].at + 0xa0..][..0x37a];
            let donor_tree = Tree::parse(&donor).unwrap();
            let holder = one(
                &donor_tree.params(CHILD_EFFECT_PARAM_TYPE, 2101),
                "181 child",
            )
            .unwrap();
            let original = &donor[donor_tree.nodes[holder].at..][..0x37a];
            let changed = spliced.iter().zip(original).filter(|(a, b)| a != b).count();
            assert!(changed <= 4 * 10, "{stock}: {changed} light bytes changed");
            assert_eq!(
                i32_at(&out, tree.root + OBJECT_HEADER + 4),
                Some(stripped_id(stock) as i32)
            );
            assert!(
                tree.params(CHILD_EFFECT_PARAM_TYPE, SPARKLE_CHILD_EFFECT)
                    .is_empty(),
                "{stock}: 2032 is still there"
            );
            let light = one(&tree.params(ACTION_PARAM_TYPE, POINT_LIGHT_ACTION), "light").unwrap();

            // The light: the stone's rgb with 181's alphas, the candle radius and flicker.
            let colour_a = value(&tree, tree.arg(light, 0).unwrap(), 19, COLOUR_CLASS);
            let colour_b = value(&tree, tree.arg(light, 1).unwrap(), 19, COLOUR_CLASS);
            for (c, channel) in base.iter().take(3).enumerate() {
                assert_eq!(f32_at(&out, colour_a + 4 * c), Some(*channel));
                assert_eq!(f32_at(&out, colour_b + 4 * c), Some(*channel));
            }
            assert!((f32_at(&out, colour_a + 12).unwrap() - 0.251).abs() < 1e-3);
            assert!((f32_at(&out, colour_b + 12).unwrap() - 0.059).abs() < 1e-3);
            let radius = value(&tree, tree.arg(light, 2).unwrap(), 11, FLOAT_CLASS);
            assert_eq!(f32_at(&out, radius), Some(4.0));
            let p6 = value(&tree, tree.arg(light, 6).unwrap(), 1, INT_CLASS);
            let p7 = value(&tree, tree.arg(light, 7).unwrap(), 1, INT_CLASS);
            let p8 = value(&tree, tree.arg(light, 8).unwrap(), 7, FLOAT_CLASS);
            assert_eq!(i32_at(&out, p6), Some(3));
            assert_eq!(i32_at(&out, p7), Some(9));
            assert_eq!(f32_at(&out, p8), Some(0.6));

            // The glow: action 59's arg 8 is a type-20 curve with five keys that loop.
            let billboard = one(&tree.params(ACTION_PARAM_TYPE, BILLBOARD_ACTION), "59").unwrap();
            let curve = tree.arg(billboard, GLOW_COLOUR_ARG).unwrap();
            assert_eq!(
                tree.nodes[curve].kind,
                Some(LOOPING_COLOUR_CURVE_PARAM_TYPE)
            );
            assert_eq!(tree.nodes[curve].first, Some(5));
            assert_eq!(tree.nodes[curve].len, 0xda);
            let ticks: Vec<usize> = tree
                .children(curve)
                .filter(|&c| tree.nodes[c].class == TICK_CLASS)
                .collect();
            let colours: Vec<usize> = tree
                .children(curve)
                .filter(|&c| tree.nodes[c].class == COLOUR_CLASS)
                .collect();
            assert_eq!((ticks.len(), colours.len()), (5, 5));
            for (key, (time, factor)) in GLOW_FLICKER.iter().enumerate() {
                let tick_at = tree.nodes[ticks[key]].at + OBJECT_HEADER;
                assert_eq!(f32_at(&out, tick_at), Some(*time));
                let colour_at = tree.nodes[colours[key]].at + OBJECT_HEADER;
                for (c, channel) in base.iter().enumerate() {
                    assert_eq!(f32_at(&out, colour_at + 4 * c), Some(channel * factor));
                }
            }
        }
    }
}
