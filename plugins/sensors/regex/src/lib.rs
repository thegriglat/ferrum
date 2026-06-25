use ferrum_sdk::{Plugin, RequestContext, Score, Sensor};
use regex::RegexSet;

const DEFAULT_MAX_LEN: usize = 8 * 1024;

/// Which part of the request to match patterns against.
#[derive(Debug, Clone)]
pub enum Target {
    /// Match against the request URI (path + query).
    Uri,
    /// Match against the raw request body.
    Body,
    /// Match against a specific header value (lower-cased name).
    Header(String),
}

/// Sensor that scores `matched_count / total_patterns` using a [`RegexSet`].
///
/// Uses the `regex` crate exclusively (linear-time matching, no backtracking).
pub struct RegexSensor;

/// Compiled arguments for [`RegexSensor`].
pub struct RegexArgs {
    /// Pre-compiled set of patterns.
    pub set: RegexSet,
    /// Total number of patterns (denominator for the score).
    pub total: usize,
    /// Which request field to inspect.
    pub target: Target,
    /// Inputs longer than this are skipped and return `0.0`.
    pub max_len: usize,
}

impl Plugin for RegexSensor {}

impl Sensor for RegexSensor {
    type Args = RegexArgs;

    fn compile_args(&self, args: &ferrum_sdk::toml::Value) -> RegexArgs {
        let patterns: Vec<String> = args
            .get("patterns")
            .and_then(|v| v.as_array())
            .expect("sensor-regex: missing 'patterns' array")
            .iter()
            .map(|v| {
                v.as_str()
                    .expect("sensor-regex: patterns must be strings")
                    .to_string()
            })
            .collect();

        let target_str = args
            .get("target")
            .and_then(|v| v.as_str())
            .expect("sensor-regex: missing 'target'");

        let target = parse_target(target_str);
        let total = patterns.len();
        let set = RegexSet::new(&patterns).expect("sensor-regex: invalid regex pattern");

        RegexArgs {
            set,
            total,
            target,
            max_len: DEFAULT_MAX_LEN,
        }
    }

    fn evaluate(&self, ctx: &mut RequestContext, args: &RegexArgs) -> Score {
        if args.total == 0 {
            return Score(0);
        }

        let input: Option<&str> = match &args.target {
            Target::Uri => Some(ctx.uri.as_str()),
            Target::Body => ctx
                .body
                .as_deref()
                .and_then(|b| std::str::from_utf8(b).ok()),
            Target::Header(name) => ctx.headers.get(name).map(|s| s.as_str()),
        };

        let input = match input {
            Some(s) if s.len() <= args.max_len => s,
            _ => return Score(0),
        };

        let matched = args.set.matches(input).iter().count();
        Score::from(matched as f32 / args.total as f32)
    }
}

ferrum_sdk::inventory::submit! {
    ferrum_sdk::SensorFactory {
        name: "sensor-regex",
        build: |args, own_id| RegexSensor.into_entry(args, own_id),
    }
}

fn parse_target(s: &str) -> Target {
    match s {
        "uri" => Target::Uri,
        "body" => Target::Body,
        other => {
            let name = other
                .strip_prefix("header:")
                .expect("sensor-regex: target must be 'uri', 'body', or 'header:<name>'");
            Target::Header(name.to_lowercase())
        }
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use ferrum_sdk::{RequestContext, Score, Sensor};

    use super::RegexSensor;

    fn make_args(patterns: &[&str], target: &str) -> ferrum_sdk::toml::Value {
        let pats: toml::value::Array = patterns
            .iter()
            .map(|s| ferrum_sdk::toml::Value::String(s.to_string()))
            .collect();
        let mut t = toml::value::Table::new();
        t.insert("patterns".into(), ferrum_sdk::toml::Value::Array(pats));
        t.insert(
            "target".into(),
            ferrum_sdk::toml::Value::String(target.into()),
        );
        ferrum_sdk::toml::Value::Table(t)
    }

    fn ctx_with_body(body: &str) -> RequestContext {
        let mut ctx =
            RequestContext::new("1.2.3.4".parse().unwrap(), "POST".into(), "/login".into());
        ctx.body = Some(Bytes::copy_from_slice(body.as_bytes()));
        ctx
    }

    fn ctx_with_uri(uri: &str) -> RequestContext {
        RequestContext::new("1.2.3.4".parse().unwrap(), "GET".into(), uri.into())
    }

    #[test]
    fn sqli_union_select_in_body() {
        let sensor = RegexSensor;
        let compiled = sensor.compile_args(&make_args(&["(?i)union.*select"], "body"));
        let mut ctx = ctx_with_body("1 UNION SELECT * FROM users");
        assert_eq!(sensor.evaluate(&mut ctx, &compiled), Score(100));
    }

    #[test]
    fn sqli_or_tautology_in_body() {
        let sensor = RegexSensor;
        let compiled = sensor.compile_args(&make_args(
            &["(?i)union.*select", "(?i)' or '1'='1"],
            "body",
        ));
        let mut ctx = ctx_with_body("' or '1'='1");
        // 1 of 2 patterns matched → 50
        assert_eq!(sensor.evaluate(&mut ctx, &compiled), Score(50));
    }

    #[test]
    fn sqli_union_select_in_uri() {
        let sensor = RegexSensor;
        let compiled = sensor.compile_args(&make_args(&["(?i)union.*select"], "uri"));
        let mut ctx = ctx_with_uri("/search?q=1+UNION+SELECT+1");
        assert_eq!(sensor.evaluate(&mut ctx, &compiled), Score(100));
    }

    #[test]
    fn body_exceeds_max_len_returns_zero() {
        let sensor = RegexSensor;
        let args_value = make_args(&["(?i)union.*select"], "body");
        let mut compiled = sensor.compile_args(&args_value);
        compiled.max_len = 4;
        let mut ctx = ctx_with_body("UNION SELECT 1");
        assert_eq!(sensor.evaluate(&mut ctx, &compiled), Score(0));
    }

    #[test]
    fn no_match_returns_zero() {
        let sensor = RegexSensor;
        let compiled = sensor.compile_args(&make_args(&["(?i)union.*select"], "body"));
        let mut ctx = ctx_with_body("hello world");
        assert_eq!(sensor.evaluate(&mut ctx, &compiled), Score(0));
    }
}
