//! The `.sl2` container, and the one edit that makes a foreign save loadable.
//!
//! # Why the DLL does this rather than a script
//!
//! DARK SOULS II writes the owning account's SteamID64, as ASCII hex, into the decrypted payload of
//! `USER_DATA000`. It compares that against the running account and refuses a mismatch -- which is
//! what "The save data was not loaded correctly" means when the path is otherwise fine. So a donor
//! save has to be rebound before it will load.
//!
//! The rebind belongs here because **this is the only place that already knows the answer**. The
//! detour on `ds2_rva::SAVE_DIR_BUILD` is handed the running account's Steam ID as its second
//! argument -- the game fetches it through a vtable slot and passes it in to build the folder name.
//! Nobody has to look their ID up, type it into a config, or get it wrong.
//!
//! # The container
//!
//! A stock BND4. Header, a table of `0x20`-byte entry headers at `0x40`, then payloads. Each
//! payload is
//!
//! ```text
//! [ 16-byte MD5 of everything after it ][ 16-byte CBC IV ][ AES-128-CBC ciphertext ]
//! ```
//!
//! and the MD5 covers the **ciphertext with its IV**, not the plaintext. Two consequences that are
//! easy to get wrong and produce a file rejected exactly like an unpatched one:
//!
//! * a patched entry must be re-encrypted under its **own original IV**, and
//! * the hash must be recomputed over `iv || ciphertext` afterwards.
//!
//! Reusing the IV also keeps every byte outside the patched blocks identical to the source, so a
//! diff between the two files shows the edit and nothing else.
//!
//! # The key is not typed in from memory
//!
//! It is the value the game keeps in `.rdata` as a UTF-16 string in front of the property name
//! `SaveLoad2.Title.EncryptionKey`. `scripts/ds2-sl2.py --key-from-image` re-derives it from the
//! shipped binary, which is the check to run if a patch ever moves it.

use md5::{Digest, Md5};

/// AES-128 key for SOTFS `.sl2` payloads. See the module docs: read out of the game image by
/// `scripts/ds2-sl2.py --key-from-image`, not remembered.
const KEY: [u8; 16] = [
    0x59, 0x9F, 0x9B, 0x69, 0x96, 0x40, 0xA5, 0x52, 0x36, 0xEE, 0x2D, 0x70, 0x83, 0x5E, 0xC7, 0x44,
];

/// A SteamID64 for an individual account is `0x0110000100000000 + accountid`, so its hex rendering
/// is the literal `01100001` followed by eight hex digits. Anchoring on that eight-character prefix
/// is what stops a scan reporting every run of sixteen hex digits in a multi-megabyte payload.
const STEAM_ID_PREFIX: &[u8] = b"01100001";

/// A SteamID64 in hex is always this long, which is what makes the patch length-preserving: no
/// section size changes, no offset table rewrite.
pub const STEAM_ID_LEN: usize = 16;

/// What went wrong, in terms a log line can name.
#[derive(Debug)]
pub enum Sl2Error {
    /// Not a BND4 container at all.
    NotBnd4,
    /// The entry table points outside the file.
    Truncated,
    /// A payload was not a whole number of AES blocks.
    NotBlockAligned,
    /// No Steam ID anywhere in the decrypted payloads -- not the layout this knows.
    NoSteamId,
    /// The replacement was not sixteen hex characters.
    BadSteamId,
}

impl core::fmt::Display for Sl2Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let text = match self {
            Self::NotBnd4 => "not a BND4 container",
            Self::Truncated => "entry table points outside the file",
            Self::NotBlockAligned => "payload is not a whole number of AES blocks",
            Self::NoSteamId => "no Steam ID found in any payload",
            Self::BadSteamId => "replacement Steam ID is not 16 hex characters",
        };
        f.write_str(text)
    }
}

/// How many payloads a save holds, or why it is not a save.
///
/// **A structural check, not a decryption.** It reads the BND4 magic and walks the entry table,
/// which is enough to separate "this is a Dark Souls II save" from "this is a truncated file, a
/// zero-byte placeholder, or something else entirely" -- and it is all a caller needs before
/// deciding whether there is a character to act on. Decrypting the payloads would prove more and
/// costs an AES pass over the whole file; nothing that only wants to know the save is real should
/// pay for that.
///
/// The count is returned rather than a bare `bool` because zero entries is a structurally valid
/// BND4 that holds no character, and a caller that wants to refuse that can.
pub fn validate(save: &[u8]) -> Result<usize, Sl2Error> {
    entries(save).map(|entries| entries.len())
}

