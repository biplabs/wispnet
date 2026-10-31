//! Precise Linux protocol timers, driven by the same reactor as the QUIC socket.

use std::{
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    pin::Pin,
    sync::Once,
    task::{Context, Poll, Waker, ready},
    time::Instant,
};

use noq::{AsyncTimer, Runtime as _};
use tokio::io::{Interest, unix::AsyncFd};

pub(super) fn new_timer(deadline: Instant) -> Pin<Box<dyn AsyncTimer>> {
    // Kernel timers cannot follow Tokio's paused or shifted test clock.
    if !uses_monotonic_clock() {
        return noq::TokioRuntime.new_timer(deadline);
    }
    match Timer::new(deadline) {
        Ok(timer) => Box::pin(timer),
        Err(error) => {
            warn_once("create", &error);
            noq::TokioRuntime.new_timer(deadline)
        }
    }
}

/// The real Tokio clock is a std::Instant sampled between these two reads.
/// This also detects virtual clocks in downstream tests, where cfg(test) on
/// Iroh itself is not set. Check again on reset/poll in case time was paused.
fn uses_monotonic_clock() -> bool {
    let before = Instant::now();
    let now = tokio::time::Instant::now().into_std();
    let after = Instant::now();
    before <= now && now <= after
}

fn warn_once(operation: &'static str, error: &io::Error) {
    static WARNED: Once = Once::new();
    WARNED.call_once(|| {
        tracing::warn!(%error, operation, "high-resolution QUIC timer unavailable; using Tokio timer");
    });
}

#[derive(Debug)]
enum Backend {
    Kernel(AsyncFd<OwnedFd>),
    Tokio(Pin<Box<dyn AsyncTimer>>),
}

#[derive(Debug)]
struct Timer {
    backend: Backend,
    deadline: Instant,
    waker: Option<Waker>,
}

impl Timer {
    fn new(deadline: Instant) -> io::Result<Self> {
        // SAFETY: timerfd_create has no pointer arguments and transfers ownership
        // of a newly created descriptor to us on success.
        let fd = unsafe {
            libc::timerfd_create(
                libc::CLOCK_MONOTONIC,
                libc::TFD_NONBLOCK | libc::TFD_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: fd is the newly created descriptor, with no other owner.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        // SAFETY: OwnedFd keeps this descriptor open and unchanged for the
        // registration's entire lifetime, including if registration fails.
        let io = unsafe { AsyncFd::register_with_interest(fd, Interest::READABLE)? };
        arm(&io, deadline)?;
        Ok(Self {
            backend: Backend::Kernel(io),
            deadline,
            waker: None,
        })
    }

    fn use_tokio(&mut self) {
        // Replacing the backend also closes/deregisters the kernel timer.
        self.backend = Backend::Tokio(noq::TokioRuntime.new_timer(self.deadline));
        // reset may replace an already-pending fd before the caller polls the
        // new Sleep. Preserve its wakeup instead of relying on an immediate poll.
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }

    fn failed(&mut self, operation: &'static str, error: io::Error) {
        warn_once(operation, &error);
        self.use_tokio();
    }
}

impl AsyncTimer for Timer {
    fn reset(self: Pin<&mut Self>, deadline: Instant) {
        let this = self.get_mut();
        this.deadline = deadline;
        match &mut this.backend {
            Backend::Tokio(timer) => timer.as_mut().reset(deadline),
            Backend::Kernel(_) if !uses_monotonic_clock() => this.use_tokio(),
            Backend::Kernel(io) => {
                // settime replaces the deadline and clears unread expirations.
                // Stale reactor readiness is cleared by try_io(EAGAIN).
                if let Err(error) = arm(io, deadline) {
                    this.failed("reset", error);
                }
            }
        }
    }

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        loop {
            if let Backend::Tokio(timer) = &mut this.backend {
                // Preserve stock behavior, including virtual-time semantics.
                return timer.as_mut().poll(cx);
            }
            if !uses_monotonic_clock() {
                this.use_tokio();
                continue;
            }
            // A stale readiness notification must never finish a reset deadline
            // early. Also handle deadlines already elapsed before polling.
            if Instant::now() >= this.deadline {
                this.waker = None;
                return Poll::Ready(());
            }
            let Backend::Kernel(io) = &this.backend else {
                unreachable!();
            };
            match poll_expiration(io, cx) {
                Poll::Pending => {
                    match &mut this.waker {
                        Some(waker) => waker.clone_from(cx.waker()),
                        None => this.waker = Some(cx.waker().clone()),
                    }
                    return Poll::Pending;
                }
                Poll::Ready(Ok(())) => {
                    if Instant::now() >= this.deadline {
                        this.waker = None;
                        return Poll::Ready(());
                    }
                    if let Err(error) = arm(io, this.deadline) {
                        this.failed("rearm", error);
                    }
                }
                Poll::Ready(Err(error)) => this.failed("read", error),
            }
        }
    }
}

fn arm(io: &AsyncFd<OwnedFd>, deadline: Instant) -> io::Result<()> {
    let duration = deadline.saturating_duration_since(Instant::now());
    let seconds = duration.as_secs().try_into().map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidInput, "timer deadline exceeds time_t")
    })?;
    // A zero timerfd deadline disarms it; poll handles elapsed deadlines itself.
    let nanos = if duration.is_zero() {
        1
    } else {
        duration.subsec_nanos()
    };
    let spec = libc::itimerspec {
        it_interval: libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        },
        it_value: libc::timespec {
            tv_sec: seconds,
            // Subsecond nanoseconds fit even a 32-bit Linux c_long.
            tv_nsec: nanos as libc::c_long,
        },
    };
    // SAFETY: io owns a valid timerfd and spec points to an initialized value.
    let result =
        unsafe { libc::timerfd_settime(io.get_ref().as_raw_fd(), 0, &spec, std::ptr::null_mut()) };
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn poll_expiration(io: &AsyncFd<OwnedFd>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
    loop {
        let mut readiness = ready!(io.poll_read_ready(cx))?;
        match readiness.try_io(|io| read_expiration(io.get_ref())) {
            Ok(result) => return Poll::Ready(result),
            // Poll readiness again to register the current waker after clearing
            // stale readiness. Returning Pending immediately could miss a wake.
            Err(_would_block) => continue,
        }
    }
}

