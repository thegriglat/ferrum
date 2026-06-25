use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use ferrum_sdk::{HttpSession, Plugin, RequestContext, Terminator};

struct TimeoutTerminator {
    delay_ms: u64,
}

impl Plugin for TimeoutTerminator {}

#[async_trait]
impl Terminator for TimeoutTerminator {
    async fn execute(&self, _ctx: &RequestContext, session: &mut dyn HttpSession) -> bool {
        tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
        let _ = session.close().await;
        true
    }
}

ferrum_sdk::inventory::submit! {
    ferrum_sdk::TerminatorFactory {
        name: "timeout",
        build: |args| {
            let delay_ms = args
                .get("delay_ms")
                .and_then(|v| v.as_integer())
                .unwrap_or(5000) as u64;
            Arc::new(TimeoutTerminator { delay_ms })
        },
    }
}
