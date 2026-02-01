//! Actor mailbox implementation
//!
//! TigerStyle: Bounded queues with explicit limits, no silent drops.

use std::collections::VecDeque;

use bytes::Bytes;
use tokio::sync::oneshot;

use kelpie_core::constants::MAILBOX_DEPTH_MAX;
use kelpie_core::io::{TimeProvider, WallClockTime};

/// Error when mailbox is full
#[derive(Debug, Clone)]
pub struct MailboxFullError {
    pub mailbox_depth: usize,
    pub limit: usize,
}

impl std::fmt::Display for MailboxFullError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "mailbox full: {} messages (limit: {})",
            self.mailbox_depth, self.limit
        )
    }
}

impl std::error::Error for MailboxFullError {}

/// A message in the mailbox
#[derive(Debug)]
pub struct Envelope {
    /// The operation name
    pub operation: String,
    /// The message payload
    pub payload: Bytes,
    /// Channel to send the response
    pub reply_tx: oneshot::Sender<Result<Bytes, kelpie_core::Error>>,
    /// When the message was enqueued (monotonic timestamp in ms)
    pub enqueued_at_ms: u64,
}

impl Envelope {
    /// Create a new envelope using production wall clock
    ///
    /// For DST, use `new_with_time`.
    pub fn new(
        operation: String,
        payload: Bytes,
        reply_tx: oneshot::Sender<Result<Bytes, kelpie_core::Error>>,
    ) -> Self {
        Self::new_with_time(operation, payload, reply_tx, &WallClockTime::new())
    }

    /// Create a new envelope with injected time provider (for DST)
    pub fn new_with_time(
        operation: String,
        payload: Bytes,
        reply_tx: oneshot::Sender<Result<Bytes, kelpie_core::Error>>,
        time: &dyn TimeProvider,
    ) -> Self {
        debug_assert!(!operation.is_empty(), "operation must not be empty");

        Self {
            operation,
            payload,
            reply_tx,
            enqueued_at_ms: time.monotonic_ms(),
        }
    }

    /// Get the time this message has been waiting in milliseconds
    ///
    /// For DST, use `wait_time_ms_with_time`.
    pub fn wait_time_ms(&self) -> u64 {
        self.wait_time_ms_with_time(&WallClockTime::new())
    }

    /// Get the time this message has been waiting in milliseconds with injected time (for DST)
    pub fn wait_time_ms_with_time(&self, time: &dyn TimeProvider) -> u64 {
        time.monotonic_ms().saturating_sub(self.enqueued_at_ms)
    }
}

/// Bounded mailbox for actor messages
///
/// # TigerStyle
/// - Explicit capacity limit
/// - FIFO ordering
/// - No silent drops (returns error when full)
#[derive(Debug)]
pub struct Mailbox {
    /// Pending messages
    queue: VecDeque<Envelope>,
    /// Maximum number of messages
    capacity: usize,
    /// Total messages enqueued (for metrics)
    enqueued_count: u64,
    /// Total messages processed (for metrics)
    processed_count: u64,
}

impl Mailbox {
    /// Create a new mailbox with default capacity
    pub fn new() -> Self {
        Self::with_capacity(MAILBOX_DEPTH_MAX)
    }

    /// Create a new mailbox with specified capacity
    pub fn with_capacity(capacity: usize) -> Self {
        debug_assert!(capacity > 0, "capacity must be positive");
        debug_assert!(
            capacity <= MAILBOX_DEPTH_MAX,
            "capacity exceeds MAILBOX_DEPTH_MAX"
        );

        Self {
            queue: VecDeque::with_capacity(capacity.min(1024)), // Pre-allocate reasonably
            capacity,
            enqueued_count: 0,
            processed_count: 0,
        }
    }