/// One BND4 entry: where its payload starts and how long it is.
struct Entry {
    offset: usize,
    size: usize,
}

/// Read the entry table. Deliberately tolerant of nothing: a bad offset is an error, not a skip.
fn entries(save: &[u8]) -> Result<Vec<Entry>, Sl2Error> {
    if save.len() < 0x40 || &save[..4] != b"BND4" {
        return Err(Sl2Error::NotBnd4);
    }
    let count = u32::from_le_bytes(
        save[0x0C..0x10]
            .try_into()
            .map_err(|_| Sl2Error::Truncated)?,
    );
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count as usize {
        let header = 0x40 + i * 0x20;
        if header + 0x20 > save.len() {
            return Err(Sl2Error::Truncated);
        }
        let size = u64::from_le_bytes(
            save[header + 8..header + 16]
                .try_into()
                .map_err(|_| Sl2Error::Truncated)?,
        ) as usize;
        let offset = u32::from_le_bytes(
            save[header + 16..header + 20]
                .try_into()
                .map_err(|_| Sl2Error::Truncated)?,
        ) as usize;
        // A payload is the hash, the IV, and at least one cipher block.
        if size < 48 || offset.saturating_add(size) > save.len() {
            return Err(Sl2Error::Truncated);
        }
        out.push(Entry { offset, size });
    }
    Ok(out)
}

/// AES-128-CBC over whole blocks, in place. No padding: the payload length is the message length.
///
/// Written against the block cipher directly rather than through a padded-mode helper because the
/// data has no padding to strip -- asking a padded API to "unpad" a save payload would either
/// truncate it or fail on the last block.
fn cbc(data: &mut [u8], iv: &[u8; 16], encrypt: bool) -> Result<(), Sl2Error> {
    use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit, generic_array::GenericArray};

    if !data.len().is_multiple_of(16) {
        return Err(Sl2Error::NotBlockAligned);
    }
    let cipher = aes::Aes128::new(&GenericArray::from(KEY));
    let mut chain = *iv;
    let (blocks, rest) = data.as_chunks_mut::<16>();
    debug_assert!(rest.is_empty(), "length was checked above");
    for block in blocks {
        if encrypt {
            for (b, c) in block.iter_mut().zip(chain.iter()) {
                *b ^= *c;
            }
            let mut ga = GenericArray::from(*block);
            cipher.encrypt_block(&mut ga);
            block.copy_from_slice(&ga);
            // The ciphertext block just produced is what the next plaintext block is XORed with.
            chain = *block;
        } else {
            // Saved BEFORE decryption: CBC needs the previous CIPHERTEXT block, and this one is
            // about to be overwritten in place with its own plaintext.
            let previous = *block;
            let mut ga = GenericArray::from(*block);
            cipher.decrypt_block(&mut ga);
            block.copy_from_slice(&ga);
            for (b, c) in block.iter_mut().zip(chain.iter()) {
                *b ^= *c;
            }
            chain = previous;
        }
    }
    Ok(())
}

/// Every offset in `plain` where an ASCII Steam ID starts.
fn steam_ids(plain: &[u8]) -> Vec<usize> {
    let mut out = Vec::new();
    if plain.len() < STEAM_ID_LEN {
        return out;
    }
    for i in 0..=plain.len() - STEAM_ID_LEN {
        if &plain[i..i + STEAM_ID_PREFIX.len()] != STEAM_ID_PREFIX {
            continue;
        }
        if plain[i..i + STEAM_ID_LEN].iter().all(u8::is_ascii_hexdigit) {
            out.push(i);
        }
    }
    out
}

/// What [`rebind`] did, for the log.
#[derive(Debug, Default)]
pub struct Rebound {
    /// Steam IDs replaced. Every save examined has exactly one.
    pub replaced: usize,
    /// The ID that was in the file before, if any.
    pub previous: Option<String>,
}

