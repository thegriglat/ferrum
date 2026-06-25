use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use ferrum_sdk::{HttpSession, Plugin, RequestContext, Terminator};

ferrum_sdk::inventory::submit! {
    ferrum_sdk::TerminatorFactory {
        name: "block",
        build: |args| {
            let status = args
                .get("status")
                .and_then(|v| v.as_integer())
                .unwrap_or(403) as u16;
            Arc::new(BlockTerminator { status })
        },
    }
}

struct BlockTerminator {
    status: u16,
}

impl Plugin for BlockTerminator {}

#[async_trait]
impl Terminator for BlockTerminator {
    async fn execute(&self, _ctx: &RequestContext, session: &mut dyn HttpSession) -> bool {
        let _ = session.respond(self.status, vec![], Bytes::new()).await;
        true
    }
}
