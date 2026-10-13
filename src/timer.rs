use std::{
    sync::atomic::{AtomicBool, AtomicI8, AtomicI64, Ordering},
    time::Instant,
};

use crate::param::{MAX_DEPTH, MAX_NODES, MAX_TIME};

#[derive(Debug)]
pub struct Timer {
    instant: Instant,
    start: AtomicI64,
    duration: AtomicI64,
    stopped: AtomicBool,
    pondering: AtomicBool,
    // constants
    pub max_nodes: AtomicI64,
    pub max_depth: AtomicI8,
    pub opt_time: AtomicI64,
}

impl Timer {
    pub fn new() -> Self {
        Self {
            instant: Instant::now(),
            start: AtomicI64::new(0),
            duration: AtomicI64::new(MAX_TIME),
            stopped: AtomicBool::new(false),
            pondering: AtomicBool::new(false),
            max_depth: AtomicI8::new(MAX_DEPTH),
            max_nodes: AtomicI64::new(MAX_NODES),
            opt_time: AtomicI64::new(MAX_TIME),
        }
    }

    fn elapsed(&self) -> i64 {
        self.instant.elapsed().as_millis() as i64 - self.start.load(Ordering::Relaxed)
    }

    pub fn start(&self, duration: i64, pondering: bool) {
        self.start
            .store(self.instant.elapsed().as_millis() as i64, Ordering::Relaxed);
        self.duration.store(duration, Ordering::Relaxed);
        self.stopped.store(false, Ordering::Relaxed);
        // this is set last because ponderhit sets this from true -> false
        self.pondering.store(pondering, Ordering::Release);
    }

    pub fn check(&self) {
        if self.stopped() {
            return;
        }

        if self.elapsed() >= self.duration.load(Ordering::Relaxed) {
            self.stopped.store(true, Ordering::Relaxed);
        }
    }

    pub fn stopped(&self) -> bool {
        let pondering = self.pondering.load(Ordering::Acquire);
        !pondering && self.stopped.load(Ordering::Relaxed)
    }

    pub fn force_stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        self.pondering.store(false, Ordering::Relaxed);
    }

    pub fn test(&self, duration: i64) -> bool {
        !self.pondering.load(Ordering::Relaxed) && self.elapsed() >= duration
    }

    pub fn delta(&self) -> i64 {
        self.elapsed()
    }

    /// Returns (opt_time, max_time)
    pub fn plan(_moves: usize, time: i64, inc: i64) -> (i64, i64) {
        let overhead = 5;
        let moves_left = 40;
        let time_left = (time + inc * (moves_left - 1) - overhead * (2 * moves_left)).max(10);
        let opt_scale = f64::min(
            1.0 / (moves_left as f64),
            0.21 * time as f64 / time_left as f64,
        );
        let opt_time = i64::max(10, (opt_scale * time_left as f64) as i64);
        let max_time = i64::min(opt_time * 4, time * 80 / 100 - overhead).max(10);
        (opt_time, max_time)
    }
}