/// Rewrite every Steam ID in `save` to `steam_id`, resealing each entry that changed.
///
/// `steam_id` is the running account's, as the game itself spells it -- sixteen lowercase hex
/// characters. Returns without touching the file if it already belongs to this account, which is
/// the ordinary case on the second launch of the same staged save.
pub fn rebind(save: &mut [u8], steam_id: &str) -> Result<Rebound, Sl2Error> {
    if steam_id.len() != STEAM_ID_LEN || !steam_id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Sl2Error::BadSteamId);
    }
    let table = entries(save)?;
    let mut report = Rebound::default();
    let mut seen_any = false;

    for entry in table {
        let iv: [u8; 16] = save[entry.offset + 16..entry.offset + 32]
            .try_into()
            .map_err(|_| Sl2Error::Truncated)?;
        let mut plain = save[entry.offset + 32..entry.offset + entry.size].to_vec();
        cbc(&mut plain, &iv, false)?;

        let hits = steam_ids(&plain);
        if hits.is_empty() {
            continue;
        }
        seen_any = true;
        let mut changed = false;
        for at in hits {
            let existing = &plain[at..at + STEAM_ID_LEN];
            if report.previous.is_none() {
                report.previous = Some(String::from_utf8_lossy(existing).into_owned());
            }
            if existing == steam_id.as_bytes() {
                continue;
            }
            plain[at..at + STEAM_ID_LEN].copy_from_slice(steam_id.as_bytes());
            report.replaced += 1;
            changed = true;
        }
        if !changed {
            continue;
        }
        // Re-encrypt under the ENTRY'S OWN IV, then re-hash over `iv || ciphertext`. Both halves
        // matter; a correct re-encryption with a stale hash is rejected exactly like no patch.
        cbc(&mut plain, &iv, true)?;
        save[entry.offset + 32..entry.offset + entry.size].copy_from_slice(&plain);
        let mut hasher = Md5::new();
        hasher.update(&save[entry.offset + 16..entry.offset + entry.size]);
        let digest = hasher.finalize();
        save[entry.offset..entry.offset + 16].copy_from_slice(&digest);
    }

    if !seen_any {
        return Err(Sl2Error::NoSteamId);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CBC must round-trip, or every save this touches is silently corrupted. Checked against a
    /// non-zero IV specifically: a bug that ignores the IV still round-trips with an all-zero one.
    #[test]
    fn cbc_round_trips() {
        let iv = [0x11u8; 16];
        let original: Vec<u8> = (0..64).map(|i| i as u8).collect();
        let mut data = original.clone();
        cbc(&mut data, &iv, true).unwrap();
        assert_ne!(data, original, "encryption did nothing");
        cbc(&mut data, &iv, false).unwrap();
        assert_eq!(data, original);
    }

    #[test]
    fn cbc_rejects_a_partial_block() {
        assert!(matches!(
            cbc(&mut [0u8; 17], &[0u8; 16], true),
            Err(Sl2Error::NotBlockAligned)
        ));
    }

    #[test]
    fn steam_ids_anchors_on_the_prefix() {
        assert_eq!(steam_ids(b"xx01100001526d6d84yy"), vec![2]);
        // Sixteen hex characters that are not a Steam ID must not match.
        assert!(steam_ids(b"deadbeefdeadbeef").is_empty());
    }

    #[test]
    fn rebind_refuses_a_bad_id() {
        assert!(matches!(
            rebind(&mut [], "nothex"),
            Err(Sl2Error::BadSteamId)
        ));
    }
}

#[cfg(test)]
mod validate_tests {
    use super::*;

    /// Anything that is not a BND4 container is refused by name.
    #[test]
    fn a_non_save_is_refused_rather_than_counted() {
        assert!(matches!(validate(b""), Err(Sl2Error::NotBnd4)));
        assert!(matches!(
            validate(b"not a save at all"),
            Err(Sl2Error::NotBnd4)
        ));
        // The right magic on a file too short to hold a header is still not a save.
        assert!(matches!(validate(b"BND4"), Err(Sl2Error::NotBnd4)));
    }

    /// A header claiming more entries than the file can hold is truncation, not a count.
    #[test]
    fn a_header_that_overruns_the_file_is_truncation() {
        let mut save = vec![0u8; 0x40];
        save[..4].copy_from_slice(b"BND4");
        save[0x0C..0x10].copy_from_slice(&99u32.to_le_bytes());
        assert!(matches!(validate(&save), Err(Sl2Error::Truncated)));
    }

    /// A well-formed header with no entries counts zero rather than erroring.
    ///
    /// Structurally a save, holding nothing -- which is a different answer from "not a save", and
    /// the reason this returns a count instead of a bool.
    #[test]
    fn an_empty_container_counts_zero() {
        let mut save = vec![0u8; 0x40];
        save[..4].copy_from_slice(b"BND4");
        save[0x0C..0x10].copy_from_slice(&0u32.to_le_bytes());
        assert!(matches!(validate(&save), Ok(0)));
    }
}