fn read_expiration(fd: &OwnedFd) -> io::Result<()> {
    let mut expirations = [0u8; 8];
    loop {
        // SAFETY: fd remains open and expirations is a writable eight-byte buffer.
        let count = unsafe {
            libc::read(
                fd.as_raw_fd(),
                expirations.as_mut_ptr().cast(),
                expirations.len(),
            )
        };
        if count == expirations.len() as isize {
            return Ok(());
        }
        if count < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "short timerfd expiration read",
        ));
    }
}

#[cfg(test)]
mod tests {
    use std::{
        future::poll_fn,
        os::unix::net::UnixStream,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        task::Wake,
        time::Duration,
    };

    use super::*;

    async fn wait(timer: &mut Pin<Box<impl AsyncTimer + ?Sized>>) {
        poll_fn(|cx| timer.as_mut().poll(cx)).await;
    }

    #[tokio::test]
    async fn never_completes_before_deadline() {
        assert!(uses_monotonic_clock());
        for micros in [1, 25, 50, 100, 300, 800, 1_500, 5_000] {
            let deadline = Instant::now() + Duration::from_micros(micros);
            let mut timer = Box::pin(Timer::new(deadline).unwrap());
            wait(&mut timer).await;
            assert!(Instant::now() >= deadline);
        }
    }

    #[tokio::test]
    async fn reset_postpones_deadline() {
        let mut timer = Box::pin(Timer::new(Instant::now() + Duration::from_millis(1)).unwrap());
        let later = Instant::now() + Duration::from_secs(60);
        timer.as_mut().reset(later);
        let mut cx = Context::from_waker(Waker::noop());
        assert!(timer.as_mut().poll(&mut cx).is_pending());
        let later = Instant::now() + Duration::from_millis(5);
        timer.as_mut().reset(later);
        wait(&mut timer).await;
        assert!(Instant::now() >= later);
    }

    #[tokio::test]
    async fn unread_expiry_can_be_rearmed() {
        let mut timer = Box::pin(Timer::new(Instant::now() + Duration::from_millis(1)).unwrap());
        // Leave the kernel expiration unread before replacing the deadline.
        tokio::time::sleep(Duration::from_millis(5)).await;
        let Backend::Kernel(io) = &timer.backend else {
            unreachable!();
        };
        // Retain cached reactor readiness without consuming the expiration.
        drop(poll_fn(|cx| io.poll_read_ready(cx)).await.unwrap());
        let later = Instant::now() + Duration::from_secs(60);
        timer.as_mut().reset(later);
        let mut cx = Context::from_waker(Waker::noop());
        assert!(timer.as_mut().poll(&mut cx).is_pending());
        let later = Instant::now() + Duration::from_millis(5);
        timer.as_mut().reset(later);
        wait(&mut timer).await;
        assert!(Instant::now() >= later);
    }

