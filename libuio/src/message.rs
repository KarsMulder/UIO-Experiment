/// Reusing Wayland terminology, requests are messages from the client to the server.
#[derive(Serialize, Deserialize, Debug)]
pub enum RequestMsg {
    Announce(AnnounceMsg),
}

#[derive(Serialize, Deserialize, Debug)]
pub struct AnnounceMsg {
    pub name: String,
}

/// Events are messages from the server to the client.
#[derive(Serialize, Deserialize, Debug)]
pub enum EventMsg {
    AnnounceAccepted,
}
