use std::time::{SystemTime, UNIX_EPOCH};

use crate::param::{MAX_DEPTH, MAX_NODES, MAX_TIME};

#[derive(Debug)]
pub struct Timer {
    start: i64,
    duration: i64,
    stopped: bool,
    // constants
    pub max_nodes: i64,
    pub max_depth: i8,
    pub opt_time: i64,
}

impl Timer {
    pub fn new() -> Self {
        Self {
            start: 0,
            duration: MAX_TIME,
            stopped: false,
            max_depth: MAX_DEPTH,
            max_nodes: MAX_NODES,
            opt_time: MAX_TIME,
        }
    }

    fn now() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
    }

    pub fn start(&mut self, duration: i64) {
        self.start = Self::now();
        self.duration = duration;
        self.stopped = false;
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

    pub fn test(&self, duration: i64) -> bool {
        Self::now() >= self.start + duration
    }

    pub fn delta(&self) -> i64 {
        Self::now() - self.start
    }

    /// Returns (opt_time, max_time)
    pub fn plan(moves: usize, time: i64, inc: i64) -> (i64, i64) {
        let overhead = 10;
        let moves_left = 30;
        let time_left = time + inc * (moves_left - 1) - overhead * (2 * moves_left);
        let opt_scale = f64::min(
            1.0 / (moves_left as f64),
            0.21 * time as f64 / time_left as f64,
        );
        let opt_time = i64::max(10, (opt_scale * time_left as f64) as i64);
        let max_time = i64::min(opt_time * 4, time * 80 / 100 - overhead).max(10);
        (opt_time, max_time)
    }
}
