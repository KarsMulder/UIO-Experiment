use std::mem::MaybeUninit;
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::PathBuf;

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
        // Create a socket FD.
        let socket = rustix::net::socket(rustix::net::AddressFamily::UNIX, rustix::net::SocketType::SEQPACKET, None)?;

        // Give the file descriptor the proper flags.
        rustix::fs::fcntl_setfd(&socket, rustix::fs::FdFlags::CLOEXEC)?;
        rustix::fs::fcntl_setfl(&socket, rustix::fs::OFlags::NONBLOCK)?;

        // Bind the socket to the filesystem.
        let socket_name = rustix::net::SocketAddrUnix::new(path.clone())?;
        rustix::net::bind_unix(&socket, &socket_name)?;

        // Start listening to incoming connections.
        let backlog_size = 32;
        rustix::net::listen(&socket, backlog_size)?;

        Ok(SeqPacketSocket {
            fd: socket, _path: UnlinkOnDrop::new(path)
        })
    }

    /// Receives a new incoming connection from a program.
    pub fn accept(&self) -> Result<SeqPacketChannel, std::io::Error> {
        let fd = rustix::net::accept_with(self, rustix::net::SocketFlags::NONBLOCK | rustix::net::SocketFlags::CLOEXEC)?;
        Ok(SeqPacketChannel { fd, read_buffer: Packet::empty() })
    }
}

impl std::os::fd::AsFd for SeqPacketSocket {
    fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.fd.as_fd()
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

impl std::os::fd::AsFd for SeqPacketChannel {
    fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.fd.as_fd()
    }
}
