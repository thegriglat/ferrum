use ferrum_sdk::{Evaluator, Plugin, ProviderId, Score, Transformer};

/// Compiled args for the `not` transformer.
pub struct Args {
    pub input: ProviderId,
}

/// `not` transformer: fuzzy NOT — `100 - score`.
pub struct NotTransformer;

impl Plugin for NotTransformer {}

impl Transformer for NotTransformer {
    type Args = Args;

    fn compile_args(&self, args: &ferrum_sdk::toml::Value) -> Args {
        let name = args
            .get("input")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("transformer-not: 'not' requires an 'input' string"));
        Args {
            input: ProviderId::from(name),
        }
    }

    fn evaluate(&self, args: &Args, eval: &dyn Evaluator) -> Score {
        !eval.score(args.input)
    }

    fn dep_ids(args: &Args) -> Vec<ProviderId> {
        vec![args.input]
    }
}

ferrum_sdk::inventory::submit! {
    ferrum_sdk::TransformerFactory {
        name: "not",
        build: |args| NotTransformer.into_entry(args),
    }
}

#[cfg(test)]
mod tests {
    use ferrum_sdk::{Evaluator, ProviderId, Score, Transformer};

    use super::{Args, NotTransformer};

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
    fn not_inverts() {
        let args = Args { input: pid(1) };
        let eval = StubEval(vec![(pid(1), Score(40))]);
        assert_eq!(NotTransformer.evaluate(&args, &eval), Score(60));
    }
}
