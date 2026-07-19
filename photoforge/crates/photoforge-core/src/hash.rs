//! Content hashing with BLAKE3.
//!
//! Files are streamed in 1 MB chunks so hashing a 50 MB RAW-ish JPEG never
//! loads the whole file into memory, and the buffer is large enough that the
//! read syscall count stays low.

use crate::Result;
use std::fs::File;
use std::io::Read;
use std::path::Path;

/// Chunk size for streamed hashing (1 MB, per the project spec).
const CHUNK: usize = 1024 * 1024;

/// Compute the BLAKE3 hash of a file's contents as raw 32 bytes (what the
/// `hashes.blake3` BLOB column stores).
pub fn blake3_file(path: impl AsRef<Path>) -> Result<[u8; 32]> {
    let mut file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    // A heap buffer this size would blow the stack as an array; `vec!` puts it
    // on the heap (Rust arrays are stack-allocated by default, unlike Java's).
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(*hasher.finalize().as_bytes())
}

/// Hex-string convenience wrapper around [`blake3_file`] (CLI display).
pub fn hash_file(path: impl AsRef<Path>) -> Result<String> {
    let bytes = blake3_file(path)?;
    Ok(blake3::Hash::from_bytes(bytes).to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_known_vector() {
        let dir = std::env::temp_dir();
        let p = dir.join(format!("pf_hash_{}.bin", std::process::id()));
        std::fs::write(&p, b"hello").unwrap();
        // blake3 of "hello", independently computed.
        assert_eq!(
            hash_file(&p).unwrap(),
            "ea8f163db38682925e4491c5e58d4bb3506ef8c14eb78a86e908c5624a67200f"
        );
        let _ = std::fs::remove_file(&p);
    }
}
