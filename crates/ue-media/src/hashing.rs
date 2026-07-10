//! Fast content hash: xxh3 of the first and last 4 MB + size.
//! Enough for cache keys and relink without reading huge files.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use xxhash_rust::xxh3::Xxh3;

use crate::MediaResult;

const CHUNK: u64 = 4 * 1024 * 1024;

pub fn content_hash(path: &Path) -> MediaResult<String> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let mut hasher = Xxh3::new();
    hasher.update(&size.to_le_bytes());

    let mut buf = vec![0u8; CHUNK as usize];

    let head = file.read(&mut buf)?;
    hasher.update(&buf[..head]);

    if size > CHUNK * 2 {
        file.seek(SeekFrom::End(-(CHUNK as i64)))?;
        let tail = file.read(&mut buf)?;
        hasher.update(&buf[..tail]);
    }

    Ok(format!("xxh3:{:016x}", hasher.digest()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable_and_sensitive() {
        let dir = std::env::temp_dir();
        let a = dir.join("ue_hash_a.bin");
        let b = dir.join("ue_hash_b.bin");
        std::fs::write(&a, b"hello world hello world").unwrap();
        std::fs::write(&b, b"hello world hello worlX").unwrap();
        let ha1 = content_hash(&a).unwrap();
        let ha2 = content_hash(&a).unwrap();
        let hb = content_hash(&b).unwrap();
        assert_eq!(ha1, ha2);
        assert_ne!(ha1, hb);
        assert!(ha1.starts_with("xxh3:"));
    }
}
