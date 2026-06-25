use ferrum_sdk::{Evaluator, Plugin, ProviderId, Score, Transformer};

/// Compiled args for the `or` transformer.
pub struct Args {
    pub inputs: Vec<ProviderId>,
}

/// `or` transformer: fuzzy OR — `max(inputs)`.
pub struct OrTransformer;

impl Plugin for OrTransformer {}

impl Transformer for OrTransformer {
    type Args = Args;

    fn compile_args(&self, args: &ferrum_sdk::toml::Value) -> Args {
        let arr = args
            .get("inputs")
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| {
                panic!("transformer-or: 'inputs' must be an array of provider names")
            });
        let inputs = arr
            .iter()
            .map(|v| {
                let name = v.as_str().unwrap_or_else(|| {
                    panic!("transformer-or: each entry in 'inputs' must be a string")
                });
                ProviderId::from(name)
            })
            .collect();
        Args { inputs }
    }

    fn evaluate(&self, args: &Args, eval: &dyn Evaluator) -> Score {
        args.inputs
            .iter()
            .map(|id| eval.score(*id))
            .fold(Score(0), |a, b| a.or(b))
    }

    fn dep_ids(args: &Args) -> Vec<ProviderId> {
        args.inputs.clone()
    }
}

ferrum_sdk::inventory::submit! {
    ferrum_sdk::TransformerFactory {
        name: "or",
        build: |args| OrTransformer.into_entry(args),
    }
}

#[cfg(test)]
mod tests {
    use ferrum_sdk::{Evaluator, ProviderId, Score, Transformer};

    use super::{Args, OrTransformer};

    struct StubEval(Vec<(ProviderId, Score)>);
    impl Evaluator for StubEval {
        fn score(&self, id: ProviderId) -> Score {
            self.0
                .iter()
                .find(|(p, _)| *p == id)
                .map(|(_, s)| *s)
                .unwrap_or(Score(0))
        }
    }

    fn pid(n: u64) -> ProviderId {
        ProviderId(n)
    }

    #[test]
    fn or_returns_max() {
        let args = Args {
            inputs: vec![pid(1), pid(2)],
        };
        let eval = StubEval(vec![(pid(1), Score(30)), (pid(2), Score(70))]);
        assert_eq!(OrTransformer.evaluate(&args, &eval), Score(70));
    }

    #[test]
    fn or_empty_returns_zero() {
        let args = Args { inputs: vec![] };
        let eval = StubEval(vec![]);
        assert_eq!(OrTransformer.evaluate(&args, &eval), Score(0));
    }
}
