use std::sync::Arc;

use async_trait::async_trait;
use ferrum_sdk::{HttpSession, Plugin, RequestContext, Terminator};

struct DropTerminator;

impl Plugin for DropTerminator {}

#[async_trait]
impl Terminator for DropTerminator {
    async fn execute(&self, _ctx: &RequestContext, session: &mut dyn HttpSession) -> bool {
        let _ = session.close().await;
        true
    }
}

ferrum_sdk::inventory::submit! {
    ferrum_sdk::TerminatorFactory {
        name: "drop",
        build: |_args| Arc::new(DropTerminator),
    }
}
