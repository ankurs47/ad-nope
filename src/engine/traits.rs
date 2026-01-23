use std::sync::Arc;

/// The "Hot Path" engine for checking domains.
pub trait BlocklistMatcher: Send + Sync {
    /// Returns Some(source_id) if blocked, None if allowed.
    fn check(&self, domain: &str) -> Option<u8>;
}

/// The "Control Plane" for updates.
#[async_trait::async_trait]
pub trait BlocklistManager: Send + Sync {
    /// Fetches all configured lists and builds a new Matcher.
    async fn refresh(&self) -> Arc<dyn BlocklistMatcher>;
}
