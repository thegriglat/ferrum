use ferrum_sdk::{Evaluator, Plugin, ProviderId, Score, Transformer};

/// Transformer that computes a weighted average of sensor scores.
///
/// Each input is a `(ProviderId, weight)` pair.  The result is
/// `Σ(score_i × weight_i) / Σ(weight_i)`, clamped to `[0, 100]`.
/// Weights express relative importance; their absolute scale does not
/// affect the output.
///
/// # TOML config
///
/// ```toml
/// [transformers.combined_threat]
/// plugin = "weighted_sum"
///
/// [transformers.combined_threat.inputs]
/// sqli_uri  = 0.6
/// sqli_body = 1.0
/// ```
pub struct WeightedSumTransformer;

/// Compiled arguments for [`WeightedSumTransformer`].
pub struct WeightedSumArgs {
    /// `(provider_id, weight)` pairs compiled at startup from the `inputs` table.
    pub inputs: Vec<(ProviderId, f32)>,
}

impl Plugin for WeightedSumTransformer {}

impl Transformer for WeightedSumTransformer {
    type Args = WeightedSumArgs;

    fn compile_args(&self, args: &ferrum_sdk::toml::Value) -> WeightedSumArgs {
        let inputs_table = args
            .get("inputs")
            .and_then(|v| v.as_table())
            .expect("transformer-weighted-sum: missing 'inputs' table");

        let inputs = inputs_table
            .iter()
            .map(|(name, weight_val)| {
                let weight = weight_val.as_float().unwrap_or_else(|| {
                    weight_val
                        .as_integer()
                        .map(|i| i as f64)
                        .unwrap_or_else(|| {
                            panic!("transformer-weighted-sum: weight for '{name}' must be a number")
                        })
                }) as f32;
                (ProviderId::from(name.as_str()), weight)
            })
            .collect();

        WeightedSumArgs { inputs }
    }

    fn evaluate(&self, args: &WeightedSumArgs, eval: &dyn Evaluator) -> Score {
        let total_weight: f32 = args.inputs.iter().map(|(_, w)| *w).sum();
        if total_weight == 0.0 {
            return Score(0);
        }
        let raw: f32 = args
            .inputs
            .iter()
            .map(|(id, weight)| f32::from(eval.score(*id)) * weight)
            .sum::<f32>()
            / total_weight;
        Score::from(raw / 100.0)
    }

    fn dep_ids(args: &WeightedSumArgs) -> Vec<ProviderId> {
        args.inputs.iter().map(|(id, _)| *id).collect()
    }
}

ferrum_sdk::inventory::submit! {
    ferrum_sdk::TransformerFactory {
        name: "weighted_sum",
        build: |args| WeightedSumTransformer.into_entry(args),
    }
}

#[cfg(test)]
mod tests {
    use ferrum_sdk::{Evaluator, ProviderId, Score, Transformer};

    use super::{WeightedSumArgs, WeightedSumTransformer};

    struct StubEvaluator(Vec<(ProviderId, Score)>);

    impl Evaluator for StubEvaluator {
        fn score(&self, id: ProviderId) -> Score {
            self.0
                .iter()
                .find(|(pid, _)| *pid == id)
                .map(|(_, s)| *s)
                .unwrap_or(Score(0))
        }
    }

    fn pid(n: u64) -> ProviderId {
        ProviderId(n)
    }

    fn args(pairs: &[(u64, f32)]) -> WeightedSumArgs {
        WeightedSumArgs {
            inputs: pairs.iter().map(|(n, w)| (pid(*n), *w)).collect(),
        }
    }

    #[test]
    fn weighted_average_basic() {
        let t = WeightedSumTransformer;
        let a = args(&[(1, 0.5), (2, 0.6)]);
        let eval = StubEvaluator(vec![(pid(1), Score(80)), (pid(2), Score(50))]);
        // (80*0.5 + 50*0.6) / (0.5+0.6) = 70 / 1.1 ≈ 63.6 → rounds to 64
        let score = t.evaluate(&a, &eval);
        assert_eq!(score, Score(64), "expected 64, got {score}");
    }

    #[test]
    fn uniform_scores_invariant() {
        let t = WeightedSumTransformer;
        // With equal scores the result is independent of the weights
        let a1 = args(&[(1, 0.5), (2, 0.6)]);
        let a2 = args(&[(1, 5.0), (2, 6.0)]);
        let eval1 = StubEvaluator(vec![(pid(1), Score(80)), (pid(2), Score(80))]);
        let eval2 = StubEvaluator(vec![(pid(1), Score(80)), (pid(2), Score(80))]);
        assert_eq!(t.evaluate(&a1, &eval1), t.evaluate(&a2, &eval2));
    }

    #[test]
    fn full_score_stays_full() {
        let t = WeightedSumTransformer;
        let a = args(&[(1, 1.0), (2, 1.0)]);
        let eval = StubEvaluator(vec![(pid(1), Score(100)), (pid(2), Score(100))]);
        assert_eq!(t.evaluate(&a, &eval), Score(100));
    }

    #[test]
    fn missing_dep_scores_zero() {
        let t = WeightedSumTransformer;
        let a = args(&[(1, 0.5), (99, 0.5)]);
        let eval = StubEvaluator(vec![(pid(1), Score(100))]);
        // 100 * 0.5 + 0 * 0.5 = 50
        let score = t.evaluate(&a, &eval);
        assert_eq!(score, Score(50), "expected 50, got {score}");
    }

    #[test]
    fn dep_ids_returns_all_inputs() {
        let a = args(&[(1, 0.3), (2, 0.7), (3, 1.0)]);
        assert_eq!(WeightedSumTransformer::dep_ids(&a).len(), 3);
    }
}
