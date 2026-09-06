//! Persistent key/value blobs (settings, achievements) with one API for every
//! platform.  Native builds keep writing real files so existing saves stay
//! where they are; the browser build has no filesystem, so the same calls go
//! to `localStorage` keyed by the path's file name.
//!
//! The signatures deliberately mirror `std::fs` (including the `NotFound`
//! error kind for a missing entry) so callers keep their "no file yet vs.
//! corrupted file" branching unchanged.

use std::io;
use std::path::Path;

#[cfg(not(target_arch = "wasm32"))]
pub fn read_to_string(path: &Path) -> io::Result<String> {
    std::fs::read_to_string(path)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn write(path: &Path, data: &str) -> io::Result<()> {
    std::fs::write(path, data)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn create_dir_all(path: &Path) -> io::Result<()> {
    std::fs::create_dir_all(path)
}

// ── Browser: localStorage ──────────────────────────────────────────────────
//
// Keys are namespaced (`zombiegame2:<file name>`) so the game can't collide
// with anything else served from the same origin during development.

#[cfg(target_arch = "wasm32")]
fn local_storage() -> io::Result<web_sys::Storage> {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "localStorage unavailable"))
}

#[cfg(target_arch = "wasm32")]
fn key_for(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    format!("zombiegame2:{name}")
}

#[cfg(target_arch = "wasm32")]
pub fn read_to_string(path: &Path) -> io::Result<String> {
    let storage = local_storage()?;
    match storage.get_item(&key_for(path)) {
        Ok(Some(s)) => Ok(s),
        Ok(None) => Err(io::Error::new(io::ErrorKind::NotFound, "no such key")),
        Err(_) => Err(io::Error::new(io::ErrorKind::Other, "localStorage read failed")),
    }
}

#[cfg(target_arch = "wasm32")]
pub fn write(path: &Path, data: &str) -> io::Result<()> {
    let storage = local_storage()?;
    storage
        .set_item(&key_for(path), data)
        .map_err(|_| io::Error::new(io::ErrorKind::Other, "localStorage write failed"))
}

#[cfg(target_arch = "wasm32")]
pub fn create_dir_all(_path: &Path) -> io::Result<()> {
    Ok(())
}
