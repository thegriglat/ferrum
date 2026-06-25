use crate::{Plugin, RequestContext};

/// Compiled hook entry stored in the registry.
///
/// The closure fires after the final rule verdict for every request.
/// It must be non-blocking — delegate I/O to a background tokio worker.
pub struct HookEntry {
    pub trigger: Box<dyn Fn(&RequestContext) + Send + Sync>,
}

/// Typed trait for hook plugin authors.
///
/// Hooks receive a read-only view of the request context and must not block
/// the Pingora event loop.  The canonical pattern: send an event to a
/// [`tokio::sync::mpsc`] channel; actual network I/O runs in a background
/// worker started inside [`compile_args`].
///
/// [`compile_args`]: Hook::compile_args
pub trait Hook: Plugin {
    /// Compiled, request-independent configuration for this hook instance.
    type Args: Send + Sync + 'static;

    /// Parse `args` from the TOML hook block and return compiled `Args`.
    ///
    /// Called once at startup.  Use this to open connections, spawn background
    /// workers, and create channels.  Panics are acceptable here.
    fn compile_args(&self, args: &crate::toml::Value) -> Self::Args;

    /// Called after the final rule verdict.  Must be non-blocking.
    fn trigger(&self, ctx: &RequestContext, args: &Self::Args);

    /// Convenience: compiles args and wraps `self` in a [`HookEntry`].
    fn into_entry(self, args: &crate::toml::Value) -> HookEntry
    where
        Self: Sized + 'static,
    {
        let compiled = self.compile_args(args);
        HookEntry {
            trigger: Box::new(move |ctx| self.trigger(ctx, &compiled)),
        }
    }
}

/// Registration entry for a hook plugin.
///
/// Pass to [`ferrum_sdk::register`] inside your plugin's `init()` function.
///
/// # Example
///
/// ```ignore
/// pub fn init() {
///     ferrum_sdk::register(HookFactory {
///         name: "my-hook",
///         build: |args| MyHook.into_entry(args),
///     });
/// }
/// ```
#[derive(Clone, Copy)]
pub struct HookFactory {
    /// Plugin name as it appears in `ferrum.toml` under `plugin = "..."`.
    pub name: &'static str,
    /// Builds a [`HookEntry`] from TOML args.
    pub build: fn(args: &crate::toml::Value) -> HookEntry,
}
