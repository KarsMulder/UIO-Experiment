use libuio::message::AnnounceMsg;

use crate::state::Client;

enum ClientState {
    /// The client has not identified itself.
    Unknown,
}

impl ClientState {
    fn new() -> ClientState {
        ClientState::Unknown
    }
}

pub fn handle_ready_client(client: &mut Client) {
    for packet in client.channel_mut().read_packets().expect("Failed to read message!") {
        let (message, _fds) = packet.try_into_request().expect("Failed to parse packet as request!");
        println!("Received request: {message:?}");

        match message {
            libuio::message::RequestMsg::Announce(announcement) => {
                let AnnounceMsg { name } = announcement;
                println!("The client {name} connected.");
            }
        }
    }
}
