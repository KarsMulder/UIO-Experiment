use std::path::Path;

use anyhow::Context;

use crate::socket::SeqPacketSocket;

mod socket;
mod fs_utils;

#[macro_use]
extern crate anyhow;

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
    println!("Hello, world!");
}
