//! One entry out of DARK SOULS II's `GameDataEbl` archive, read from the player's install.
//!
//! The menu atlases the panels borrow art from (`waku_03`, `In-game_01`) are not loose files like
//! the fonts: they sit in `.tpf`s inside `BND4`s inside `GameDataEbl.bdt`. This is the Rust half
//! of `scripts/ds2-ebl.py`, which is the reference and says where each step comes from:
//!
//! 1. `GameDataEbl.bhd` is RSA-encrypted in 256-byte blocks with the public key beside it,
//!    `GameDataKeyCode.pem`; each block decrypts to 255 bytes. Decrypting with a public key is a
//!    plain modular exponentiation, done here with Montgomery multiplication rather than a new
//!    dependency.
//! 2. The decrypted `BHD5` header is a bucket table of `u32 path hash, u32 size, u64 offset,
//!    i64 salted hash offset, i64 AES key offset` records.
//! 3. An entry is read from the `.bdt` at its offset, and nothing else of the `.bdt` is read.
//!    Entries with an AES key record are refused, as the script refuses them: none of the menu
//!    atlases is one.
//!
//! Nothing is written anywhere; nothing is shipped.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::fefont::Error;

type Result<T> = core::result::Result<T, Error>;

/// Limbs in a 2048-bit number, 32 bits each, least significant first.
const LIMBS: usize = 64;
// RSA block size in bytes, and what each block decrypts to.
const BLOCK: usize = 256;
const PLAIN: usize = 255;

type Big = [u32; LIMBS];

fn from_be(bytes: &[u8]) -> Big {
    let mut out = [0u32; LIMBS];
    for (i, chunk) in bytes.rchunks(4).enumerate().take(LIMBS) {
        let mut word = [0u8; 4];
        word[4 - chunk.len()..].copy_from_slice(chunk);
        out[i] = u32::from_be_bytes(word);
    }
    out
}

fn to_be(value: &Big) -> [u8; BLOCK] {
    let mut out = [0u8; BLOCK];
    for (i, limb) in value.iter().enumerate() {
        let at = BLOCK - 4 * (i + 1);
        out[at..at + 4].copy_from_slice(&limb.to_be_bytes());
    }
    out
}

/// `a >= b`.
fn geq(a: &Big, b: &Big) -> bool {
    for i in (0..LIMBS).rev() {
        if a[i] != b[i] {
            return a[i] > b[i];
        }
    }
    true
}

/// `a -= b`, wrapping.
fn sub_in_place(a: &mut Big, b: &Big) {
    let mut borrow = 0i64;
    for i in 0..LIMBS {
        let d = i64::from(a[i]) - i64::from(b[i]) - borrow;
        a[i] = d as u32;
        borrow = i64::from(d < 0);
    }
}

/// Modular exponentiation with an odd modulus, by Montgomery multiplication.
struct Montgomery {
    n: Big,
    /// `-n^-1 mod 2^32`.
    n0: u32,
    /// `R^2 mod n` with `R = 2^(32 * LIMBS)`.
    r2: Big,
}

impl Montgomery {
    fn new(n: Big) -> Self {
        // Newton's iteration doubles the correct low bits each round: five rounds reach 32.
        let mut inv = 1u32;
        for _ in 0..5 {
            inv = inv.wrapping_mul(2u32.wrapping_sub(n[0].wrapping_mul(inv)));
        }
        // R^2 mod n by doubling 1 a total of 2 * 32 * LIMBS times, reducing as it goes.
        let mut r2 = [0u32; LIMBS];
        r2[0] = 1;
        for _ in 0..2 * 32 * LIMBS {
            let top = r2[LIMBS - 1] >> 31;
            for i in (1..LIMBS).rev() {
                r2[i] = (r2[i] << 1) | (r2[i - 1] >> 31);
            }
            r2[0] <<= 1;
            if top == 1 || geq(&r2, &n) {
                sub_in_place(&mut r2, &n);
            }
        }
        Self {
            n,
            n0: inv.wrapping_neg(),
            r2,
        }
    }

    /// `a * b * R^-1 mod n` (CIOS).
    fn mul(&self, a: &Big, b: &Big) -> Big {
        let mut t = [0u32; LIMBS + 2];
        for &ai in a {
            let mut carry = 0u64;
            for j in 0..LIMBS {
                let x = u64::from(t[j]) + u64::from(ai) * u64::from(b[j]) + carry;
                t[j] = x as u32;
                carry = x >> 32;
            }
            let x = u64::from(t[LIMBS]) + carry;
            t[LIMBS] = x as u32;
            t[LIMBS + 1] = (x >> 32) as u32;

            let m = t[0].wrapping_mul(self.n0);
            let x = u64::from(t[0]) + u64::from(m) * u64::from(self.n[0]);
            let mut carry = x >> 32;
            for j in 1..LIMBS {
                let x = u64::from(t[j]) + u64::from(m) * u64::from(self.n[j]) + carry;
                t[j - 1] = x as u32;
                carry = x >> 32;
            }
            let x = u64::from(t[LIMBS]) + carry;
            t[LIMBS - 1] = x as u32;
            t[LIMBS] = t[LIMBS + 1] + (x >> 32) as u32;
            t[LIMBS + 1] = 0;
        }
        let mut out = [0u32; LIMBS];
        out.copy_from_slice(&t[..LIMBS]);
        if t[LIMBS] != 0 || geq(&out, &self.n) {
            sub_in_place(&mut out, &self.n);
        }
        out
    }

