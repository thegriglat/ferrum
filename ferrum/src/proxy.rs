use std::net::IpAddr;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use pingora_core::Result;
use pingora_core::upstreams::peer::HttpPeer;
use pingora_http::{ResponseHeader, StatusCode};
use pingora_proxy::{ProxyHttp, Session};
use rustc_hash::FxHashMap;
use tracing::{info, warn};

use ferrum_core::config::{CompiledHook, CompiledRule};
use ferrum_core::engine;
use ferrum_sdk::{HttpSession, ProviderId, RequestContext, Score};

use ferrum_core::config::CompiledAction;

use crate::loader::FerrumConfig;
use crate::metrics;

const BODY_LIMIT: usize = 256 * 1024;

/// Per-request context threaded through Pingora callbacks.
pub struct ReqCtx {
    ferrum_ctx: RequestContext,
    body_buf: Vec<u8>,
    needs_body: bool,
    blocked: bool,
}

/// Ferrum reverse proxy.
pub struct FerrumProxy {
    config: Arc<FerrumConfig>,
    upstream: String,
}

impl FerrumProxy {
    /// Creates a new proxy backed by `config`.
    pub fn new(config: Arc<FerrumConfig>) -> Self {
        let upstream = config.server.upstream.clone();
        Self { config, upstream }
    }
}

/// Wraps a Pingora [`Session`] to implement [`HttpSession`] for terminator plugins.
struct PingoraSession<'a> {
    inner: &'a mut Session,
    blocked_status: Option<u16>,
}

#[async_trait]
impl HttpSession for PingoraSession<'_> {
    async fn respond(
        &mut self,
        status: u16,
        headers: Vec<(String, String)>,
        body: Bytes,
    ) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let status_code = StatusCode::from_u16(status)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        let mut resp = ResponseHeader::build(status_code, Some(headers.len()))
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        for (k, v) in headers {
            resp.insert_header(k, v)
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        }
        let end_of_stream = body.is_empty();
        self.inner
            .write_response_header(Box::new(resp), end_of_stream)
            .await?;
        if !end_of_stream {
            self.inner.write_response_body(Some(body), true).await?;
        }
        self.blocked_status = Some(status);
        Ok(())
    }

    async fn close(&mut self) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.inner.set_keepalive(None);
        self.blocked_status = Some(0);
        Ok(())
    }
}

/// Walks the action tree starting from `action`.
///
/// Returns `true` if the request was handled by a terminator.
/// Hook nodes fire their trigger (non-blocking) and continue to `next`.
/// Uses `Box::pin` to support mutual recursion through async rule trees.
fn eval_action<'a>(
    action: &'a CompiledAction,
    rules: &'a FxHashMap<ProviderId, CompiledRule>,
    hooks: &'a FxHashMap<ProviderId, CompiledHook>,
    ctx: &'a mut RequestContext,
    session: &'a mut dyn HttpSession,
    registry: &'a ferrum_core::registry::PluginRegistry,
) -> Pin<Box<dyn std::future::Future<Output = bool> + Send + 'a>> {
    Box::pin(async move {
        match action {
            CompiledAction::Pass => false,
            CompiledAction::Terminator(id) => {
                let t = registry
                    .get_terminator(*id)
                    .unwrap_or_else(|| panic!("proxy: unknown terminator id {id:?}"));
                t.execute(ctx, session).await
            }
            CompiledAction::Hook(id) => {
                registry.trigger_hook(*id, ctx);
                let hook = hooks
                    .get(id)
                    .unwrap_or_else(|| panic!("proxy: unknown hook id {id:?}"));
                eval_action(&hook.next, rules, hooks, ctx, session, registry).await
            }
            CompiledAction::Rule(id) => {
                let rule = rules
                    .get(id)
                    .unwrap_or_else(|| panic!("proxy: unknown rule id {id:?}"));
                let score = engine::evaluate(rule.input, ctx, registry);
                ctx.current_score = score;
                let (next_action, branch) = if score >= rule.threshold {
                    (&rule.if_action, "if")
                } else {
                    (&rule.else_action, "else")
                };
                info!(
                    rule_id = ?rule.id,
                    client_ip = %ctx.client_ip,
                    uri = %ctx.uri,
                    method = %ctx.method,
                    score = %score,
                    threshold = %rule.threshold,
                    branch,
                    "waf rule evaluated"
                );
                metrics::record_decision(branch, &format!("{:?}", rule.id), score);
                eval_action(next_action, rules, hooks, ctx, session, registry).await
            }
        }
    })
}

