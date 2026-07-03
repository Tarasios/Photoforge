//! Content hashing with BLAKE3.

use crate::Result;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

/// Compute the BLAKE3 hash of a file's contents, returned as a hex string.
pub fn hash_file(path: impl AsRef<Path>) -> Result<String> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut hasher = blake3::Hasher::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}
