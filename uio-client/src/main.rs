#![allow(dead_code)]

use std::path::Path;

use libuio::message::AnnounceMsg;
use rustix::event::{PollFd, PollFlags};
use libuio::socket::{Packet, SeqPacketChannel};

fn main() {
    // Ensure that the path to our socket is available.
    let path = Path::new(libuio::socket::DEFAULT_UIO_SOCKET_PATH);

    // Create the actual socket.
    let mut channel = SeqPacketChannel::open(path)
        .expect("Failed to connect to the UIO server!");

    println!("Connected to server!");

    let packet = Packet::try_from_request(libuio::message::RequestMsg::Announce(AnnounceMsg {
        name: "Experimental Client".to_owned()
    }), Vec::new()).unwrap();

    channel.write_packet(packet).expect("Failed to write packet!");

    loop {
        let mut to_poll = [PollFd::new(&channel, PollFlags::IN)];
        rustix::event::poll(&mut to_poll, 0).expect("Failed to poll");
        let events = to_poll[0].revents();
    
        if events.contains(PollFlags::IN) {
            println!("Received message!");
            let packet = channel.read_packet().expect("Failed to read message!");
            let (message, _fds) = packet.try_into_event().expect("Failed to parse packet as event!");
            println!("Received event: {message:?}");
        }
        if events.contains(PollFlags::ERR) {
            panic!("Channel broken!");
        }
        if events.contains(PollFlags::HUP) {
            panic!("Channel closed!");
        }
    }
}

