//! Writing the app's files so a crash or power cut leaves the old version or
//! the new one, never an empty or half-written file.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{self, Sender};

/// Writes beside the file, syncs, renames over it and syncs the folder.
pub fn write_durably(path: impl AsRef<Path>, body: impl AsRef<[u8]>) -> std::io::Result<()> {
    let path = path.as_ref();
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let mut f = File::create(&tmp)?;
    f.write_all(body.as_ref())?;
    f.sync_all()?;
    drop(f);
    fs::rename(&tmp, path)?;
    // The file is in place by now; syncing the folder only makes it stick sooner.
    if let Some(dir) = path.parent() {
        let _ = File::open(dir).and_then(|d| d.sync_all());
    }
    Ok(())
}

/// The same, on a thread of its own, so a game never waits for the disk.
pub fn write_in_background(path: PathBuf, body: Vec<u8>) {
    send(path, Some(body));
}

/// Removes a file on that thread, after any write to it still waiting there.
pub fn remove_in_background(path: PathBuf) {
    send(path, None);
}

/// Jobs run in the order they were asked for.
fn send(path: PathBuf, body: Option<Vec<u8>>) {
    type Job = (PathBuf, Option<Vec<u8>>);
    static WRITER: Mutex<Option<Sender<Job>>> = Mutex::new(None);
    let mut writer = WRITER.lock().expect("writer lock");
    let tx = writer.get_or_insert_with(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::spawn(move || {
            for (path, body) in rx {
                let Some(body) = body else {
                    let _ = fs::remove_file(&path);
                    continue;
                };
                match write_durably(&path, &body) {
                    Ok(()) => println!("[Files] wrote {} ({} bytes)", path.display(), body.len()),
                    Err(e) => println!("[Files] can't write {}: {}", path.display(), e),
                }
            }
        });
        tx
    });
    let _ = tx.send((path, body));
}
