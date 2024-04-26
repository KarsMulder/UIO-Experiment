/// Reusing Wayland terminology, requests are messages from the client to the server.
#[derive(Serialize, Deserialize)]
pub enum RequestMsg {
    Announce(AnnounceMsg),
}

#[derive(Serialize, Deserialize)]
pub struct AnnounceMsg {
    name: String,
}

/// Events are messages from the server to the client.
#[derive(Serialize, Deserialize)]
pub enum EventMsg {
    AnnounceAccepted,
}
