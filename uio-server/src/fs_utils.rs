use std::path::{Path, PathBuf};

/// A to a file that will be deleted when this structure is dropped.
pub struct UnlinkOnDrop {
    path: PathBuf,
}

impl UnlinkOnDrop {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
    
    pub fn path(&self) -> &Path {
        self.path.as_path()
    }
}

impl Drop for UnlinkOnDrop {
    fn drop(&mut self) {
        if let Err(err) = std::fs::remove_file(&self.path) {
            eprintln!("Warning: failed to unlink the file {}: {err}", self.path.display());
        }
    }
}
