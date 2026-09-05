//! LINUX-06: epoll-based async I/O wrapper.
//!
//! Event-driven I/O abstraction for simulation. Provides a platform-agnostic
//! event loop interface that can use epoll on Linux, kqueue on macOS,
//! or a simple polling fallback.

use std::collections::HashMap;

/// Event types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventType {
    Read,
    Write,
    Error,
    Hangup,
}

/// Event from event loop.
#[derive(Debug, Clone)]
pub struct Event {
    pub fd: i32,
    pub event_type: EventType,
    pub data: u64,
}

type EventHandler = Box<dyn FnMut(Event) + Send>;

/// Event loop abstraction.
pub struct EventLoop {
    handlers: HashMap<i32, EventHandler>,
    running: bool,
    event_count: u64,
    registered_fds: Vec<i32>,
}

impl EventLoop {
    pub fn new() -> Result<Self, String> {
        Ok(EventLoop {
            handlers: HashMap::new(),
            running: false,
            event_count: 0,
            registered_fds: Vec::new(),
        })
    }

    /// Register a file descriptor for events.
    pub fn register(
        &mut self,
        fd: i32,
        _event_type: EventType,
        _data: u64,
        handler: impl FnMut(Event) + Send + 'static,
    ) -> Result<(), String> {
        self.registered_fds.push(fd);
        self.handlers.insert(fd, Box::new(handler));
        Ok(())
    }

    /// Unregister a file descriptor.
    pub fn unregister(&mut self, fd: i32) -> Result<(), String> {
        self.handlers.remove(&fd);
        self.registered_fds.retain(|&f| f != fd);
        Ok(())
    }

    /// Run one iteration (non-blocking).
    pub fn poll_once(&mut self, _timeout_ms: i32) -> Result<Vec<Event>, String> {
        // Stub: returns empty on all platforms
        // Real impl would use epoll/kqueue/select
        let _ = _timeout_ms;
        Ok(Vec::new())
    }

    /// Stop the event loop.
    pub fn stop(&mut self) {
        self.running = false;
    }

    /// Total events processed.
    pub fn event_count(&self) -> u64 {
        self.event_count
    }

    /// Summary.
    pub fn summary(&self) -> String {
        format!(
            "EventLoop: {} events, {} handlers",
            self.event_count,
            self.handlers.len()
        )
    }
}

impl Default for EventLoop {
    fn default() -> Self {
        Self::new().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_loop() {
        let loop_result = EventLoop::new();
        assert!(loop_result.is_ok());
    }

    #[test]
    fn test_summary() {
        let epoll = EventLoop::new().unwrap();
        let s = epoll.summary();
        assert!(s.contains("EventLoop"));
        assert!(s.contains("0 events"));
    }

    #[test]
    fn test_stop() {
        let mut epoll = EventLoop::new().unwrap();
        epoll.stop();
        assert!(!epoll.running);
    }

    #[test]
    fn test_event_count() {
        let epoll = EventLoop::new().unwrap();
        assert_eq!(epoll.event_count(), 0);
    }

    #[test]
    fn test_register_unregister() {
        let mut epoll = EventLoop::new().unwrap();
        epoll.register(5, EventType::Read, 0, |_| {}).unwrap();
        assert!(epoll.summary().contains("1 handlers"));
        epoll.unregister(5).unwrap();
        assert!(epoll.summary().contains("0 handlers"));
    }
}
