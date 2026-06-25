use ferrum_sdk::{Evaluator, Plugin, ProviderId, Score, Transformer};

/// Compiled args for the `clamp` transformer.
pub struct Args {
    pub input: ProviderId,
    /// Input score that maps to `Score(0)`.
    pub low: Score,
    /// Input score that maps to `Score(100)`.
    pub high: Score,
}

/// `clamp` transformer: linearly remaps a sub-range `[low, high]` of the input score to `[0, 100]`.
///
/// Scores at or below `low` become `Score(0)`; at or above `high` become `Score(100)`.
/// Useful when a sensor's meaningful signal occupies only part of the full range.
pub struct ClampTransformer;

impl Plugin for ClampTransformer {}

impl Transformer for ClampTransformer {
    type Args = Args;

    fn compile_args(&self, args: &ferrum_sdk::toml::Value) -> Args {
        let name = args
            .get("input")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("transformer-clamp: missing 'input' string"));

        let parse_bound = |key: &str| -> Score {
            let v = args
                .get(key)
                .and_then(|v| v.as_integer())
                .unwrap_or_else(|| panic!("transformer-clamp: missing '{key}' integer"));
            assert!(
                (0..=100).contains(&v),
                "transformer-clamp: '{key}' must be in [0, 100], got {v}"
            );
            Score::from(v)
        };

        let low = parse_bound("low");
        let high = parse_bound("high");
        assert!(
            low < high,
            "transformer-clamp: 'low' ({}) must be less than 'high' ({})",
            low.0,
            high.0
        );

        Args {
            input: ProviderId::from(name),
            low,
            high,
        }
    }

    fn evaluate(&self, args: &Args, eval: &dyn Evaluator) -> Score {
        let s = eval.score(args.input);
        if s <= args.low {
            return Score(0);
        }
        if s >= args.high {
            return Score(100);
        }
        Score::from((s.0 - args.low.0) as f32 / (args.high.0 - args.low.0) as f32)
    }

    fn dep_ids(args: &Args) -> Vec<ProviderId> {
        vec![args.input]
    }
}

ferrum_sdk::inventory::submit! {
    ferrum_sdk::TransformerFactory {
        name: "clamp",
        build: |args| ClampTransformer.into_entry(args),
    }
}

#[cfg(test)]
mod tests {
    use ferrum_sdk::{Evaluator, ProviderId, Score, Transformer};

    use super::{Args, ClampTransformer};

    struct StubEval(ProviderId, Score);
    impl Evaluator for StubEval {
        fn score(&self, id: ProviderId) -> Score {
            if id == self.0 { self.1 } else { Score(0) }
        }
    }

    fn args(low: u8, high: u8) -> Args {
        Args {
            input: ProviderId(1),
            low: Score(low),
            high: Score(high),
        }
    }

    fn eval(score: u8) -> StubEval {
        StubEval(ProviderId(1), Score(score))
    }

    #[test]
    fn below_low_returns_zero() {
        assert_eq!(
            ClampTransformer.evaluate(&args(20, 80), &eval(10)),
            Score(0)
        );
    }

    #[test]
    fn at_low_returns_zero() {
        assert_eq!(
            ClampTransformer.evaluate(&args(20, 80), &eval(20)),
            Score(0)
        );
    }

    #[test]
    fn above_high_returns_hundred() {
        assert_eq!(
            ClampTransformer.evaluate(&args(20, 80), &eval(90)),
            Score(100)
        );
    }

    #[test]
    fn at_high_returns_hundred() {
        assert_eq!(
            ClampTransformer.evaluate(&args(20, 80), &eval(80)),
            Score(100)
        );
    }

    #[test]
    fn midpoint_returns_fifty() {
        // low=20 high=80 input=50 → (50-20)/(80-20)*100 = 50
        assert_eq!(
            ClampTransformer.evaluate(&args(20, 80), &eval(50)),
            Score(50)
        );
    }

    #[test]
    fn linear_passthrough() {
        // low=0 high=100: identity mapping
        assert_eq!(
            ClampTransformer.evaluate(&args(0, 100), &eval(25)),
            Score(25)
        );
        assert_eq!(
            ClampTransformer.evaluate(&args(0, 100), &eval(75)),
            Score(75)
        );
    }

    #[test]
    fn dep_ids_contains_input() {
        assert_eq!(
            Args {
                input: ProviderId(42),
                low: Score(0),
                high: Score(100),
            }
            .input,
            ProviderId(42)
        );
        let a = args(0, 50);
        assert_eq!(ClampTransformer::dep_ids(&a), vec![ProviderId(1)]);
    }
}