    /// Try to enqueue a message
    ///
    /// Returns error if mailbox is full.
    pub fn push(&mut self, envelope: Envelope) -> Result<(), MailboxFullError> {
        if self.queue.len() >= self.capacity {
            return Err(MailboxFullError {
                mailbox_depth: self.queue.len(),
                limit: self.capacity,
            });
        }

        self.queue.push_back(envelope);
        self.enqueued_count = self.enqueued_count.wrapping_add(1);

        debug_assert!(self.queue.len() <= self.capacity);
        Ok(())
    }

    /// Pop the next message from the mailbox
    pub fn pop(&mut self) -> Option<Envelope> {
        let envelope = self.queue.pop_front();
        if envelope.is_some() {
            self.processed_count = self.processed_count.wrapping_add(1);
        }
        envelope
    }

    /// Check if the mailbox is empty
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Get the number of pending messages
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Get the mailbox capacity
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Get total messages enqueued
    pub fn enqueued_count(&self) -> u64 {
        self.enqueued_count
    }

    /// Get total messages processed
    pub fn processed_count(&self) -> u64 {
        self.processed_count
    }

    /// Drain all pending messages
    ///
    /// Used during deactivation to reject pending messages.
    pub fn drain(&mut self) -> Vec<Envelope> {
        self.queue.drain(..).collect()
    }
}

impl Default for Mailbox {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    fn create_envelope(operation: &str) -> Envelope {
        let (tx, _rx) = oneshot::channel();
        Envelope::new(operation.to_string(), Bytes::new(), tx)
    }

    #[test]
    fn test_mailbox_push_pop() {
        let mut mailbox = Mailbox::with_capacity(10);

        mailbox.push(create_envelope("op1")).unwrap();
        mailbox.push(create_envelope("op2")).unwrap();

        assert_eq!(mailbox.len(), 2);
        assert!(!mailbox.is_empty());

        let env1 = mailbox.pop().unwrap();
        assert_eq!(env1.operation, "op1");

        let env2 = mailbox.pop().unwrap();
        assert_eq!(env2.operation, "op2");

        assert!(mailbox.is_empty());
        assert!(mailbox.pop().is_none());
    }

    #[test]
    fn test_mailbox_full() {
        let mut mailbox = Mailbox::with_capacity(2);

        mailbox.push(create_envelope("op1")).unwrap();
        mailbox.push(create_envelope("op2")).unwrap();

        let result = mailbox.push(create_envelope("op3"));
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert_eq!(err.mailbox_depth, 2);
        assert_eq!(err.limit, 2);
    }

    #[test]
    fn test_mailbox_fifo_order() {
        let mut mailbox = Mailbox::with_capacity(100);

        for i in 0..10 {
            mailbox.push(create_envelope(&format!("op{}", i))).unwrap();
        }

        for i in 0..10 {
            let env = mailbox.pop().unwrap();
            assert_eq!(env.operation, format!("op{}", i));
        }
    }

    #[test]
    fn test_mailbox_metrics() {
        let mut mailbox = Mailbox::with_capacity(10);

        assert_eq!(mailbox.enqueued_count(), 0);
        assert_eq!(mailbox.processed_count(), 0);

        mailbox.push(create_envelope("op1")).unwrap();
        mailbox.push(create_envelope("op2")).unwrap();

        assert_eq!(mailbox.enqueued_count(), 2);
        assert_eq!(mailbox.processed_count(), 0);

        mailbox.pop();
        assert_eq!(mailbox.processed_count(), 1);

        mailbox.pop();
        assert_eq!(mailbox.processed_count(), 2);
    }

    #[test]
    fn test_mailbox_drain() {
        let mut mailbox = Mailbox::with_capacity(10);

        mailbox.push(create_envelope("op1")).unwrap();
        mailbox.push(create_envelope("op2")).unwrap();
        mailbox.push(create_envelope("op3")).unwrap();

        let drained = mailbox.drain();
        assert_eq!(drained.len(), 3);
        assert!(mailbox.is_empty());
    }
}
