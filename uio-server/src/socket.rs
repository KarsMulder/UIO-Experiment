use std::ffi::CString;
use std::mem::MaybeUninit;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::path::PathBuf;
use std::os::unix::ffi::OsStrExt;

use anyhow::Context;

use crate::fs_utils::UnlinkOnDrop;

struct Packet {
    data: Vec<u8>,
}

impl Packet {
    pub fn empty() -> Self {
        Packet {
            data: Vec::new(),
        }
    }
}

pub struct SeqPacketSocket {
    fd: OwnedFd,
    _path: UnlinkOnDrop,
}

pub struct SeqPacketChannel {
    fd: OwnedFd,
    /// A partial packet containing data that has been read from the socket without having received end-of-message.
    read_buffer: Packet,
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

        // Give the file descriptor the proper flags.
        crate::fs_utils::set_cloexec(socket.as_fd());
        unsafe { libc::fcntl(socket.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK); }

        // Bind the socket to the filesystem.
        let socket_name = libc::sockaddr_un {
            sun_path: path_carray, sun_family: libc::AF_UNIX.try_into().unwrap()
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

    /// Receives a new incoming connection from a program.
    pub fn accept(&self) -> Result<SeqPacketChannel, std::io::Error> {
        let fd = unsafe {
            libc::accept4(
                self.fd.as_raw_fd(), std::ptr::null_mut(), std::ptr::null_mut(), libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC
            )
        };

        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };

        Ok(SeqPacketChannel { fd, read_buffer: Packet::empty() })
    }
}

impl SeqPacketChannel {
    pub fn read_packet(&mut self) -> Result<Packet, std::io::Error> {
        const MSG_BUF_SIZE: usize = 4096;
        const MSG_CONTROL_BUF_SIZE: usize = 1024;

        let mut msg_buf: MaybeUninit<[u8; MSG_BUF_SIZE]> = MaybeUninit::uninit();
        let mut msg_control: MaybeUninit<[u8; MSG_CONTROL_BUF_SIZE]> = MaybeUninit::uninit();

        let mut msg_iov: libc::iovec = libc::iovec {
            iov_base: msg_buf.as_mut_ptr() as *mut libc::c_void,
            iov_len: std::mem::size_of_val(&msg_buf),
        };

        let mut msghdr = libc::msghdr {
            msg_name: std::ptr::null_mut(),
            msg_namelen: 0,
            msg_iov: &mut msg_iov as *mut libc::iovec,
            msg_iovlen: std::mem::size_of_val(&msg_iov),
            msg_control: &mut msg_control as *mut _ as *mut libc::c_void,
            msg_controllen: std::mem::size_of_val(&msg_control),
            msg_flags: 0
        };

        let res = unsafe {
            libc::recvmsg(self.fd.as_raw_fd(), &mut msghdr, libc::MSG_CMSG_CLOEXEC)
        };
        if res < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if res == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::ConnectionReset, "The peer has performed an orderly shutdown."));
        }

        // TODO: In production code, this obviously shouln't panic.
        if msghdr.msg_flags & libc::MSG_TRUNC > 0 {
            panic!("Part of a message was truncated!");
        }
        if msghdr.msg_flags & libc::MSG_CTRUNC > 0 {
            panic!("Part of control data was discarded!");
        }
        if msghdr.msg_flags & libc::MSG_ERRQUEUE > 0 {
            panic!("Received error message through socket!");
        }

        // Check there is an end-of-packed delimination at the end of this message.
        if msghdr.msg_flags & libc::MSG_EOR > 0 {

        }

        unimplemented!()

    }
}
