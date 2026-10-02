//! Flash saves (SharedObjects, the ".sol" files) on disk, one folder per game:
//! /data/ruffle/saves/<game>/. Ruffle's default keeps them in memory only.

use std::fs;
use std::path::PathBuf;

use ruffle_core::backend::storage::StorageBackend;

pub const SAVES_DIR: &str = "/data/ruffle/saves";

pub struct DiskStorage {
    dir: PathBuf,
}

impl DiskStorage {
    pub fn new(game_key: &str) -> Self {
        let dir = PathBuf::from(SAVES_DIR).join(game_key);
        if let Err(e) = fs::create_dir_all(&dir) {
            println!("[Saves] can't create {}: {}", dir.display(), e);
        }
        DiskStorage { dir }
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
        fs::read(self.file(name)).ok()
    }

    fn put(&mut self, name: &str, value: &[u8]) -> bool {
        let path = self.file(name);
        let tmp = path.with_extension("sol.tmp");
        let ok = fs::write(&tmp, value).and_then(|_| fs::rename(&tmp, &path));
        match ok {
            Ok(()) => {
                println!("[Saves] wrote {} ({} bytes)", path.display(), value.len());
                true
            }
            Err(e) => {
                println!("[Saves] can't write {}: {}", path.display(), e);
                false
            }
        }
    }

    fn remove_key(&mut self, name: &str) {
        let _ = fs::remove_file(self.file(name));
    }
}