#[async_trait]
impl ProxyHttp for FerrumProxy {
    type CTX = ReqCtx;

    fn new_ctx(&self) -> ReqCtx {
        let needs_body = self.config.rules.values().any(|r| r.buffer_body);
        ReqCtx {
            ferrum_ctx: RequestContext::new(
                "0.0.0.0".parse().unwrap(),
                String::new(),
                String::new(),
            ),
            body_buf: Vec::new(),
            needs_body,
            blocked: false,
        }
    }

    async fn request_filter(&self, session: &mut Session, ctx: &mut ReqCtx) -> Result<bool> {
        let req = session.req_header();

        let ip: IpAddr = session
            .client_addr()
            .and_then(|a| a.as_inet())
            .map(|a| a.ip())
            .unwrap_or(IpAddr::from([0u8, 0, 0, 0]));

        let uri = req.uri.to_string();
        let method = req.method.to_string();

        let mut headers = FxHashMap::default();
        for (k, v) in req.headers.iter() {
            if let Ok(v) = v.to_str() {
                headers.insert(k.as_str().to_lowercase(), v.to_string());
            }
        }

        ctx.ferrum_ctx = RequestContext {
            client_ip: ip,
            uri,
            method,
            headers,
            body: None,
            cache: FxHashMap::default(),
            metadata: FxHashMap::default(),
            current_score: Score(0),
        };

        if ctx.needs_body {
            return Ok(false);
        }

        let mut ps = PingoraSession {
            inner: session,
            blocked_status: None,
        };
        let entry_action = self.config.entry.clone();
        let handled = eval_action(
            &entry_action,
            &self.config.rules,
            &self.config.hooks,
            &mut ctx.ferrum_ctx,
            &mut ps,
            &self.config.registry,
        )
        .await;

        if handled {
            ctx.blocked = true;
            return Ok(true);
        }

        Ok(false)
    }

    async fn request_body_filter(
        &self,
        session: &mut Session,
        body: &mut Option<Bytes>,
        end_of_stream: bool,
        ctx: &mut ReqCtx,
    ) -> Result<()> {
        if !ctx.needs_body {
            return Ok(());
        }

        if let Some(chunk) = body.take() {
            if ctx.body_buf.len() + chunk.len() <= BODY_LIMIT {
                ctx.body_buf.extend_from_slice(&chunk);
            } else {
                warn!(
                    client_ip = %ctx.ferrum_ctx.client_ip,
                    uri = %ctx.ferrum_ctx.uri,
                    "request body exceeded 256 KiB limit — body sensor disabled for this request"
                );
                ctx.ferrum_ctx.body = None;
                ctx.body_buf.clear();
            }
        }

        if end_of_stream {
            if !ctx.body_buf.is_empty() {
                let body_bytes = Bytes::copy_from_slice(&ctx.body_buf);
                metrics::record_body_size(body_bytes.len());
                ctx.ferrum_ctx.body = Some(body_bytes);
            }

            let mut ps = PingoraSession {
                inner: session,
                blocked_status: None,
            };
            let entry_action = self.config.entry.clone();
            let handled = eval_action(
                &entry_action,
                &self.config.rules,
                &self.config.hooks,
                &mut ctx.ferrum_ctx,
                &mut ps,
                &self.config.registry,
            )
            .await;

            if handled {
                ctx.blocked = true;
            }
        }

        Ok(())
    }

    async fn logging(
        &self,
        _session: &mut Session,
        _e: Option<&pingora_core::Error>,
        ctx: &mut ReqCtx,
    ) {
        if !ctx.blocked {
            info!(
                client_ip = %ctx.ferrum_ctx.client_ip,
                uri = %ctx.ferrum_ctx.uri,
                method = %ctx.ferrum_ctx.method,
                action = "pass",
                status = 200u16,
                "waf decision"
            );
            metrics::record_decision("pass", "none", Score(0));
        }
    }

    async fn upstream_peer(
        &self,
        _session: &mut Session,
        _ctx: &mut ReqCtx,
    ) -> Result<Box<HttpPeer>> {
        let peer = HttpPeer::new(self.upstream.as_str(), false, String::new());
        Ok(Box::new(peer))
    }
}
