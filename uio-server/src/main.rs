#![allow(dead_code)]

use std::path::Path;

use anyhow::Context;
use rustix::event::{PollFd, PollFlags};

use libuio::socket::SeqPacketChannel;
use libuio::socket::SeqPacketSocket;

fn main() {
    // Ensure that the path to our socket is available.
    let path = Path::new(libuio::socket::DEFAULT_UIO_SOCKET_PATH);
    let dir = path.parent().expect("UIO socket path does not lie in a directory.");
    if !dir.exists() {
        std::fs::create_dir_all(dir).expect("Failed to create the directory containing the UIO socket.");
    }

    // Create the actual socket.
    let socket = SeqPacketSocket::open(path.to_owned())
        .context("Failed to create a socket")
        .unwrap();

    println!("Socket created!");
    loop {
        let mut to_poll = [PollFd::new(&socket, PollFlags::IN)];
        rustix::event::poll(&mut to_poll, 0).expect("Failed to poll");
        let socket_events = to_poll[0].revents();
        if socket_events.contains(PollFlags::IN) {
            let channel = socket.accept().expect("Failed to accept incoming channel.");
            std::thread::spawn(|| handle_channel(channel));
        }
        if socket_events.contains(PollFlags::ERR) {
            panic!("Socket broken!");
        }
    }
}

fn handle_channel(mut channel: SeqPacketChannel) {
    println!("Handling channel!");

    let mut to_poll = [PollFd::new(&channel, PollFlags::IN)];
    rustix::event::poll(&mut to_poll, 0).expect("Failed to poll");
    let events = to_poll[0].revents();

    if events.contains(PollFlags::IN) {
        println!("Received message!");
        let _ = channel.read_packet().expect("Failed to read message!");
    }
    if events.contains(PollFlags::ERR) {
        panic!("Channel broken!");
    }
    if events.contains(PollFlags::HUP) {
        panic!("Channel closed!");
    }
}
