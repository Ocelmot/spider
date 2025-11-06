
use std::{
    future::Future,
    pin::Pin,
    task::Poll, time::Duration,
};

use tokio::time::{sleep_until, Instant, Sleep};

pub(crate) struct Deadline {
    deadline: Option<Pin<Box<Sleep>>>,
    repeat: Option<Duration>,
}

impl Deadline {
    pub(crate) fn new() -> Self {
        Self { deadline: None, repeat: None }
    }

    pub(crate) fn new_deadline(deadline: Instant) -> Self {
        Self {
            deadline: Some(Box::pin(sleep_until(deadline))),
            repeat: None,
        }
    }

    pub(crate) fn new_repeat(interval: Duration) -> Self {
        let deadline = Instant::now() + interval;
        Self {
            deadline: Some(Box::pin(sleep_until(deadline))),
            repeat: Some(interval),
        }
    }

    pub(crate) fn set_deadline_from_now(&mut self, deadline: Duration) {
        let deadline = Instant::now() + deadline;
        match &mut self.deadline {
            Some(sleep) => sleep.as_mut().reset(deadline),
            None => self.deadline = Some(Box::pin(sleep_until(deadline))),
        }
    }

    pub(crate) fn clear(&mut self) {
        self.deadline = None;
        self.repeat = None;
    }

    pub(crate) fn has_deadline(&mut self) -> bool {
        self.deadline.is_some()
    }

    pub(crate) fn set_repeat(&mut self, repeat: Duration) {
        if self.deadline.is_none() {
            let deadline = Instant::now() + repeat;
            self.deadline = Some(Box::pin(sleep_until(deadline)));
        }
        self.repeat = Some(repeat);
    }
}

impl Future for Deadline {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let s = self.get_mut();
        match &mut s.deadline {
            Some(sleep) => {
                let result = sleep.as_mut().poll(cx);
                if result.is_ready() {
                    if let Some(repeat) = s.repeat {
                        let deadline = Instant::now() + repeat;
                        sleep.as_mut().reset(deadline);
                    } else {
                        s.deadline = None;
                    }
                }
                result
            }
            None => Poll::Pending,
        }
    }
}

impl Clone for Deadline {
    fn clone(&self) -> Self {
        let deadline = match &self.deadline{
            Some(sleep) => {
                Some(Box::pin(sleep_until(sleep.deadline())))
            },
            None => None,
        };
        let repeat = self.repeat.clone();
        Self { deadline, repeat }
    }
}
