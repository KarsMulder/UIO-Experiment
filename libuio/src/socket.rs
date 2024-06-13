/// TODO: Obviously, this socket needs to go elsewhere.
pub const DEFAULT_UIO_SOCKET_PATH: &str = "/tmp/uio/socket";

use std::io::IoSlice;
use std::os::fd::{OwnedFd, AsFd, BorrowedFd};
use std::path::{Path, PathBuf};
use rustix::fd::AsRawFd;
use rustix::fs::OFlags;
use rustix::io::FdFlags;
use rustix::net::{RecvAncillaryBuffer, RecvAncillaryMessage, SendAncillaryBuffer, SendAncillaryMessage, SendFlags};

use crate::fs_utils::UnlinkOnDrop;
use crate::message::{EventMsg, RequestMsg};

/// A message that can be send through a StreamChannel. It is a vector of bytes that optionally contains
/// space for file descriptors.
pub struct Packet {
    /// The bytes without header that this packet contains.
    pub data: Vec<u8>,
    pub fds: Vec<OwnedFd>,
}

/// Holds the data read from a channel until it gets sorted into packets.
struct PartialPacket {
    /// Bytes read from this socket. Each packet has the following structure:
    /// u16 (low endian) containing the length of the packet, excluding the header.
    /// u16 (low endian) containing the amount of file descriptors sent with this packet
    /// arbitrary bytes equal to the length of the packet payload
    data: Vec<u8>,
    /// File descriptors read from the socket that have not been associated with a complete packet yet.
    fds: Vec<OwnedFd>,
}

const PACKET_HEADER_LEN: usize = 4;

impl PartialPacket {
    fn try_drain_packet(&mut self) -> Option<Packet> {
        if self.data.len() < PACKET_HEADER_LEN {
            return None;
        }

        let packet_length: usize = u16::from_le_bytes(self.data[0..2].try_into().unwrap()).into();
        if self.data.len() < packet_length {
            return None;
        }

        let num_fds: usize = u16::from_le_bytes(self.data[2..4].try_into().unwrap()).into();
        if self.fds.len() < num_fds {
            return None;
        }

        let packet_bytes = self.data[PACKET_HEADER_LEN .. PACKET_HEADER_LEN + packet_length].to_owned();
        let remaining_bytes = self.data[PACKET_HEADER_LEN + packet_length ..].to_owned();
        self.data = remaining_bytes;

        let remaining_fds = self.fds.split_off(num_fds);
        let packet_fds = std::mem::replace(&mut self.fds, remaining_fds);

        Some(Packet {
            data: packet_bytes, fds: packet_fds
        })
    }

    /// Returns all complete packets stored in this buffer. Can return zero, one, or multiple packets.
    fn drain_packets(&mut self) -> Vec<Packet> {
        let mut result = Vec::new();
        while let Some(packet) = self.try_drain_packet() {
            result.push(packet);
        }
        result
    }

    fn new() -> PartialPacket {
        PartialPacket {
            data: Vec::new(),
            fds: Vec::new(), 
        }
    }
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

pub struct StreamChannel {
    fd: OwnedFd,
    /// A partial packet containing data that has been read from the socket without having received end-of-message.
    read_buffer: PartialPacket,
}

pub struct StreamSocket {
    fd: OwnedFd,
    _path: UnlinkOnDrop,
}

impl StreamSocket {
    /// Creates a new socket that accepts incoming connections. Used by the server.
    pub fn open(path: PathBuf) -> Result<StreamSocket, std::io::Error> {
        // Create a socket FD.
        let socket = rustix::net::socket(rustix::net::AddressFamily::UNIX, rustix::net::SocketType::STREAM, None)?;

        // Give the file descriptor the proper flags.
        rustix::fs::fcntl_setfd(&socket, FdFlags::CLOEXEC)?;
        rustix::fs::fcntl_setfl(&socket, OFlags::NONBLOCK)?;

        // Bind the socket to the filesystem.
        let socket_name = rustix::net::SocketAddrUnix::new(&path)?;
        rustix::net::bind_unix(&socket, &socket_name)?;

        // Start listening to incoming connections.
        let backlog_size = 32;
        rustix::net::listen(&socket, backlog_size)?;

        Ok(StreamSocket {
            fd: socket, _path: UnlinkOnDrop::new(path)
        })
    }

