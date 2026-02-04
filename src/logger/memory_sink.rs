use super::{QueryLogEntry, QueryLogSink};
use std::collections::VecDeque;
use std::sync::{Arc, RwLock};

pub struct MemoryLogSink {
    buffer: Arc<RwLock<VecDeque<QueryLogEntry>>>,
    capacity: usize,
}

impl MemoryLogSink {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: Arc::new(RwLock::new(VecDeque::with_capacity(capacity))),
            capacity,
        }
    }

    pub fn get_recent(&self) -> Vec<QueryLogEntry> {
        let buffer = self.buffer.read().unwrap();
        buffer.iter().cloned().collect()
    }

    // Allow sharing the buffer with API handlers
    pub fn clone_buffer(&self) -> Arc<RwLock<VecDeque<QueryLogEntry>>> {
        self.buffer.clone()
    }
}

impl QueryLogSink for MemoryLogSink {
    fn log(&self, entry: &QueryLogEntry) {
        let mut buffer = self.buffer.write().unwrap();
        if buffer.len() >= self.capacity {
            buffer.pop_front();
        }
        buffer.push_back(entry.clone());
    }
}
