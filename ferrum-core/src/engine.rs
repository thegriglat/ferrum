use rustc_hash::FxHashMap;

use ferrum_sdk::{Evaluator, ProviderId, RequestContext, Score};

use crate::registry::PluginRegistry;

/// Evaluator backed by an immutable snapshot of the score cache.
///
/// Created transiently by the engine after pre-populating transformer deps;
/// passed to transformer plugins so they never touch [`RequestContext`] directly.
struct CacheEvaluator<'a>(&'a FxHashMap<ProviderId, Score>);

impl Evaluator for CacheEvaluator<'_> {
    fn score(&self, id: ProviderId) -> Score {
        self.0.get(&id).copied().unwrap_or(Score(0))
    }
}

/// Evaluates provider `id` against `ctx` using `registry`.
///
/// Scores are in `[0, 100]`.  Results are memoised in `ctx.cache`
/// so each provider is called at most once per request.
///
/// Sensors read from `ctx` directly; transformers receive a read-only
/// [`CacheEvaluator`] after all declared deps have been pre-populated.
pub fn evaluate(id: ProviderId, ctx: &mut RequestContext, registry: &PluginRegistry) -> Score {
    if let Some(&cached) = ctx.cache.get(&id) {
        return cached;
    }
    let deps = registry.dep_ids(id);
    let score = if deps.is_empty() {
        registry.evaluate_sensor(id, ctx)
    } else {
        for dep in deps {
            evaluate(dep, ctx, registry);
        }
        let eval = CacheEvaluator(&ctx.cache);
        registry.evaluate_transformer(id, &eval)
    };
    ctx.cache.insert(id, score);
    score
}
