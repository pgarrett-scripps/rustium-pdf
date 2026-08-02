//! The standard security handler: RC4 and AES, revisions 2 through 6.
//!
//! Everything here serves `Decryptor`, which the file parser consults while loading objects.
//! Only password-based encryption is implemented; certificate-based files are refused.

use aes::cipher::{block_padding::NoPadding, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use md5::{Digest, Md5};
use sha2::{Sha256, Sha384, Sha512};

use crate::error::{Error, Result};
use crate::object::{Dict, Object};

const PAD: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08,
    0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

#[derive(Clone, Copy, PartialEq)]
enum Cipher {
    Rc4,
    Aes,
    Identity,
}

pub struct Decryptor {
    key: Vec<u8>,
    string_cipher: Cipher,
    stream_cipher: Cipher,
    /// V5 uses the file key directly; earlier versions derive a per-object key.
    per_object_key: bool,
    encrypt_metadata: bool,
}

impl Decryptor {
    pub fn new(
        enc: &Dict,
        file_id: &[u8],
        password: Option<&str>,
        resolve: &dyn Fn(&Object) -> Object,
    ) -> Result<Self> {
        let get = |k: &str| enc.get(k).map(resolve);
        let filter = get("Filter").and_then(|o| o.as_name().map(str::to_owned));
        if filter.as_deref() != Some("Standard") {
            return Err(Error::UnsupportedEncryption(format!(
                "security handler {:?}",
                filter.unwrap_or_default()
            )));
        }
        let v = get("V").and_then(|o| o.as_int()).unwrap_or(0);
        let r = get("R").and_then(|o| o.as_int()).unwrap_or(0);
        let length_bits = get("Length").and_then(|o| o.as_int()).unwrap_or(40);
        let o_entry = get("O")
            .and_then(|o| o.as_string().map(<[u8]>::to_vec))
            .unwrap_or_default();
        let u_entry = get("U")
            .and_then(|o| o.as_string().map(<[u8]>::to_vec))
            .unwrap_or_default();
        let p = get("P").and_then(|o| o.as_int()).unwrap_or(-1) as i32;
        let encrypt_metadata = get("EncryptMetadata")
            .and_then(|o| o.as_bool())
            .unwrap_or(true);
        let password = password.unwrap_or("");

        // Crypt-filter ciphers for V4/V5; earlier versions are RC4 throughout.
        let (string_cipher, stream_cipher) = if v >= 4 {
            let cf = get("CF")
                .and_then(|o| o.as_dict().cloned())
                .unwrap_or_default();
            let cipher_of = |name: Option<String>| -> Cipher {
                let Some(name) = name else {
                    return Cipher::Identity;
                };
                if name == "Identity" {
                    return Cipher::Identity;
                }
                let cfm = cf
                    .get(&name)
                    .map(resolve)
                    .and_then(|o| o.as_dict().and_then(|d| d.get("CFM").map(resolve)))
                    .and_then(|o| o.as_name().map(str::to_owned));
                match cfm.as_deref() {
                    Some("V2") => Cipher::Rc4,
                    Some("AESV2") | Some("AESV3") => Cipher::Aes,
                    _ => Cipher::Identity,
                }
            };
            let strf = get("StrF").and_then(|o| o.as_name().map(str::to_owned));
            let stmf = get("StmF").and_then(|o| o.as_name().map(str::to_owned));
            (cipher_of(strf), cipher_of(stmf))
        } else {
            (Cipher::Rc4, Cipher::Rc4)
        };

        if v == 5 || r >= 5 {
            let oe = get("OE")
                .and_then(|o| o.as_string().map(<[u8]>::to_vec))
                .unwrap_or_default();
            let ue = get("UE")
                .and_then(|o| o.as_string().map(<[u8]>::to_vec))
                .unwrap_or_default();
            let key = derive_key_v5(password, r, &o_entry, &u_entry, &oe, &ue)?;
            return Ok(Self {
                key,
                string_cipher,
                stream_cipher,
                per_object_key: false,
                encrypt_metadata,
            });
        }

        if !(1..=4).contains(&v) {
            return Err(Error::UnsupportedEncryption(format!("V {v}")));
        }
        let key_len = if r == 2 {
            5
        } else {
            (length_bits / 8).clamp(5, 16) as usize
        };

        // Try the password as the user password, then as the owner password.
        let user_key = legacy_file_key(
            password.as_bytes(),
            &o_entry,
            p,
            file_id,
            r,
            key_len,
            encrypt_metadata,
        );
        if legacy_check_user(&user_key, &u_entry, file_id, r) {
            return Ok(Self {
                key: user_key,
                string_cipher,
                stream_cipher,
                per_object_key: true,
                encrypt_metadata,
            });
        }
        let recovered = legacy_owner_to_user(password.as_bytes(), &o_entry, r, key_len);
        let owner_key = legacy_file_key(
            &recovered,
            &o_entry,
            p,
            file_id,
            r,
            key_len,
            encrypt_metadata,
        );
        if legacy_check_user(&owner_key, &u_entry, file_id, r) {
            return Ok(Self {
                key: owner_key,
                string_cipher,
                stream_cipher,
                per_object_key: true,
                encrypt_metadata,
            });
        }
        Err(Error::Password)
    }

    fn object_key(&self, num: u32, gen: u16, aes: bool) -> Vec<u8> {
        if !self.per_object_key {
            return self.key.clone();
        }
        let mut h = Md5::new();
        h.update(&self.key);
        h.update(&num.to_le_bytes()[..3]);
        h.update(&gen.to_le_bytes()[..2]);
        if aes {
            h.update(b"sAlT");
        }
        let digest = h.finalize();
        let n = (self.key.len() + 5).min(16);
        digest[..n].to_vec()
    }

    pub fn decrypt_string(&self, num: u32, gen: u16, data: &[u8]) -> Vec<u8> {
        self.decrypt(self.string_cipher, num, gen, data)
    }

    pub fn decrypt_stream(&self, num: u32, gen: u16, data: &[u8]) -> Vec<u8> {
        self.decrypt(self.stream_cipher, num, gen, data)
    }

    /// Whether a stream of the given `/Type` is encrypted at all.
    pub fn stream_needs_decrypt(&self, stream_type: Option<&str>) -> bool {
        // A document may declare that its metadata stays in the clear; every other stream is
        // encrypted. Naming the exemption keeps the negation readable.
        let metadata_exempt = stream_type == Some("Metadata") && !self.encrypt_metadata;
        !metadata_exempt
    }

    fn decrypt(&self, cipher: Cipher, num: u32, gen: u16, data: &[u8]) -> Vec<u8> {
        match cipher {
            Cipher::Identity => data.to_vec(),
            Cipher::Rc4 => rc4(&self.object_key(num, gen, false), data),
            Cipher::Aes => aes_cbc_decrypt(&self.object_key(num, gen, true), data),
        }
    }
}

// ---- legacy (R2-R4) key derivation ---------------------------------------------------------

fn padded(pw: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let n = pw.len().min(32);
    out[..n].copy_from_slice(&pw[..n]);
    out[n..].copy_from_slice(&PAD[..32 - n]);
    out
}

fn legacy_file_key(
    password: &[u8],
    o_entry: &[u8],
    p: i32,
    file_id: &[u8],
    r: i64,
    key_len: usize,
    encrypt_metadata: bool,
) -> Vec<u8> {
    let mut h = Md5::new();
    h.update(padded(password));
    h.update(&o_entry[..o_entry.len().min(32)]);
    h.update(p.to_le_bytes());
    h.update(file_id);
    if r >= 4 && !encrypt_metadata {
        h.update([0xFF, 0xFF, 0xFF, 0xFF]);
    }
    let mut key = h.finalize()[..].to_vec();
    if r >= 3 {
        for _ in 0..50 {
            key = Md5::digest(&key[..key_len])[..].to_vec();
        }
    }
    key.truncate(key_len);
    key
}

fn legacy_check_user(key: &[u8], u_entry: &[u8], file_id: &[u8], r: i64) -> bool {
    if u_entry.len() < 16 {
        return false;
    }
    if r == 2 {
        return rc4(key, &PAD) == u_entry[..32.min(u_entry.len())];
    }
    let mut h = Md5::new();
    h.update(PAD);
    h.update(file_id);
    let mut val = rc4(key, &h.finalize());
    for i in 1..=19u8 {
        let k: Vec<u8> = key.iter().map(|&b| b ^ i).collect();
        val = rc4(&k, &val);
    }
    val[..16] == u_entry[..16]
}

/// Algorithm 7 in reverse: recovers the (padded) user password from `O` given the owner
/// password. Returns garbage for a wrong password, which the user check then rejects.
fn legacy_owner_to_user(owner_pw: &[u8], o_entry: &[u8], r: i64, key_len: usize) -> Vec<u8> {
    let mut key = Md5::digest(padded(owner_pw))[..].to_vec();
    if r >= 3 {
        for _ in 0..50 {
            key = Md5::digest(&key)[..].to_vec();
        }
    }
    key.truncate(key_len);
    let mut val = o_entry[..o_entry.len().min(32)].to_vec();
    if r == 2 {
        val = rc4(&key, &val);
    } else {
        for i in (0..=19u8).rev() {
            let k: Vec<u8> = key.iter().map(|&b| b ^ i).collect();
            val = rc4(&k, &val);
        }
    }
    val
}

// ---- V5 / AES-256 (R5, R6) -----------------------------------------------------------------

fn derive_key_v5(
    password: &str,
    r: i64,
    o_entry: &[u8],
    u_entry: &[u8],
    oe: &[u8],
    ue: &[u8],
) -> Result<Vec<u8>> {
    // SASLprep is required by spec for passwords; ASCII passwords (the overwhelming case) are
    // unaffected. Non-ASCII passwords are passed through as UTF-8.
    let pw = {
        let b = password.as_bytes();
        &b[..b.len().min(127)]
    };
    if o_entry.len() < 48 || u_entry.len() < 48 {
        return Err(Error::UnsupportedEncryption("short O/U entries".into()));
    }

    let hash = |pw: &[u8], salt: &[u8], udata: &[u8]| -> [u8; 32] {
        if r == 5 {
            let mut h = Sha256::new();
            h.update(pw);
            h.update(salt);
            h.update(udata);
            h.finalize().into()
        } else {
            hash_2b(pw, salt, udata)
        }
    };

    // Try owner password first: its hash covers the whole U entry.
    let (o_hash, o_vsalt, o_ksalt) = (&o_entry[..32], &o_entry[32..40], &o_entry[40..48]);
    let (u_hash, u_vsalt, u_ksalt) = (&u_entry[..32], &u_entry[32..40], &u_entry[40..48]);

    if hash(pw, o_vsalt, &u_entry[..48]) == *o_hash {
        let ikey = hash(pw, o_ksalt, &u_entry[..48]);
        let key = aes256_cbc_no_pad_decrypt(&ikey, &[0u8; 16], oe)?;
        return Ok(key);
    }
    if hash(pw, u_vsalt, &[]) == *u_hash {
        let ikey = hash(pw, u_ksalt, &[]);
        let key = aes256_cbc_no_pad_decrypt(&ikey, &[0u8; 16], ue)?;
        return Ok(key);
    }
    Err(Error::Password)
}

/// ISO 32000-2 algorithm 2.B: the hardened iterated hash used by revision 6.
fn hash_2b(pw: &[u8], salt: &[u8], udata: &[u8]) -> [u8; 32] {
    let mut k: Vec<u8> = {
        let mut h = Sha256::new();
        h.update(pw);
        h.update(salt);
        h.update(udata);
        h.finalize().to_vec()
    };
    let mut round = 0usize;
    loop {
        // K1 = (pw || K || udata) repeated 64 times.
        let mut unit = Vec::with_capacity(pw.len() + k.len() + udata.len());
        unit.extend_from_slice(pw);
        unit.extend_from_slice(&k);
        unit.extend_from_slice(udata);
        let mut k1 = Vec::with_capacity(unit.len() * 64);
        for _ in 0..64 {
            k1.extend_from_slice(&unit);
        }
        // E = AES-128-CBC(key=K[0..16], iv=K[16..32], K1)
        let enc = Aes128CbcEnc::new_from_slices(&k[..16], &k[16..32]).expect("fixed sizes");
        let e = enc.encrypt_padded_vec_mut::<NoPadding>(&k1);
        let modulus = e[..16].iter().map(|&b| b as u32).sum::<u32>() % 3;
        k = match modulus {
            0 => Sha256::digest(&e).to_vec(),
            1 => Sha384::digest(&e).to_vec(),
            _ => Sha512::digest(&e).to_vec(),
        };
        round += 1;
        if round >= 64 && (*e.last().unwrap() as usize) <= round - 32 {
            break;
        }
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&k[..32]);
    out
}

// ---- primitives ----------------------------------------------------------------------------

type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;
type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;
type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;

pub fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut s: [u8; 256] = std::array::from_fn(|i| i as u8);
    let mut j = 0u8;
    for i in 0..256 {
        j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len().max(1)]);
        s.swap(i, j as usize);
    }
    let (mut i, mut j) = (0u8, 0u8);
    data.iter()
        .map(|&b| {
            i = i.wrapping_add(1);
            j = j.wrapping_add(s[i as usize]);
            s.swap(i as usize, j as usize);
            b ^ s[(s[i as usize].wrapping_add(s[j as usize])) as usize]
        })
        .collect()
}

