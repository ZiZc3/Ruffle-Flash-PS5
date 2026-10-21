//! Flash saves (SharedObjects, the ".sol" files) on disk, one folder per game:
//! /data/ruffle/saves/<game>/. Ruffle's default keeps them in memory only.
//!
//! Many games never call SharedObject.flush() and count on the player writing
//! their saves when it closes. The game loop flushes every few seconds too, so
//! a console shut down or put to rest mid-game keeps the progress. A save is
//! written only when it changed, on the background writer, so the game never
//! waits for the disk.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use ruffle_core::backend::storage::StorageBackend;

use crate::fsutil;

pub const SAVES_DIR: &str = "/data/ruffle/saves";

pub struct DiskStorage {
    dir: PathBuf,
    /// Each save as last read or written (newer than the disk while the
    /// background writer catches up).
    known: HashMap<String, Vec<u8>>,
}

impl DiskStorage {
    pub fn new(game_key: &str) -> Self {
        let dir = PathBuf::from(SAVES_DIR).join(game_key);
        if let Err(e) = fs::create_dir_all(&dir) {
            println!("[Saves] can't create {}: {}", dir.display(), e);
        }
        DiskStorage { dir, known: HashMap::new() }
    }

    /// Ruffle names a save like "localhost/path/to/movie.swf/name"; one file each.
    fn file(&self, name: &str) -> PathBuf {
        let safe: String = name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '_' })
            .collect();
        self.dir.join(format!("{}.sol", safe))
    }
}

impl StorageBackend for DiskStorage {
    fn get(&self, name: &str) -> Option<Vec<u8>> {
        match self.known.get(name) {
            Some(v) => Some(v.clone()),
            None => fs::read(self.file(name)).ok(),
        }
    }

    fn put(&mut self, name: &str, value: &[u8]) -> bool {
        let path = self.file(name);
        let same = match self.known.get(name) {
            Some(v) => v.as_slice() == value,
            None => fs::read(&path).is_ok_and(|v| v == value),
        };
        if !same {
            fsutil::write_in_background(path, value.to_vec());
        }
        self.known.insert(name.to_string(), value.to_vec());
        true
    }

    fn remove_key(&mut self, name: &str) {
        self.known.remove(name);
        fsutil::remove_in_background(self.file(name));
    }
}
