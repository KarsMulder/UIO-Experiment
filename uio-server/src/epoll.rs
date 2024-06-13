use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::os::fd::{OwnedFd, AsFd};

use rustix::event::epoll::{EventData, EventFlags};
use rustix::fd::AsRawFd;

/// Contains all the open communication channels from all clients.
/// 
/// When registering a new FD to listen, you need to pass it a key (type K) which will be returned by
/// this epoll when that file is ready. Similar to how you pass an u64 to the epoll() systcall, but now
/// without the requirement that the key is an u64.
/// 
/// It is surprisingly difficult to make a fully "safe" wrapper around the Linux epoll interface, if safe
/// means that it is impossible to register invalid file descriptors, impossible to unregister invalid
/// file descriptors, impossible for a file descriptor to be closed while registered by the epoll.
/// 
/// Any scheme I have been able to think of would result in such convoluted interface that the program would
/// be much more likely to break due to supposedly "safe" errors, than that the program breaks due to "unsafe"
/// errors under a simple interface.
/// 
/// Also, the "worst" thing that can happen if we pass invalid file descriptors to the epoll is that the epoll
/// starts reporting bogus messages or fails to report safe errors. That's why most crates just blanket mark
/// epoll related functionality as "safe".
/// 
/// # Panics
/// Panics if K::try_from(u64::from(key)) returns an error. It must always be possible do a round-trip
/// conversion K -> u64 -> K.
pub struct Epoll<K> {
    epoll_fd: OwnedFd,
    _key: PhantomData<K>,
}

pub enum Message<K> {
    // Represents a EPOLLIN message.
    Ready(K),

    // Represents a EPOLLERR message.
    Broken(K),
    // Represents a EPOLLHUP message that is not simultaneously EPOLLERR.
    Hup(K),
}

impl<K> Epoll<K> {
    pub fn new() -> std::io::Result<Self> {
        Ok(Self {
            epoll_fd: rustix::event::epoll::create(rustix::event::epoll::CreateFlags::CLOEXEC)?,
            _key: PhantomData,
        })
    }

    pub fn delete(&self, file: impl AsFd) -> std::io::Result<()> {
        rustix::event::epoll::delete(
            &self.epoll_fd,
            file.as_fd()
        ).map_err(std::io::Error::from)
    }
}

impl<K: Into<u64>> Epoll<K> {
    pub fn add(&self, file: impl AsFd, key: K) -> std::io::Result<()> {
        rustix::event::epoll::add(
            &self.epoll_fd,
            file.as_fd(),
            EventData::new_u64(key.into()),
            EventFlags::IN | EventFlags::OUT | EventFlags::ERR
        ).map_err(std::io::Error::from)
    }
}

impl<K: TryFrom<u64>> Epoll<K> {
    pub fn poll(&self) -> std::io::Result<Vec<Message<K>>> {
        // For some reason, rustix decided to make their epoll event structure packed.
        // Which means I can't read its flags field in safe Rust.
        // So I am going to just do the polling with libc instead.
        let mut event_list = rustix::event::epoll::EventVec::with_capacity(8);
        rustix::event::epoll::wait(&self.epoll_fd, &mut event_list, -1)?;

        let mut event_list: [MaybeUninit<libc::epoll_event>; 8] = [MaybeUninit::uninit(); 8];
        let num_events = unsafe { libc::epoll_wait(
            self.epoll_fd.as_raw_fd(),
            &mut event_list as *mut _ as *mut libc::epoll_event,
            event_list.len() as i32,
            -1
        ) };
        if num_events < 0 {
            return Err(std::io::Error::last_os_error());
        }

        let mut result = Vec::new();
        for i in 0 .. (num_events as usize) {
            let event = unsafe { event_list[i].assume_init() };
            let flags = event.events as i32;
            let key = match event.u64.try_into() {
                Ok(key) => key,
                Err(_) => panic!("Failed to convert an u64 back to a poll key."),
            };

            if flags & libc::EPOLLIN != 0 {
                result.push(Message::Ready(key));
                continue;
            }
            if flags & libc::EPOLLIN != 0 {
                result.push(Message::Broken(key));
                continue;
            }
            if flags & libc::EPOLLIN != 0 {
                result.push(Message::Hup(key));
                continue;
            }
        }

        Ok(result)
    }
}