    #[tokio::test]
    async fn reset_can_move_deadline_earlier() {
        let mut timer = Box::pin(Timer::new(Instant::now() + Duration::from_secs(60)).unwrap());
        let earlier = Instant::now() + Duration::from_millis(1);
        timer.as_mut().reset(earlier);
        tokio::time::timeout(Duration::from_secs(2), wait(&mut timer))
            .await
            .unwrap();
        assert!(Instant::now() >= earlier);
    }

    #[tokio::test]
    async fn elapsed_timer_can_be_polled_and_reset_again() {
        let mut timer = Box::pin(Timer::new(Instant::now()).unwrap());
        wait(&mut timer).await;
        let mut cx = Context::from_waker(Waker::noop());
        assert!(timer.as_mut().poll(&mut cx).is_ready());
        let deadline = Instant::now() + Duration::from_millis(1);
        timer.as_mut().reset(deadline);
        wait(&mut timer).await;
        assert!(Instant::now() >= deadline);
        timer.as_mut().reset(Instant::now());
        assert!(timer.as_mut().poll(&mut cx).is_ready());
    }

    fn non_timer_fd(deadline: Instant) -> (Pin<Box<Timer>>, UnixStream) {
        let (reader, writer) = UnixStream::pair().unwrap();
        reader.set_nonblocking(true).unwrap();
        let fd: OwnedFd = reader.into();
        // SAFETY: OwnedFd keeps the descriptor stable until deregistration.
        let io = unsafe { AsyncFd::register_with_interest(fd, Interest::READABLE).unwrap() };
        (
            Box::pin(Timer {
                backend: Backend::Kernel(io),
                deadline,
                waker: None,
            }),
            writer,
        )
    }

    #[derive(Default)]
    struct Notified(AtomicBool);

    impl Wake for Notified {
        fn wake(self: Arc<Self>) {
            self.0.store(true, Ordering::Relaxed);
        }
    }

    #[tokio::test]
    async fn rearm_failure_falls_back_and_wakes_pending_task() {
        let (mut timer, _writer) = non_timer_fd(Instant::now() + Duration::from_secs(60));
        let notified = Arc::new(Notified::default());
        let waker = Waker::from(notified.clone());
        let mut cx = Context::from_waker(&waker);
        assert!(timer.as_mut().poll(&mut cx).is_pending());
        // A valid socket fd is safe to pass, but timerfd_settime returns EINVAL.
        let deadline = Instant::now() + Duration::from_millis(5);
        timer.as_mut().reset(deadline);
        assert!(matches!(timer.backend, Backend::Tokio(_)));
        assert!(notified.0.load(Ordering::Relaxed));
        wait(&mut timer).await;
        assert!(Instant::now() >= deadline);
        let later = Instant::now() + Duration::from_millis(1);
        timer.as_mut().reset(later);
        wait(&mut timer).await;
        assert!(Instant::now() >= later);
    }

    #[tokio::test]
    async fn read_failure_falls_back_at_current_deadline() {
        let deadline = Instant::now() + Duration::from_secs(60);
        let (mut timer, writer) = non_timer_fd(deadline);
        drop(writer); // Readable EOF exercises the short expiration read path.
        let mut cx = Context::from_waker(Waker::noop());
        // Await readiness explicitly so a delayed test still exercises the read.
        let Backend::Kernel(io) = &timer.backend else {
            unreachable!();
        };
        drop(poll_fn(|cx| io.poll_read_ready(cx)).await.unwrap());
        assert!(timer.as_mut().poll(&mut cx).is_pending());
        assert!(matches!(timer.backend, Backend::Tokio(_)));
        let deadline = Instant::now() + Duration::from_millis(5);
        timer.as_mut().reset(deadline);
        wait(&mut timer).await;
        assert!(Instant::now() >= deadline);
    }

    #[tokio::test(start_paused = true)]
    async fn paused_clock_uses_tokio_timer() {
        assert!(!uses_monotonic_clock());
        let deadline = (tokio::time::Instant::now() + Duration::from_secs(60)).into_std();
        let mut timer = new_timer(deadline);
        wait(&mut timer).await;
        assert!(Instant::now() < deadline);
        assert!(tokio::time::Instant::now().into_std() >= deadline);
    }

    #[tokio::test]
    async fn pausing_after_creation_switches_to_tokio_timer() {
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut timer = Box::pin(Timer::new(deadline).unwrap());
        let mut cx = Context::from_waker(Waker::noop());
        assert!(timer.as_mut().poll(&mut cx).is_pending());
        tokio::time::pause();
        wait(&mut timer).await;
        assert!(Instant::now() < deadline);
        assert!(matches!(timer.backend, Backend::Tokio(_)));
    }
}