    /// Receives a new incoming connection from a program.
    pub fn accept(&self) -> Result<StreamChannel, std::io::Error> {
        let fd = rustix::net::accept_with(self, rustix::net::SocketFlags::NONBLOCK | rustix::net::SocketFlags::CLOEXEC)?;
        Ok(StreamChannel { fd, read_buffer: PartialPacket::new() })
    }
}

impl std::os::fd::AsFd for StreamSocket {
    fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl StreamChannel {
    /// Connects to an already existing socket. Used by the client.
    pub fn open(path: &Path) -> Result<Self, std::io::Error> {
        // Create a socket FD.
        let socket = rustix::net::socket(rustix::net::AddressFamily::UNIX, rustix::net::SocketType::STREAM, None)?;

        // Give the file descriptor the proper flags.
        rustix::fs::fcntl_setfd(&socket, FdFlags::CLOEXEC)?;
        rustix::fs::fcntl_setfl(&socket, OFlags::NONBLOCK)?;
        
        // Open the socket from the filesystem.
        let socket_name = rustix::net::SocketAddrUnix::new(path)?;
        rustix::net::connect_unix(&socket, &socket_name)?;

        Ok(StreamChannel {
            fd: socket, read_buffer: PartialPacket::new()
        })
    }

    pub fn read_packets(&mut self) -> Result<Vec<Packet>, std::io::Error> {
        const MSG_BUF_SIZE: usize = 16 * 1024;

        // ... I'm not a fan of how rustix requires us to zero-init the whole buffer, but then again, I have
        // better things to do right now than micro-optimizations.
        let mut msg_buf: [u8; MSG_BUF_SIZE] = [0; MSG_BUF_SIZE];
        let mut control_space = [0; rustix::cmsg_space!(ScmRights(32))];

        let mut iovec = libc::iovec {
            iov_base: &mut msg_buf as *mut _ as *mut libc::c_void,
            iov_len: std::mem::size_of_val(&msg_buf),
        };

        let mut msghdr = libc::msghdr {
            msg_name: std::ptr::null_mut(),
            msg_namelen: 0,
            msg_iov: &mut iovec as *mut _,
            msg_iovlen: std::mem::size_of_val(&iovec),
            msg_control: &mut control_space as *mut _ as *mut libc::c_void,
            msg_controllen: std::mem::size_of_val(&control_space),
            msg_flags: 0,
        };

        let num_bytes = unsafe { libc::recvmsg(
            self.fd.as_raw_fd(),
            &mut msghdr,
            libc::MSG_CMSG_CLOEXEC
        )};

        if num_bytes < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let bytes = num_bytes as usize;

        let mut control_buf = RecvAncillaryBuffer::new(&mut control_space);
        let flags = msghdr.msg_flags;

        // TODO: This can cause out-of-memory when dealing with a malicious client.
        let message = &msg_buf[0 .. bytes];
        self.read_buffer.data.extend_from_slice(message);

        // TODO: In production code, all of the following instances of panic! are obviously unacceptable.
        if flags & libc::MSG_TRUNC > 0 {
            panic!("Part of a message was truncated!");
        }
        if flags & libc::MSG_ERRQUEUE > 0 {
            panic!("Received error message through socket!");
        }
        if flags & libc::MSG_CTRUNC > 0 {
            panic!("Part of control data was discarded!");
        }

        for control_msg in control_buf.drain() {
            match control_msg {
                RecvAncillaryMessage::ScmRights(fds) => self.read_buffer.fds.extend(fds),
                RecvAncillaryMessage::ScmCredentials(_) => panic!("Received credentials!"),
                _ => panic!("Received unknown ancillary data!"),
            }
        }

        println!("Received bytes: {}, received flags: {:x}", bytes, flags);
        
        return Ok(self.read_buffer.drain_packets())
    }

    pub fn write_packet(&mut self, packet: Packet) -> Result<(), std::io::Error> {
        // Add the header to the packet for transmission.
        let mut data_with_header = Vec::with_capacity(packet.data.len() + PACKET_HEADER_LEN);
        data_with_header.extend_from_slice(&u16::to_le_bytes(packet.data.len().try_into().expect("Packet is too big!")));
        data_with_header.extend_from_slice(&u16::to_le_bytes(packet.fds.len().try_into().expect("Packet has too many file descriptors!")));
        data_with_header.extend_from_slice(&packet.data);

        // Put the data in a format that libc expects.
        let slice = [IoSlice::new(&data_with_header)];
        let mut control_space = [0; rustix::cmsg_space!(ScmRights(32))];
        let mut control_buf = SendAncillaryBuffer::new(&mut control_space);
        let rights: Vec<BorrowedFd> = packet.fds.iter().map(|fd| fd.as_fd()).collect();
        let res = control_buf.push(SendAncillaryMessage::ScmRights(&rights));
        if !res {
            panic!("Failed to send file descriptors.")
        }

        // Send the data.
        let num_sent_bytes = rustix::net::sendmsg(
            &self,
            &slice,
            &mut control_buf,
            SendFlags::empty()
        )?;

        // It is possible that not all data is transmitted in a single call. Or even any amount of calls, in case the receiving
        // buffer is full. We need to think about how to handle that situation in the release version, but for experiment we just
        // panic if anything looks remotely funny.
        if num_sent_bytes != data_with_header.len() {
            panic!("Failed to transmit a packet within a single syscall!");
        }

        Ok(())
    }
}

impl std::os::fd::AsFd for StreamChannel {
    fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.fd.as_fd()
    }
}
