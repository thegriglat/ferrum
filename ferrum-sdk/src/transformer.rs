use crate::{Plugin, ProviderId, Score};

/// Read-only view of the per-request score cache.
///
/// Passed to transformers so they can query pre-computed provider scores
/// without direct access to [`RequestContext`].
pub trait Evaluator: Send + Sync {
    /// Returns the cached score for `id`, or `Score(0)` if not yet evaluated.
    fn score(&self, id: ProviderId) -> Score;
}

/// Type alias for a compiled transformer evaluation closure.
pub type EvaluateFn = Box<dyn Fn(&dyn Evaluator) -> Score + Send + Sync>;

/// Compiled transformer entry stored in the registry.
///
/// Bundles the provider, its pre-compiled args, and the declared dependency list
/// in a single struct so the registry needs no type parameters or `Any` downcasting.
pub struct TransformerEntry {
    /// Provider IDs that must be evaluated before this transformer is called.
    pub dep_ids: Vec<ProviderId>,
    pub evaluate: EvaluateFn,
}

/// Typed trait for transformer plugin authors.
///
/// Transformers are blind to the HTTP request — they consume only scores
/// produced by sensors or other transformers.  `Args` is compiled once at
/// startup and reused for every request.
pub trait Transformer: Plugin {
    /// Compiled, request-independent configuration for this transformer instance.
    type Args: Send + Sync + 'static;

    /// Parse `args` from the TOML transformer block and return compiled `Args`.
    ///
    /// Called once at startup.  Panics are acceptable here (misconfiguration).
    fn compile_args(&self, args: &crate::toml::Value) -> Self::Args;

    /// Evaluate this transformer given pre-computed dependency scores via `eval`.
    ///
    /// Returns a [`Score`] in `[0, 100]`.  Must be deterministic and free of side effects.
    fn evaluate(&self, args: &Self::Args, eval: &dyn Evaluator) -> Score;

    /// Returns the [`ProviderId`]s of all providers this transformer depends on.
    ///
    /// The engine evaluates all declared deps and populates the cache before
    /// calling [`evaluate`], so [`Evaluator::score`] always returns a real value.
    fn dep_ids(args: &Self::Args) -> Vec<ProviderId>;

    /// Convenience: compiles args and wraps `self` in a [`TransformerEntry`].
    fn into_entry(self, args: &crate::toml::Value) -> TransformerEntry
    where
        Self: Sized + 'static,
    {
        let compiled = self.compile_args(args);
        let dep_ids = Self::dep_ids(&compiled);
        TransformerEntry {
            dep_ids,
            evaluate: Box::new(move |eval| self.evaluate(&compiled, eval)),
        }
    }
}

/// Registration entry for a transformer plugin.
///
/// Pass to [`ferrum_sdk::register`] inside your plugin's `init()` function.
///
/// # Example
///
/// ```ignore
/// pub fn init() {
///     ferrum_sdk::register(TransformerFactory {
///         name: "my-transformer",
///         build: |args| MyTransformer.into_entry(args),
///     });
/// }
/// ```
#[derive(Copy, Clone)]
pub struct TransformerFactory {
    /// Plugin name as it appears in `ferrum.toml` under `plugin = "..."`.
    pub name: &'static str,
    /// Builds a [`TransformerEntry`] from TOML args.
    pub build: fn(args: &crate::toml::Value) -> TransformerEntry,
}
