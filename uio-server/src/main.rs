#![allow(dead_code)]

mod handler;
mod state;
mod epoll;
mod poll;

use std::collections::HashMap;
use std::path::Path;

use anyhow::Context;
use epoll::Epoll;
use poll::PollId;
use libuio::socket::StreamSocket;
use rustix::fd::{AsFd, AsRawFd, RawFd};
use state::Client;

struct Program {
    epoll: Epoll<PollId>,
}

fn main() -> ! {
    // Ensure that the path to our socket is available.
    let path = Path::new(libuio::socket::DEFAULT_UIO_SOCKET_PATH);
    if path.exists() {
        std::fs::remove_file(path).expect("Failed to free the occupied socket path");
    }

    let dir = path.parent().expect("UIO socket path does not lie in a directory.");
    if !dir.exists() {
        std::fs::create_dir_all(dir).expect("Failed to create the directory containing the UIO socket.");
    }

    // Create the actual socket.
    let socket = StreamSocket::open(path.to_owned())
        .context("Failed to create a socket")
        .unwrap();

    let epoll: Epoll<PollId> = Epoll::new().expect("Failed to create an epoll instance.");
    epoll.add(&socket, PollId::Socket).expect("Failed to add socket to epoll.");

    // Identifies clients by the file descriptor of their channel.
    //
    // Using file descriptors for identification is handy because the kernel automatically manages them for us:
    // as long as a client with an open channel is in this hashmap, we are sure that its file descriptor is still
    // valid. When a client gets closed, its file descriptor can be reused, preventing some DoS attack that tries
    // to overflow our ID count by connecting and disconnecting a bazillion times.
    let mut clients: HashMap<RawFd, Client> = HashMap::new();

    println!("Socket created!");
    loop {
        let events = epoll.poll()
            .expect("Failed to poll from the epoll.");
        println!("Received {} events.", events.len());

        for event in events {
            match event {
                epoll::Message::Ready(key) => match key {
                    PollId::Client(raw_fd) => {
                        println!("Client ready.");
                        let Some(client) = clients.get_mut(&raw_fd) else { continue };
                        crate::handler::handle_ready_client(client);
                    },
                    PollId::Socket => {
                        println!("Socket ready.");
                        let channel = socket.accept().expect("Failed to accept incoming channel.");
                        let client = Client::new(channel);
                        let raw_fd = client.as_raw_fd();

                        epoll.add(&client, PollId::Client(raw_fd))
                            .expect("Failed to register a new client with the epoll!");

                        let old_client_using_fd = clients.insert(raw_fd, client);
                        
                        // It should be impossible that there was another client using the same file descriptor,
                        // because the file descriptor of a client cannot be closed without dropping the Client
                        // structure, and if the Client is dropped, then it can no longer occupy a spot in the
                        // HashMap. I am still asserting that anyway, because if that logic were to somehow fail,
                        // there'd probably be a security hole.
                        assert!(old_client_using_fd.is_none());
                    },
                },
                epoll::Message::Broken(key) | epoll::Message::Hup(key) => match key {
                    PollId::Client(raw_fd) => {
                        println!("Client broken.");
                        let Some(client) = clients.remove(&raw_fd) else { continue };
                        epoll.delete(client.channel().as_fd())
                            .expect("Failed to remove a client from the epoll!");
                    },
                    PollId::Socket => panic!("Socket broken!"),
                },
            }
        }
    }
}