/// AES-CBC with the IV in the first block, PKCS#5 padding stripped leniently.
fn aes_cbc_decrypt(key: &[u8], data: &[u8]) -> Vec<u8> {
    if data.len() < 32 || !(data.len() - 16).is_multiple_of(16) {
        return Vec::new();
    }
    let (iv, body) = data.split_at(16);
    let mut buf = body.to_vec();
    let ok = match key.len() {
        16 => Aes128CbcDec::new_from_slices(key, iv)
            .map(|c| c.decrypt_padded_mut::<NoPadding>(&mut buf).map(|_| ()))
            .is_ok_and(|r| r.is_ok()),
        32 => Aes256CbcDec::new_from_slices(key, iv)
            .map(|c| c.decrypt_padded_mut::<NoPadding>(&mut buf).map(|_| ()))
            .is_ok_and(|r| r.is_ok()),
        _ => false,
    };
    if !ok {
        return Vec::new();
    }
    // Strip padding when it is well-formed; tolerate producers that omit it.
    if let Some(&pad) = buf.last() {
        if (1..=16).contains(&pad) && buf.len() >= pad as usize {
            buf.truncate(buf.len() - pad as usize);
        }
    }
    buf
}

fn aes256_cbc_no_pad_decrypt(key: &[u8; 32], iv: &[u8; 16], data: &[u8]) -> Result<Vec<u8>> {
    if !data.len().is_multiple_of(16) || data.is_empty() {
        return Err(Error::UnsupportedEncryption("bad OE/UE length".into()));
    }
    let mut buf = data.to_vec();
    Aes256CbcDec::new_from_slices(key, iv)
        .map_err(|_| Error::UnsupportedEncryption("aes-256 init".into()))?
        .decrypt_padded_mut::<NoPadding>(&mut buf)
        .map_err(|_| Error::UnsupportedEncryption("aes-256 decrypt".into()))?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rc4_test_vectors() {
        // RFC 6229-style known vector: key "Key", plaintext "Plaintext".
        let out = rc4(b"Key", b"Plaintext");
        assert_eq!(out, [0xBB, 0xF3, 0x16, 0xE8, 0xD9, 0x40, 0xAF, 0x0A, 0xD3]);
        // Round trip.
        assert_eq!(rc4(b"Key", &out), b"Plaintext");
    }

    #[test]
    fn padding_pads_and_truncates() {
        assert_eq!(padded(b"")[..], PAD[..]);
        let p = padded(b"secret");
        assert_eq!(&p[..6], b"secret");
        assert_eq!(&p[6..], &PAD[..26]);
    }
}
