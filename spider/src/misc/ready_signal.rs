//! A signal to indicate if something is ready. This coalesces multiple ready
//! events into a single signal. The signal is acquired, and then must be
//! consumed once processing is complete.

use tokio::sync::Notify;

pub struct ReadySignal {
    notify: Notify,
}

impl ReadySignal {
    /// Create a new ReadySignal
    pub fn new() -> Self {
        Self {
            notify: Notify::new(),
        }
    }

    pub fn signal(&self) {
        self.notify.notify_one();
    }

    pub async fn wait(&self) -> ReadySignalGuard<'_> {
        self.notify.notified().await;
        ReadySignalGuard::new(&self.notify)
    }
}

pub struct ReadySignalGuard<'a> {
    notify: &'a Notify,
    consumed: bool,
}

impl<'a> ReadySignalGuard<'a> {
    fn new(notify: &'a Notify) -> Self {
        Self {
            notify,
            consumed: false,
        }
    }

    pub fn consume(mut self) {
        self.consumed = true;
    }
}

impl Drop for ReadySignalGuard<'_> {
    fn drop(&mut self) {
        // If this wasnt consumed, return the permit
        if !self.consumed {
            self.notify.notify_one();
        }
    }
}
