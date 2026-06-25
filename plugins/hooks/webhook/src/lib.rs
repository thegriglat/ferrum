use std::sync::mpsc::SyncSender;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ferrum_sdk::{Hook, Plugin, RequestContext};
use serde::Serialize;

ferrum_sdk::inventory::submit! {
    ferrum_sdk::HookFactory {
        name: "hook-webhook",
        build: |args| WebhookHook.into_entry(args),
    }
}

struct WebhookHook;

impl Plugin for WebhookHook {}

#[derive(Serialize)]
struct WebhookEvent {
    method: String,
    uri: String,
    client_ip: String,
    user_agent: String,
    timestamp: u64,
}

/// Compiled args for `hook-webhook`.
pub struct WebhookArgs {
    sender: SyncSender<WebhookEvent>,
}

impl Hook for WebhookHook {
    type Args = WebhookArgs;

    fn compile_args(&self, args: &ferrum_sdk::toml::Value) -> Self::Args {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .expect("hook-webhook: 'url' is required")
            .to_string();

        let timeout_ms = args
            .get("timeout_ms")
            .and_then(|v| v.as_integer())
            .unwrap_or(3000) as u64;

        let extra_headers: Vec<(String, String)> = args
            .get("headers")
            .and_then(|v| v.as_table())
            .map(|t| {
                t.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let (tx, rx) = std::sync::mpsc::sync_channel::<WebhookEvent>(512);

        std::thread::spawn(move || {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_millis(timeout_ms))
                .build()
                .expect("hook-webhook: failed to build HTTP client");

            for event in rx {
                let mut req = client.post(&url).json(&event);
                for (k, v) in &extra_headers {
                    req = req.header(k.as_str(), v.as_str());
                }
                if let Err(e) = req.send() {
                    tracing::warn!(error = %e, url = %url, "hook-webhook: delivery failed");
                }
            }
        });

        WebhookArgs { sender: tx }
    }

    /// Non-blocking: clones relevant fields and sends the event to the background thread.
    fn trigger(&self, ctx: &RequestContext, args: &Self::Args) {
        let event = WebhookEvent {
            method: ctx.method.clone(),
            uri: ctx.uri.clone(),
            client_ip: ctx.client_ip.to_string(),
            user_agent: ctx.headers.get("user-agent").cloned().unwrap_or_default(),
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        };
        let _ = args.sender.try_send(event);
    }
}
