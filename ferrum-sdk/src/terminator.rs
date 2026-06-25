use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;

use crate::RequestContext;

/// Abstracts the active HTTP session for terminator plugins.
///
/// Implemented by the proxy layer; the SDK has no dependency on any HTTP server crate.
#[async_trait]
pub trait HttpSession: Send {
    /// Send an HTTP response to the client.
    async fn respond(
        &mut self,
        status: u16,
        headers: Vec<(String, String)>,
        body: Bytes,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// Drop the TCP connection without sending a response.
    async fn close(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

/// Trait for all terminator plugins.
///
/// Implement this to create a custom terminator.  Register instances in the
/// TOML config under `[terminators.*]` and reference them from rules via
/// `if_action = "<name>"` or `else_action = "<name>"`.
#[async_trait]
pub trait Terminator: crate::Plugin {
    /// Called when the rule threshold is met.
    ///
    /// Return `true` if the request was handled (response written or connection dropped).
    /// Return `false` to pass the request to the upstream.
    async fn execute(&self, ctx: &RequestContext, session: &mut dyn HttpSession) -> bool;
}

/// Registration entry for a terminator plugin.
///
/// Pass to [`ferrum_sdk::register`] inside your plugin's `init()` function.
#[derive(Clone)]
pub struct TerminatorFactory {
    /// Plugin name as it appears in `ferrum.toml` under `plugin = "..."`.
    pub name: &'static str,
    /// Builds a [`Terminator`] instance from TOML args.
    pub build: fn(args: &crate::toml::Value) -> Arc<dyn Terminator>,
}
