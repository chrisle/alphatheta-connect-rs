//! OneLibrary database encryption.
//!
//! The database is encrypted with SQLCipher 4. The encryption key is derived
//! from a hardcoded obfuscated blob.

use std::io::Read;

use crate::{Error, Result};

/// The obfuscated encryption key blob from pyrekordbox.
const BLOB: &[u8] = b"PN_1dH8$oLJY)16j_RvM6qphWw`476>;C1cWmI#se(PG`j}~xAjlufj?`#0i{;=glh(SkW)y0>n?YEiD`l%t(";

/// XOR key used for deobfuscation.
const BLOB_KEY: &[u8] = b"657f48f84c437cc1";

/// Base85 (RFC 1924) decode.
fn base85_decode(input: &[u8]) -> Result<Vec<u8>> {
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz!#$%&()*+-;<=>?@^_`{|}~";

    let mut result = Vec::with_capacity(input.len() * 4 / 5 + 4);

    for chunk in input.chunks(5) {
        let mut value: u64 = 0;
        for &c in chunk {
            let v = ALPHABET
                .iter()
                .position(|a| *a == c)
                .ok_or_else(|| Error::Database(format!("Invalid base85 character: {}", c as char)))?;
            value = value * 85 + v as u64;
        }

        // Upstream does the arithmetic in 32-bit JavaScript integers.
        let value = value as u32;
        let bytes = value.to_be_bytes();
        let num_bytes = if chunk.len() == 5 { 4 } else { chunk.len() - 1 };
        result.extend_from_slice(&bytes[..num_bytes]);
    }

    Ok(result)
}

/// Deobfuscate the blob to get the encryption key.
fn deobfuscate(blob: &[u8]) -> Result<String> {
    let decoded = base85_decode(blob)?;

    let xored: Vec<u8> = decoded.iter().enumerate().map(|(i, b)| b ^ BLOB_KEY[i % BLOB_KEY.len()]).collect();

    let mut decompressed = Vec::new();
    flate2::read::ZlibDecoder::new(&xored[..])
        .read_to_end(&mut decompressed)
        .map_err(|e| Error::Database(format!("could not inflate the encryption key: {e}")))?;

    Ok(String::from_utf8_lossy(&decompressed).into_owned())
}

/// Get the SQLCipher encryption key for OneLibrary databases.
pub fn get_encryption_key() -> Result<String> {
    let key = deobfuscate(BLOB)?;
    if !key.starts_with("r8gd") {
        return Err(Error::Database("Invalid encryption key derived".into()));
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_the_key() {
        let key = get_encryption_key().unwrap();
        assert!(key.starts_with("r8gd"));
        assert!(key.len() > 10);
    }
}
