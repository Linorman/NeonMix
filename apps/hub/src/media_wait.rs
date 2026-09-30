//! Socket/native-sample driven media wakeups, outside the audio callback.
//! macOS uses a one-shot critical kqueue timer for the next source/pacing
//! deadline. This requests minimal timer coalescing, not realtime scheduling.
#![allow(unsafe_code)] // Register a documented macOS timer on our owned kqueue.
use mio::{Events, Interest, Poll, Token, Waker};
use std::{
    io,
    sync::Arc,
    time::{Duration, Instant},
};

const RECEIVE_BURST: usize = 32;
const RECEIVE_COOLDOWN: Duration = Duration::from_millis(1);

pub struct MediaWait {
    poll: Poll,
    events: Events,
    wake: Arc<Waker>,
    socket: mio::net::UdpSocket,
    received_in_burst: usize,
    receive_window: Instant,
    resume_read_at: Option<Instant>,
    pub receive_throttles: u64,
}
impl MediaWait {
    pub fn new(socket: std::net::UdpSocket) -> io::Result<Self> {
        socket.set_nonblocking(true)?;
        let poll = Poll::new()?;
        let mut socket = mio::net::UdpSocket::from_std(socket);
        poll.registry()
            .register(&mut socket, Token(0), Interest::READABLE)?;
        let wake = Arc::new(Waker::new(poll.registry(), Token(1))?);
        Ok(Self {
            poll,
            events: Events::with_capacity(8),
            wake,
            socket,
            received_in_burst: 0,
            receive_window: Instant::now(),
            resume_read_at: None,
            receive_throttles: 0,
        })
    }
    pub fn waker(&self) -> Arc<Waker> {
        self.wake.clone()
    }

    pub fn send(&self, packet: &[u8]) -> io::Result<usize> {
        self.socket.send(packet)
    }

    pub fn recv(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.recv_at(bytes, Instant::now())
    }

    fn resume_reading(&mut self, now: Instant) -> io::Result<()> {
        if self.resume_read_at.is_some_and(|deadline| now >= deadline) {
            self.poll
                .registry()
                .register(&mut self.socket, Token(0), Interest::READABLE)?;
            self.resume_read_at = None;
        }
        Ok(())
    }

