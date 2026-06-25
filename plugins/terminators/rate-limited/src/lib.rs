use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use ferrum_sdk::{HttpSession, Plugin, RequestContext, Terminator};

struct RateLimitedTerminator {
    retry_after: u32,
}

impl Plugin for RateLimitedTerminator {}

#[async_trait]
impl Terminator for RateLimitedTerminator {
    async fn execute(&self, _ctx: &RequestContext, session: &mut dyn HttpSession) -> bool {
        let headers = vec![("retry-after".to_string(), self.retry_after.to_string())];
        let _ = session.respond(429, headers, Bytes::new()).await;
        true
    }
}

ferrum_sdk::inventory::submit! {
    ferrum_sdk::TerminatorFactory {
        name: "rate_limited",
        build: |args| {
            let retry_after = args
                .get("retry_after")
                .and_then(|v| v.as_integer())
                .unwrap_or(60) as u32;
            Arc::new(RateLimitedTerminator { retry_after })
        },
    }
}
