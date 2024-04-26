pub const DEFAULT_UIO_SOCKET_PATH: &str = "/run/uio/socket";

use std::io::{IoSlice, IoSliceMut};
use std::os::fd::{OwnedFd, AsFd, BorrowedFd};
use std::path::{Path, PathBuf};
use rustix::fs::OFlags;
use rustix::io::FdFlags;
use rustix::net::{RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, SendAncillaryBuffer, SendAncillaryMessage, SendFlags};

use crate::fs_utils::UnlinkOnDrop;
use crate::message::{EventMsg, RequestMsg};

/// A message that can be send through a SeqPacketChannel. It is a vector of bytes that optionally contains
/// space for file descriptors.
pub struct Packet {
    pub data: Vec<u8>,
    pub fds: Vec<OwnedFd>,
}

impl Packet {
    // TODO: This leaks implementation details. The public API shouldn't expose bincode::Error.
    // Also, I should consider using TryInto and TryFrom.
    pub fn try_into_event(self) -> Result<(EventMsg, Vec<OwnedFd>), bincode::Error> {
        let msg = bincode::deserialize(&self.data)?;
        Ok((msg, self.fds))
    }
    pub fn try_from_event(event: EventMsg, fds: Vec<OwnedFd>) -> Result<Packet, bincode::Error> {
        let data = bincode::serialize(&event)?;
        Ok(Packet { data, fds })
    }

    pub fn try_into_request(self) -> Result<(RequestMsg, Vec<OwnedFd>), bincode::Error> {
        let msg = bincode::deserialize(&self.data)?;
        Ok((msg, self.fds))
    }
    pub fn try_from_request(request: RequestMsg, fds: Vec<OwnedFd>) -> Result<Packet, bincode::Error> {
        let data = bincode::serialize(&request)?;
        Ok(Packet { data, fds })
    }
}

pub struct Message<T> {
    pub msg: T,
    pub fds: Vec<OwnedFd>,
}

impl Packet {
    pub fn empty() -> Self {
        Packet {
            data: Vec::new(),
            fds: Vec::new(),
        }
    }
}

pub struct SeqPacketChannel {
    fd: OwnedFd,
    /// A partial packet containing data that has been read from the socket without having received end-of-message.
    read_buffer: Packet,
}

pub struct SeqPacketSocket {
    fd: OwnedFd,
    _path: UnlinkOnDrop,
}

impl SeqPacketSocket {
    /// Creates a new socket that accepts incoming connections. Used by the server.
    pub fn open(path: PathBuf) -> Result<SeqPacketSocket, std::io::Error> {
        // Create a socket FD.
        let socket = rustix::net::socket(rustix::net::AddressFamily::UNIX, rustix::net::SocketType::SEQPACKET, None)?;

        // Give the file descriptor the proper flags.
        rustix::fs::fcntl_setfd(&socket, FdFlags::CLOEXEC)?;
        rustix::fs::fcntl_setfl(&socket, OFlags::NONBLOCK)?;

        // Bind the socket to the filesystem.
        let socket_name = rustix::net::SocketAddrUnix::new(&path)?;
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
    /// Connects to an already existing socket. Used by the client.
    pub fn open(path: &Path) -> Result<Self, std::io::Error> {
        // Create a socket FD.
        let socket = rustix::net::socket(rustix::net::AddressFamily::UNIX, rustix::net::SocketType::SEQPACKET, None)?;

        // Give the file descriptor the proper flags.
        rustix::fs::fcntl_setfd(&socket, FdFlags::CLOEXEC)?;
        rustix::fs::fcntl_setfl(&socket, OFlags::NONBLOCK)?;
        
        // Open the socket from the filesystem.
        let socket_name = rustix::net::SocketAddrUnix::new(path)?;
        rustix::net::connect_unix(&socket, &socket_name)?;

        Ok(SeqPacketChannel {
            fd: socket, read_buffer: Packet::empty()
        })
    }

    pub fn read_packet(&mut self) -> Result<Packet, std::io::Error> {
        const MSG_BUF_SIZE: usize = 16 * 1024;

        loop {
            // ... I'm not a fan of how rustix requires us to zero-init the whole buffer, but then again, I have
            // better things to do right now than micro-optimizations.
            let mut msg_buf: [u8; MSG_BUF_SIZE] = [0; MSG_BUF_SIZE];
            let mut ioslice = [IoSliceMut::new(&mut msg_buf)];

            let mut control_space = [0; rustix::cmsg_space!(ScmRights(64))];
            let mut control_buf = RecvAncillaryBuffer::new(&mut control_space);

            let res = rustix::net::recvmsg(&self, &mut ioslice, &mut control_buf, RecvFlags::CMSG_CLOEXEC)?;

            // TODO: This can cause out-of-memory when dealing with a malicious client.
            let message = &msg_buf[0 .. res.bytes];
            self.read_buffer.data.extend_from_slice(message);

            // TODO: In production code, all of the following instances of panic! are obviously unacceptable.
            if res.bytes == 0 {
                panic!("Received zero bytes from socket. Investigate why this happens.");
            }
            if res.flags.contains(RecvFlags::TRUNC) {
                panic!("Part of a message was truncated!");
            }
            if res.flags.contains(RecvFlags::ERRQUEUE) {
                panic!("Received error message through socket!");
            }
            // For some reason rustix doesn't contain this flag. Maybe I should send a pull request?
            if res.flags.bits() & (libc::MSG_CTRUNC as u32) > 0 {
                panic!("Part of control data was discarded!");
            }

            for control_msg in control_buf.drain() {
                match control_msg {
                    RecvAncillaryMessage::ScmRights(fds) => self.read_buffer.fds.extend(fds),
                    RecvAncillaryMessage::ScmCredentials(_) => panic!("Received credentials!"),
                    _ => panic!("Received unknown ancillary data!"),
                }
            }

            // Check there is an end-of-packed delimination at the end of this message.
            if res.flags.bits() & (libc::MSG_EOR as u32) > 0 {
                let empty_buffer = Packet::empty();
                return Ok(std::mem::replace(&mut self.read_buffer, empty_buffer));
            } else {
                // Keep trying again until we either get a complete message or fail by EWOULDBLOCK or EAGAIN.
                println!("Message exceeded buffer size, reading again...");
            }
        }
    }

    pub fn write_packet(&mut self, packet: Packet) -> Result<(), std::io::Error> {
        let slice = [IoSlice::new(&packet.data)];

        let mut control_space = [0; rustix::cmsg_space!(ScmRights(64))];
        let mut control_buf = SendAncillaryBuffer::new(&mut control_space);
        let rights: Vec<BorrowedFd> = packet.fds.iter().map(|fd| fd.as_fd()).collect();
        let res = control_buf.push(SendAncillaryMessage::ScmRights(&rights));
        if !res {
            panic!("Failed to send file descriptors.")
        }

        let num_sent_bytes = rustix::net::sendmsg(
            &self,
            &slice,
            &mut control_buf,
            // TODO: This seems misspelled? It compiles to MSG_EOR by inspecting the source code. I should raise an issue.
            SendFlags::EOT
        )?;

        if num_sent_bytes != packet.data.len() {
            panic!("Failed to transmit a packet within a single syscall!");
        }

        Ok(())
    }
}

impl std::os::fd::AsFd for SeqPacketChannel {
    fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.fd.as_fd()
    }
}