    fn recv_at(&mut self, bytes: &mut [u8], now: Instant) -> io::Result<usize> {
        self.resume_reading(now)?;
        if self.resume_read_at.is_some() {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        if now.saturating_duration_since(self.receive_window) >= RECEIVE_COOLDOWN {
            self.receive_window = now;
            self.received_in_burst = 0;
        }
        match self.socket.recv(bytes) {
            Ok(n) => {
                self.received_in_burst += 1;
                if self.received_in_burst == RECEIVE_BURST {
                    // Remove socket readiness during the cooldown. Other
                    // native-sample wakes still run PCM, pacing and control,
                    // but cannot bypass the ingress work budget.
                    self.poll.registry().deregister(&mut self.socket)?;
                    self.resume_read_at = Some(self.receive_window + RECEIVE_COOLDOWN);
                    self.received_in_burst = 0;
                    self.receive_throttles = self.receive_throttles.saturating_add(1);
                }
                Ok(n)
            }
            Err(error) => Err(error),
        }
    }

    /// Readiness is a hint: callers always drain bounded work before waiting.
    /// Callers set a short retry deadline when their drain budget is exhausted,
    /// so edge-triggered readiness cannot strand already queued datagrams/PCM.
    /// A zero duration means a source/pacing deadline is already due.
    pub fn wait(&mut self, duration: Duration) -> io::Result<()> {
        let now = Instant::now();
        self.resume_reading(now)?;
        let duration = self.resume_read_at.map_or(duration, |deadline| {
            duration.min(deadline.saturating_duration_since(now))
        });
        if duration.is_zero() {
            return Ok(());
        }
        #[cfg(target_os = "macos")]
        {
            use std::os::fd::AsRawFd;
            let timer = libc::kevent {
                ident: 2,
                filter: libc::EVFILT_TIMER,
                flags: libc::EV_ADD | libc::EV_ONESHOT,
                fflags: libc::NOTE_NSECONDS | libc::NOTE_CRITICAL,
                data: duration.as_nanos().min(isize::MAX as u128) as isize,
                udata: std::ptr::null_mut(),
            };
            // SAFETY: Registry owns this kqueue; timer is initialized and valid
            // for the syscall. No output events or timeout pointers are supplied.
            if unsafe {
                libc::kevent(
                    self.poll.as_raw_fd(),
                    &timer,
                    1,
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null(),
                )
            } < 0
            {
                let error = io::Error::last_os_error();
                return if error.kind() == io::ErrorKind::Interrupted {
                    Ok(())
                } else {
                    Err(error)
                };
            }
        }
        // Keep a bounded fallback timeout as well. The timer may wake sooner;
        // sockets and appsink notifications wake immediately on all platforms.
        match self.poll.poll(&mut self.events, Some(duration)) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => Ok(()),
            result => result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{net::UdpSocket, time::Instant};

    #[test]
    fn socket_and_preexisting_native_notifications_wake_without_timer_polling() {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let address = socket.local_addr().unwrap();
        let mut wait = MediaWait::new(socket).unwrap();
        wait.waker().wake().unwrap();
        let start = Instant::now();
        wait.wait(Duration::from_secs(2)).unwrap();
        assert!(start.elapsed() < Duration::from_secs(1));
        let source = UdpSocket::bind("127.0.0.1:0").unwrap();
        source.send_to(b"wake", address).unwrap();
        let start = Instant::now();
        wait.wait(Duration::from_secs(2)).unwrap();
        assert!(start.elapsed() < Duration::from_secs(1));
        let mut bytes = [0; 4];
        assert_eq!(wait.socket.recv_from(&mut bytes).unwrap().0, 4);
        assert_eq!(&bytes, b"wake");
        // The timer remains usable after socket and user events interrupt it.
        wait.wait(Duration::from_millis(2)).unwrap();
    }

    #[test]
    fn continuous_datagrams_cannot_bypass_cooldown_with_native_wakes() {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let address = socket.local_addr().unwrap();
        let mut wait = MediaWait::new(socket).unwrap();
        let peer = UdpSocket::bind("127.0.0.1:0").unwrap();
        for n in 0..64u8 {
            peer.send_to(&[n], address).unwrap();
        }
        let now = Instant::now();
        let mut bytes = [0u8; 4];
        for n in 0..32u8 {
            assert_eq!(wait.recv_at(&mut bytes, now).unwrap(), 1);
            assert_eq!(bytes[0], n);
        }
        wait.waker().wake().unwrap();
        assert_eq!(
            wait.recv_at(&mut bytes, now).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        // The unread half remains on the real socket and is delivered in order
        // when the monotonic cooldown expires; notification timing cannot skip it.
        let resumed = now + RECEIVE_COOLDOWN;
        for n in 32..64u8 {
            assert_eq!(wait.recv_at(&mut bytes, resumed).unwrap(), 1);
            assert_eq!(bytes[0], n);
        }
        assert_eq!(wait.receive_throttles, 2);
        assert_eq!(
            wait.recv_at(&mut bytes, resumed).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn brief_empty_socket_intervals_do_not_reset_the_receive_window() {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let address = socket.local_addr().unwrap();
        let mut wait = MediaWait::new(socket).unwrap();
        let peer = UdpSocket::bind("127.0.0.1:0").unwrap();
        let now = Instant::now();
        let mut bytes = [0; 4];
        for n in 0..32u8 {
            peer.send_to(&[n], address).unwrap();
            // Kernel loopback delivery can be asynchronous. Wait for real
            // readiness while keeping the budget's injected clock fixed.
            wait.poll
                .poll(&mut wait.events, Some(Duration::from_secs(1)))
                .unwrap();
            assert_eq!(wait.recv_at(&mut bytes, now).unwrap(), 1);
            assert_eq!(
                wait.recv_at(&mut bytes, now).unwrap_err().kind(),
                io::ErrorKind::WouldBlock
            );
        }
        peer.send_to(&[32], address).unwrap();
        assert_eq!(
            wait.recv_at(&mut bytes, now).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(wait.receive_throttles, 1);
        let resumed = now + RECEIVE_COOLDOWN;
        wait.resume_reading(resumed).unwrap();
        wait.poll
            .poll(&mut wait.events, Some(Duration::from_secs(1)))
            .unwrap();
        assert_eq!(wait.recv_at(&mut bytes, resumed).unwrap(), 1);
        assert_eq!(bytes[0], 32);
    }
}
