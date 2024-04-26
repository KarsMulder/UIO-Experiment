use std::os::fd::{AsRawFd, BorrowedFd};
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

/// Sets the O_CLOEXEC flag on this file descriptor.
pub fn set_cloexec(fd: BorrowedFd) {
    unsafe {
        unsafe {
            libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::O_CLOEXEC);
        }
    }
}


