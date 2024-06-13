
use libuio::socket::StreamChannel;
use std::os::fd::{AsFd, AsRawFd};

pub struct Client {
    channel: StreamChannel,
}

impl AsFd for Client {
    fn as_fd(&self) -> std::os::unix::prelude::BorrowedFd<'_> {
        self.channel.as_fd()
    }
}

impl AsRawFd for Client {
    fn as_raw_fd(&self) -> std::os::unix::prelude::RawFd {
        self.as_fd().as_raw_fd()
    }
}

impl Client {
    pub fn new(channel: StreamChannel) -> Self {
        Self {
            channel
        }
    }

    pub fn channel(&self) -> &StreamChannel {
        &self.channel
    }

    pub fn channel_mut(&mut self) -> &mut StreamChannel {
        &mut self.channel
    }
}

