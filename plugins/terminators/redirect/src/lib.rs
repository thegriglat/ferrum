use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use ferrum_sdk::{HttpSession, Plugin, RequestContext, Terminator};

struct RedirectTerminator {
    target_url: String,
    status_code: u16,
}

impl Plugin for RedirectTerminator {}

#[async_trait]
impl Terminator for RedirectTerminator {
    async fn execute(&self, _ctx: &RequestContext, session: &mut dyn HttpSession) -> bool {
        let headers = vec![("location".to_string(), self.target_url.clone())];
        let _ = session
            .respond(self.status_code, headers, Bytes::new())
            .await;
        true
    }
}

ferrum_sdk::inventory::submit! {
    ferrum_sdk::TerminatorFactory {
        name: "redirect",
        build: |args| {
            let target_url = args
                .get("target_url")
                .and_then(|v| v.as_str())
                .unwrap_or("/")
                .to_string();
            let status_code = args
                .get("status_code")
                .and_then(|v| v.as_integer())
                .unwrap_or(302) as u16;
            Arc::new(RedirectTerminator {
                target_url,
                status_code,
            })
        },
    }
}
