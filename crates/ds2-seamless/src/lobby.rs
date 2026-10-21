//! What DARK SOULS II Seamless Co-op 0.0.5 writes on its Steam lobbies and what its invasion search
//! asks for, measured, so nothing else in this repo spells a hashed key inline.
//!
//! # How it was measured (2026-10-21)
//!
//! - `scripts/ds2-lobby-bands.py --dump`, a standalone Steamworks client as app 335300, reading every
//!   key of every lobby a `RequestLobbyList` returned.
//! - `scripts/frida/ds2sc-lobby-watch.js` in a live game: every `SetLobbyData` Seamless sent when the
//!   owner hosted with Pharros' Heliograph, and every filter its search sent when they used the
//!   Fragment of Malevolance.
//!
//! The key names are hashed per Seamless build (they are 64 hex digits, and none is the SHA-256 of
//! any plain name tried), so a new build needs them measured again; every one of them is a
//! constant here so that is one edit.
//!
//! # What a host publishes
//!
//! On `CreateLobby`, wherever they are: a join-pool key that is different for every co-op
//! password ([`JOIN_POOL_KEY_PREFIX`]), [`MARKER_KEY`] and [`FLAGS_KEY`]. Only in an area that can
//! be invaded (not Majula) it adds [`OPEN_KEY`], [`BRACKET_KEY`] and [`REGULATION_KEY`]. Petrified
//! Fingers changed none of it.
//!
//! # What an invasion search asks for
//!
//! One `RequestLobbyList`, run from Seamless's per-frame tick on the game's main thread
//! (`ds2sc.dll+0x756f3`, called from `DarkSoulsII.exe+0x84b56e`): [`OPEN_KEY`] `== "true"`,
//! [`BRACKET_KEY`] `==` the searcher's own bracket value, [`MARKER_KEY`] `==` [`MARKER_VALUE`],
//! [`REGULATION_KEY`] `==` the searcher's regulation value, worldwide distance, no count filter.
//! It joins the first lobby returned about 0.1 s after the answer.

/// The advertisement marker's key. Every Seamless master lobby carries it.
pub const MARKER_KEY: &str = "6b7fbc14d929b5568933d215d2d966b10492f0a15d99f8d088e1d1dc5d44751f";
/// The advertisement marker's value in 0.0.5.
pub const MARKER_VALUE: &str = "ds2_ynx3_seamless_master_lobby_precopy_a";

/// The open flag: `"true"` while the host can be invaded, `"false"` when not.
pub const OPEN_KEY: &str = "d00915e3f3206d533beef85d842594b3dfe07f0b16620f8a94773e1ce7e388bb";
/// The open flag's value on a host that can be invaded now.
pub const OPEN_VALUE: &str = "true";

/// The invasion bracket: `ds2sc_yui_nexus_session_search_005_bimm_<N>_6`, compared for equality by
/// the search. The value's shape is `ds2_invadable_hosts`' to parse.
pub const BRACKET_KEY: &str = "8c1e0aea8218b194b8598fc57ec01ccf4c5444e3522d367f1f8ec903f1d12ae5";

/// The regulation the host plays on. Only hosts with the searcher's own value can be invaded, so
/// this is the pool.
pub const REGULATION_KEY: &str = "404e9b227e020645fc8424868eee50ab0e87b0699ba0df674fa7aa0f1646185f";
/// The regulation value every 0.0.5 host and search measured carried. A census uses it until
/// Seamless has been seen writing or filtering on the key this session.
pub const REGULATION_VALUE_MEASURED: &str = "test_regulation_type";

/// A 32-digit binary string every master carries.
///
/// `00000000000000000000000010111111` is the most common. What its bits mean is not established;
/// Steam compares it as a decimal number, which is what makes it usable to split a lobby list.
pub const FLAGS_KEY: &str = "6cfe9fe6db999d53f8ce637935e0a128729c8dd84b5407a65e6c1587c4729044";

/// The join pool: `005_<64 hex>_6`, a different hex for every co-op password, with
/// [`JOIN_POOL_VALUE`] as its value. Not part of an invasion search.
pub const JOIN_POOL_KEY_PREFIX: &str = "005_";
/// The join pool key's value on every host measured.
pub const JOIN_POOL_VALUE: &str =
    "a3fdd484bf34fc5a2ab99f05f0ad04f5358fb3bbb7e8f06ea5d8a9ff4f1ca5d1";

/// Steam's distance filter value Seamless's search sends: `k_ELobbyDistanceFilterWorldwide`.
pub const SEARCH_DISTANCE: i32 = 3;

/// The goods id of the Fragment of Malevolance, the item that starts an invasion search. Measured
/// 2026-10-21: using it from the bag made Seamless search (`scripts/frida/bracket-scout.js`).
pub const FRAGMENT_OF_MALEVOLANCE_GOODS_ID: u32 = 100_000_002;