    /// `base ^ exponent mod n`, for `base < n`; the exponent is big-endian bytes.
    fn pow(&self, base: &Big, exponent: &[u8]) -> Big {
        let mut one = [0u32; LIMBS];
        one[0] = 1;
        let base_m = self.mul(base, &self.r2);
        let mut acc = self.mul(&one, &self.r2);
        for byte in exponent {
            for bit in (0..8).rev() {
                acc = self.mul(&acc, &acc);
                if (byte >> bit) & 1 == 1 {
                    acc = self.mul(&acc, &base_m);
                }
            }
        }
        self.mul(&acc, &one)
    }
}

fn base64(text: &str) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\r' | b'\n' | b' ' => continue,
            _ => return Err(Error::Format("the .pem is not base64")),
        };
        acc = ((acc << 6) | u32::from(v)) & 0xffff;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok(out)
}

/// One DER element at `at`: its body and where the next one starts.
fn der(buf: &[u8], mut at: usize) -> Result<(&[u8], usize)> {
    let short = || Error::Format("short DER");
    at += 1; // tag
    let first = *buf.get(at).ok_or_else(short)?;
    at += 1;
    let mut len = usize::from(first);
    if first & 0x80 != 0 {
        len = 0;
        for _ in 0..(first & 0x7f) {
            len = (len << 8) | usize::from(*buf.get(at).ok_or_else(short)?);
            at += 1;
        }
    }
    let body = buf.get(at..at + len).ok_or_else(short)?;
    Ok((body, at + len))
}

/// `(modulus, exponent)` out of a PKCS#1 `RSA PUBLIC KEY` PEM: two DER INTEGERs in a SEQUENCE.
fn public_key(pem: &str) -> Result<(Vec<u8>, Vec<u8>)> {
    let body: String = pem.lines().filter(|l| !l.contains("-----")).collect();
    let bytes = base64(&body)?;
    let (sequence, _) = der(&bytes, 0)?;
    let (modulus, next) = der(sequence, 0)?;
    let (exponent, _) = der(sequence, next)?;
    // A DER INTEGER carries a leading zero to stay positive.
    let modulus = modulus.iter().skip_while(|&&b| b == 0).copied().collect();
    Ok((modulus, exponent.to_vec()))
}

/// Decrypt a `.bhd` with its `.pem`.
fn decrypt_bhd(bhd: &[u8], pem: &str) -> Result<Vec<u8>> {
    if !bhd.len().is_multiple_of(BLOCK) {
        return Err(Error::Format("the .bhd is not whole RSA blocks"));
    }
    let (modulus, exponent) = public_key(pem)?;
    if modulus.len() != BLOCK {
        return Err(Error::Format("the .pem's modulus is not 2048 bits"));
    }
    let rsa = Montgomery::new(from_be(&modulus));
    let mut out = Vec::with_capacity(bhd.len() / BLOCK * PLAIN);
    for block in bhd.chunks(BLOCK) {
        let plain = to_be(&rsa.pow(&from_be(block), &exponent));
        out.extend_from_slice(&plain[BLOCK - PLAIN..]);
    }
    if out.get(..4) != Some(b"BHD5") {
        return Err(Error::Format("the .bhd did not decrypt to a BHD5 header"));
    }
    Ok(out)
}

/// Where an entry sits in the `.bdt`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Entry {
    size: usize,
    offset: u64,
    encrypted: bool,
}

/// Find `hash` in a decrypted `BHD5` header.
fn find(bhd5: &[u8], hash: u32) -> Result<Entry> {
    let short = || Error::Format("short BHD5");
    let u32_at = |at: usize| {
        bhd5.get(at..at + 4)
            .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
            .ok_or_else(short)
    };
    let u64_at = |at: usize| {
        bhd5.get(at..at + 8)
            .map(|s| u64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]))
            .ok_or_else(short)
    };
    let buckets = u32_at(0x10)?;
    let table = u32_at(0x14)? as usize;
    if buckets == 0 {
        return Err(Error::Format("BHD5 has no buckets"));
    }
    let bucket = (hash % buckets) as usize;
    let count = u32_at(table + bucket * 8)? as usize;
    let first = u32_at(table + bucket * 8 + 4)? as usize;
    // Records are `<IIQqq`: hash, size, offset, salted hash offset, AES key offset.
    for index in 0..count {
        let at = first + index * 32;
        if u32_at(at)? == hash {
            return Ok(Entry {
                size: u32_at(at + 4)? as usize,
                offset: u64_at(at + 8)?,
                encrypted: u64_at(at + 24)? != 0,
            });
        }
    }
    Err(Error::Format("no archive entry has that hash"))
}

