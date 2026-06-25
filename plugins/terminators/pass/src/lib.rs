use std::sync::Arc;

use async_trait::async_trait;
use ferrum_sdk::{HttpSession, Plugin, RequestContext, Terminator};

ferrum_sdk::inventory::submit! {
    ferrum_sdk::TerminatorFactory {
        name: "pass",
        build: |_| Arc::new(BypassTerminator),
    }
}

ferrum_sdk::inventory::submit! {
    ferrum_sdk::TerminatorFactory {
        name: "bypass",
        build: |_| Arc::new(BypassTerminator),
    }
}

struct BypassTerminator;

impl Plugin for BypassTerminator {}

#[async_trait]
impl Terminator for BypassTerminator {
    async fn execute(&self, _ctx: &RequestContext, _session: &mut dyn HttpSession) -> bool {
        false
    }
}
