use std::sync::{Arc, RwLock};
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct AppState {
    // If Some(Instant), blocking is paused until that instant.
    // If None or Instant passed, blocking is active.
    blocking_paused_until: Arc<RwLock<Option<Instant>>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            blocking_paused_until: Arc::new(RwLock::new(None)),
        }
    }

    pub fn is_blocking_active(&self) -> bool {
        let guard = self.blocking_paused_until.read().unwrap();
        if let Some(until) = *guard {
            if Instant::now() < until {
                return false;
            }
        }
        true
    }

    pub fn pause_blocking(&self, duration: std::time::Duration) {
        let mut guard = self.blocking_paused_until.write().unwrap();
        *guard = Some(Instant::now() + duration);
    }

    pub fn resume_blocking(&self) {
        let mut guard = self.blocking_paused_until.write().unwrap();
        *guard = None;
    }

    pub fn get_pause_remaining_secs(&self) -> Option<u64> {
        let guard = self.blocking_paused_until.read().unwrap();
        if let Some(until) = *guard {
            let now = Instant::now();
            if until > now {
                return Some(until.duration_since(now).as_secs());
            }
        }
        None
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
