//! The `SpEffect` catalog the selector scrolls.
//!
//! The file is `data/speffect-visual-catalog.json`, written by
//! `scripts/ds2-speffect-visual.py <extracted regulation> --catalog <path>`. It lists the
//! `SpEffect*.emevd` events whose every instruction draws (Spawn SFX, Apply `ChrFullBodySFX`) or
//! sets a length, and nothing else -- see `docs/DS2-SPEFFECT-VISUAL.md`. Its shape is
//! er-net-effects' master catalog (`schema_version`, `kind`, `source`, `field_index`, `effects`
//! with `id` / `name` / `vfx` / `tags` / `fields`), so the two catalogs read the same way.
//!
//! The DLL embeds the file ([`EMBEDDED_CATALOG`]) and reads a copy beside the game
//! ([`CATALOG_FILE_NAME`]) in its place when one is there, so a hand-edited catalog needs no
//! rebuild.

use serde_json::Value;

/// The catalog built into the DLL.
pub const EMBEDDED_CATALOG: &str = include_str!("../data/speffect-visual-catalog.json");

/// The `kind` the generator writes. A file with another kind is refused rather than half-read.
pub const CATALOG_KIND: &str = "ds2-speffect-visual-master-catalog";

/// The file name beside `DarkSoulsII.exe` that replaces the embedded catalog.
pub const CATALOG_FILE_NAME: &str = "ds2-net-effects-catalog.json";

/// One selectable effect.
#[derive(Clone, Debug, PartialEq)]
pub struct CatalogEntry {
    /// The `SpEffect` id: the event id in its `SpEffect*.emevd`.
    pub id: i32,
    /// The display name the generator derived.
    pub name: String,
    /// Spawn SFX ids.
    pub sfx: Vec<i32>,
    /// `ChrFullBodySfxParam` ids.
    pub full_body_sfx: Vec<i32>,
    /// Dummy polys the sfx spawn on.
    pub dummy_poly: Vec<i32>,
    /// `Set SpEffect Length` in seconds; `None` when the event sets none.
    pub duration: Option<f32>,
    /// Tags: the emevd band, and the doc's findings (`recommended`, `avoid`, `exe-literal`, ...).
    pub tags: Vec<String>,
}

impl CatalogEntry {
    /// Whether the entry carries `tag`.
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }
}

/// Why a catalog text was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogError {
    /// Not JSON.
    Json(String),
    /// JSON, but not this catalog: the `kind` or the `effects` array is wrong.
    Shape(String),
}

fn ints(value: Option<&Value>) -> Vec<i32> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_i64)
                .filter_map(|v| i32::try_from(v).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Read a catalog text.
///
/// # Errors
///
/// [`CatalogError`] when the text is not JSON, names another `kind`, has no `effects` array, or
/// holds an entry without an integer `id` in `i32` range. An entry with no `name` gets
/// `SpEffect <id>`.
pub fn parse_catalog(text: &str) -> Result<Vec<CatalogEntry>, CatalogError> {
    let root: Value =
        serde_json::from_str(text).map_err(|error| CatalogError::Json(error.to_string()))?;
    let kind = root.get("kind").and_then(Value::as_str).unwrap_or_default();
    if kind != CATALOG_KIND {
        return Err(CatalogError::Shape(format!(
            "kind is {kind:?}, not {CATALOG_KIND:?}"
        )));
    }
    let effects = root
        .get("effects")
        .and_then(Value::as_array)
        .ok_or_else(|| CatalogError::Shape("no effects array".to_string()))?;
    let mut out = Vec::with_capacity(effects.len());
    for (index, effect) in effects.iter().enumerate() {
        let id = effect
            .get("id")
            .and_then(Value::as_i64)
            .and_then(|v| i32::try_from(v).ok())
            .ok_or_else(|| CatalogError::Shape(format!("effects[{index}] has no i32 id")))?;
        let fields = effect.get("fields");
        let field = |name: &str| fields.and_then(|f| f.get(name));
        out.push(CatalogEntry {
            id,
            name: effect
                .get("name")
                .and_then(Value::as_str)
                .filter(|n| !n.trim().is_empty())
                .map_or_else(|| format!("SpEffect {id}"), str::to_string),
            sfx: ints(field("sfx")),
            full_body_sfx: ints(field("fullBodySfxParam")),
            dummy_poly: ints(field("dummyPoly")),
            duration: field("effectEndurance")
                .and_then(Value::as_f64)
                .map(|v| v as f32),
            tags: effect
                .get("tags")
                .and_then(Value::as_array)
                .map(|t| {
                    t.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The file the DLL ships reads, and holds what the generator wrote into it.
    #[test]
    fn the_embedded_catalog_reads() {
        let entries = parse_catalog(EMBEDDED_CATALOG).expect("the shipped catalog parses");
        assert!(!entries.is_empty());
        let sfx = entries
            .iter()
            .find(|e| e.id == 140_001_010)
            .expect("the default visual effect is in the catalog");
        assert_eq!(sfx.sfx, vec![659]);
        assert_eq!(sfx.dummy_poly, vec![249]);
        assert_eq!(sfx.duration, Some(1.5));
        assert!(sfx.has_tag("recommended"));
        let full_body = entries
            .iter()
            .find(|e| e.id == 901_100)
            .expect("the full-body entry");
        assert_eq!(full_body.full_body_sfx, vec![400_010]);
        assert_eq!(full_body.duration, None, "no length in its event");
        let mut ids: Vec<i32> = entries.iter().map(|e| e.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), entries.len(), "no id twice");
    }

    #[test]
    fn another_kind_or_no_effects_is_refused() {
        assert!(matches!(parse_catalog("nope"), Err(CatalogError::Json(_))));
        assert!(matches!(
            parse_catalog(r#"{"kind":"other","effects":[]}"#),
            Err(CatalogError::Shape(_))
        ));
        assert!(matches!(
            parse_catalog(&format!(r#"{{"kind":"{CATALOG_KIND}"}}"#)),
            Err(CatalogError::Shape(_))
        ));
        assert!(matches!(
            parse_catalog(&format!(
                r#"{{"kind":"{CATALOG_KIND}","effects":[{{"name":"x"}}]}}"#
            )),
            Err(CatalogError::Shape(_))
        ));
    }

    #[test]
    fn a_bare_entry_gets_a_fallback_name_and_empty_fields() {
        let entries = parse_catalog(&format!(
            r#"{{"kind":"{CATALOG_KIND}","effects":[{{"id":7}}]}}"#
        ))
        .expect("parses");
        assert_eq!(entries[0].name, "SpEffect 7");
        assert!(entries[0].sfx.is_empty());
        assert_eq!(entries[0].duration, None);
    }
}
