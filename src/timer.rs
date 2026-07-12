use std::time::{SystemTime, UNIX_EPOCH};

pub struct Timer {
    start: u128,
    duration: u128,
    stopped: bool,
}

impl Timer {
    pub fn new() -> Self {
        Self {
            start: 0,
            duration: 0,
            stopped: false,
        }
    }

    fn now() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis()
    }

    pub fn start(&mut self, duration: u128) {
        self.start = Self::now();
        self.duration = duration;
    }

    pub fn check(&mut self) {
        if self.stopped {
            return;
        }

        if Self::now() >= self.start + self.duration {
            self.stopped = true;
        }
    }

    pub fn stopped(&self) -> bool {
        self.stopped
    }

    pub fn force_stop(&mut self) {
        self.stopped = true;
    }

    pub fn test(&self, duration: u128) -> bool {
        Self::now() >= self.start + duration
    }

    pub fn delta(&self) -> u128 {
        Self::now() - self.start
    }
}
