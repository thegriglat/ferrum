use ferrum_sdk::{Evaluator, Plugin, ProviderId, Score, Transformer};

/// Compiled args for the `and` transformer.
pub struct Args {
    pub inputs: Vec<ProviderId>,
}

/// `and` transformer: fuzzy AND — `min(inputs)`.
pub struct AndTransformer;

impl Plugin for AndTransformer {}

impl Transformer for AndTransformer {
    type Args = Args;

    fn compile_args(&self, args: &ferrum_sdk::toml::Value) -> Args {
        let arr = args
            .get("inputs")
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| {
                panic!("transformer-and: 'inputs' must be an array of provider names")
            });
        let inputs = arr
            .iter()
            .map(|v| {
                let name = v.as_str().unwrap_or_else(|| {
                    panic!("transformer-and: each entry in 'inputs' must be a string")
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
            .fold(Score(100), |a, b| a.and(b))
    }

    fn dep_ids(args: &Args) -> Vec<ProviderId> {
        args.inputs.clone()
    }
}

ferrum_sdk::inventory::submit! {
    ferrum_sdk::TransformerFactory {
        name: "and",
        build: |args| AndTransformer.into_entry(args),
    }
}

#[cfg(test)]
mod tests {
    use ferrum_sdk::{Evaluator, ProviderId, Score, Transformer};

    use super::{AndTransformer, Args};

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
    fn and_returns_min() {
        let args = Args {
            inputs: vec![pid(1), pid(2)],
        };
        let eval = StubEval(vec![(pid(1), Score(30)), (pid(2), Score(70))]);
        assert_eq!(AndTransformer.evaluate(&args, &eval), Score(30));
    }

    #[test]
    fn and_empty_returns_hundred() {
        let args = Args { inputs: vec![] };
        let eval = StubEval(vec![]);
        assert_eq!(AndTransformer.evaluate(&args, &eval), Score(100));
    }
}