/// `GameDataEbl` with its header decrypted once, for reading any number of entries.
pub struct Archive {
    header: Vec<u8>,
    bdt: std::path::PathBuf,
}

impl Archive {
    /// Decrypt the header of the archive in the game directory `game`.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] when a file cannot be read, [`Error::Format`] when the header does not
    /// decrypt.
    pub fn open(game: &Path) -> Result<Self> {
        let bhd = std::fs::read(game.join("GameDataEbl.bhd")).map_err(Error::Io)?;
        let pem = std::fs::read_to_string(game.join("GameDataKeyCode.pem")).map_err(Error::Io)?;
        Ok(Self {
            header: decrypt_bhd(&bhd, &pem)?,
            bdt: game.join("GameDataEbl.bdt"),
        })
    }

    /// The raw bytes of the entry whose path hash is `hash`.
    ///
    /// # Errors
    ///
    /// [`Error::Format`] when the hash is not there or the entry is one of the AES-encrypted ones,
    /// [`Error::Io`] when the `.bdt` cannot be read.
    pub fn entry(&self, hash: u32) -> Result<Vec<u8>> {
        let found = find(&self.header, hash)?;
        if found.encrypted {
            return Err(Error::Format("that archive entry is AES-encrypted"));
        }
        let mut bdt = File::open(&self.bdt).map_err(Error::Io)?;
        bdt.seek(SeekFrom::Start(found.offset)).map_err(Error::Io)?;
        let mut data = vec![0u8; found.size];
        bdt.read_exact(&mut data).map_err(Error::Io)?;
        Ok(data)
    }
}

/// The archive's hash of a path: `h = h * 37 + byte` over the lowercased path, wrapping at 32 bits
/// (`path_hash` in `scripts/ds2-ebl.py`).
#[must_use]
pub fn path_hash(path: &str) -> u32 {
    path.bytes().fold(0u32, |hash, byte| {
        hash.wrapping_mul(37)
            .wrapping_add(u32::from(byte.to_ascii_lowercase()))
    })
}

/// The raw bytes of the `GameDataEbl` entry whose path hash is `hash`, from the game directory.
///
/// # Errors
///
/// As [`Archive::open`] and [`Archive::entry`].
pub fn entry(game: &Path, hash: u32) -> Result<Vec<u8>> {
    Archive::open(game)?.entry(hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn game() -> Option<PathBuf> {
        let home = std::env::var_os("HOME")?;
        let dir = PathBuf::from(home).join(
            ".local/share/Steam/steamapps/common/Dark Souls II Scholar of the First Sin/Game",
        );
        dir.join("GameDataEbl.bhd").is_file().then_some(dir)
    }

    /// The hash `scripts/ds2-ebl.py extract /menu/tex/icon/ic_0001220000.tpf` printed, whatever the
    /// case of the path.
    #[test]
    fn a_path_hashes_as_the_script_hashes_it() {
        assert_eq!(path_hash("/menu/tex/icon/ic_0001220000.tpf"), 0x0cc1_5707);
        assert_eq!(path_hash("/menu/tex/Icon/IC_0001220000.tpf"), 0x0cc1_5707);
        assert_eq!(path_hash(""), 0);
    }

    /// The Montgomery path against plain arithmetic, on a modulus small enough to check by hand.
    #[test]
    fn montgomery_pow_matches_u128_arithmetic() {
        let n: u128 = 0xffff_fffb;
        let mut big_n = [0u32; LIMBS];
        big_n[0] = n as u32;
        let rsa = Montgomery::new(big_n);
        for (base, exp) in [(2u128, 65537u32), (123_456, 3), (0xffff_fffa, 17), (5, 1)] {
            let mut expected = 1u128;
            for _ in 0..exp {
                expected = expected * base % n;
            }
            let mut b = [0u32; LIMBS];
            b[0] = base as u32;
            let got = rsa.pow(&b, &exp.to_be_bytes());
            assert_eq!(u128::from(got[0]), expected, "{base}^{exp}");
            assert!(got[1..].iter().all(|&l| l == 0));
        }
    }

    #[test]
    fn the_header_decrypts_and_the_waku_03_container_is_readable() {
        let Some(game) = game() else {
            eprintln!("no DARK SOULS II install here; skipped");
            return;
        };
        let bhd = std::fs::read(game.join("GameDataEbl.bhd")).unwrap();
        let pem = std::fs::read_to_string(game.join("GameDataKeyCode.pem")).unwrap();
        let header = decrypt_bhd(&bhd, &pem).expect("decrypts");
        assert_eq!(&header[..4], b"BHD5");
        // `waku_03.tpf`'s container, as `scripts/ds2-tpf.py` indexed it.
        let found = find(&header, 0xb1fa_153f).expect("present");
        assert!(!found.encrypted);
        let data = entry(&game, 0xb1fa_153f).unwrap();
        assert!(
            data.starts_with(b"DCX\0") || data.starts_with(b"BND4"),
            "{:?}",
            &data[..4]
        );
    }
}
