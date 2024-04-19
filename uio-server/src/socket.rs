use std::ffi::CString;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::PathBuf;
use std::os::unix::ffi::OsStrExt;

use anyhow::Context;

use crate::fs_utils::UnlinkOnDrop;

pub struct SeqPacketSocket {
    fd: OwnedFd,
    _path: UnlinkOnDrop,
}

impl SeqPacketSocket {
    pub fn open(path: PathBuf) -> anyhow::Result<Self> {
        // Convert the path from a Rust representation to something more compatible with libc.
        // ... I really think Rust should make basic FFI tasks like this easier than it currently is.
        let path_cstr = CString::new(path.as_os_str().as_bytes()).expect("UIO socket path contains null bytes.");
        let mut path_carray: [i8; 108] = [0; 108];
        if path_cstr.as_bytes_with_nul().len() > path_carray.len() {
            bail!("Socket path too long.");
        }
        unsafe { libc::strncpy(&mut path_carray as *mut i8, path_cstr.as_ptr(), path_carray.len() - 1) };

        // Create a socket FD.
        let socket = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_SEQPACKET, 0) };
        if socket < 0 {
            return Err(anyhow::Error::from(std::io::Error::last_os_error()))
                .context("Failed to create a socket fd!");
        }
        let socket = unsafe { OwnedFd::from_raw_fd(socket) };

        // Bind the socket to the filesystem.
        let socket_name = libc::sockaddr_un {
            sun_path: path_carray, sun_family: libc::AF_UNIX as _
        };
        let res = unsafe {
            libc::bind(
                socket.as_raw_fd(),
                &socket_name as *const libc::sockaddr_un as *const libc::sockaddr,
                socket_name.sun_path.len().try_into().unwrap()
            )
        };
        if res < 0 {
            return Err(anyhow::Error::from(std::io::Error::last_os_error()))
                .context("Failed to bind the socket.");
        }

        // Start listening to incoming connections.
        let backlog_size = 32;
        let res = unsafe { libc::listen(socket.as_raw_fd(), backlog_size) };
        if res < 0 {
            return Err(anyhow::Error::from(std::io::Error::last_os_error()))
                .context("Failed to listen to the socket.");
        }

        Ok(SeqPacketSocket {
            fd: socket, _path: UnlinkOnDrop::new(path)
        })
    }
}
