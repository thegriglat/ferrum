use ferrum_sdk::{Plugin, RequestContext, Score, Sensor};

/// Which HTTP field to extract and compare.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Field {
    /// HTTP method (e.g. `GET`, `POST`).
    Method,
    /// URL path component, without the query string.
    Path,
    /// URL query string, without the leading `?`.
    Query,
    /// `Host` request header value.
    Host,
    /// `Content-Type` request header value.
    ContentType,
}

/// How to compare the extracted field value against each entry in `values`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Op {
    /// Exact equality.
    Eq,
    /// Field starts with the pattern.
    Prefix,
    /// Field ends with the pattern.
    Suffix,
    /// Pattern is a substring of the field.
    Contains,
}

/// Sensor that scores `100` when a named HTTP field matches any of the configured values.
pub struct StringMatchSensor;

/// Compiled arguments for [`StringMatchSensor`].
pub struct StringMatchArgs {
    /// Which field to extract from the request.
    pub field: Field,
    /// Comparison operator.
    pub op: Op,
    /// List of patterns; any match returns `Score(100)`.
    pub values: Vec<String>,
}

impl Plugin for StringMatchSensor {}

impl Sensor for StringMatchSensor {
    type Args = StringMatchArgs;

    fn compile_args(&self, args: &ferrum_sdk::toml::Value) -> StringMatchArgs {
        let field_str = args
            .get("field")
            .and_then(|v| v.as_str())
            .expect("sensor-string-match: missing 'field'");

        let field = match field_str {
            "method" => Field::Method,
            "path" => Field::Path,
            "query" => Field::Query,
            "host" => Field::Host,
            "content-type" => Field::ContentType,
            other => panic!("sensor-string-match: unknown field '{other}'"),
        };

        let op_str = args.get("op").and_then(|v| v.as_str()).unwrap_or("eq");

        let op = match op_str {
            "eq" => Op::Eq,
            "prefix" => Op::Prefix,
            "suffix" => Op::Suffix,
            "contains" => Op::Contains,
            other => panic!("sensor-string-match: unknown op '{other}'"),
        };

        let values: Vec<String> = args
            .get("values")
            .and_then(|v| v.as_array())
            .expect("sensor-string-match: missing 'values' array")
            .iter()
            .map(|v| {
                v.as_str()
                    .expect("sensor-string-match: values must be strings")
                    .to_string()
            })
            .collect();

        StringMatchArgs { field, op, values }
    }

    fn evaluate(&self, ctx: &mut RequestContext, args: &StringMatchArgs) -> Score {
        let value: Option<String> = match args.field {
            Field::Method => Some(ctx.method.clone()),
            Field::Path => Some(
                ctx.uri
                    .split_once('?')
                    .map_or(ctx.uri.as_str(), |(p, _)| p)
                    .to_string(),
            ),
            Field::Query => Some(ctx.uri.split_once('?').map_or("", |(_, q)| q).to_string()),
            Field::Host => ctx.headers.get("host").cloned(),
            Field::ContentType => ctx.headers.get("content-type").cloned(),
        };

        let value = match value {
            Some(v) => v,
            None => return Score(0),
        };

        let matched = args.values.iter().any(|pat| match args.op {
            Op::Eq => value == *pat,
            Op::Prefix => value.starts_with(pat.as_str()),
            Op::Suffix => value.ends_with(pat.as_str()),
            Op::Contains => value.contains(pat.as_str()),
        });

        matched.into()
    }
}

ferrum_sdk::inventory::submit! {
    ferrum_sdk::SensorFactory {
        name: "sensor-string-match",
        build: |args, own_id| StringMatchSensor.into_entry(args, own_id),
    }
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use ferrum_sdk::{RequestContext, Score, Sensor};

    use super::StringMatchSensor;

    fn make_toml(field: &str, op: &str, values: &[&str]) -> ferrum_sdk::toml::Value {
        let list: toml::value::Array = values
            .iter()
            .map(|s| ferrum_sdk::toml::Value::String(s.to_string()))
            .collect();
        let mut t = toml::value::Table::new();
        t.insert(
            "field".into(),
            ferrum_sdk::toml::Value::String(field.into()),
        );
        t.insert("op".into(), ferrum_sdk::toml::Value::String(op.into()));
        t.insert("values".into(), ferrum_sdk::toml::Value::Array(list));
        ferrum_sdk::toml::Value::Table(t)
    }

    fn make_ctx(method: &str, uri: &str) -> RequestContext {
        RequestContext::new(
            "1.2.3.4".parse::<IpAddr>().unwrap(),
            method.into(),
            uri.into(),
        )
    }

    fn make_ctx_with_header(method: &str, uri: &str, name: &str, value: &str) -> RequestContext {
        let mut ctx = make_ctx(method, uri);
        ctx.headers.insert(name.to_string(), value.to_string());
        ctx
    }

    #[test]
    fn method_eq_match() {
        let s = StringMatchSensor;
        let args = s.compile_args(&make_toml("method", "eq", &["POST", "PUT"]));
        assert_eq!(s.evaluate(&mut make_ctx("POST", "/"), &args), Score(100));
    }

    #[test]
    fn method_eq_no_match() {
        let s = StringMatchSensor;
        let args = s.compile_args(&make_toml("method", "eq", &["POST"]));
        assert_eq!(s.evaluate(&mut make_ctx("GET", "/"), &args), Score(0));
    }

    #[test]
    fn path_prefix_match() {
        let s = StringMatchSensor;
        let args = s.compile_args(&make_toml("path", "prefix", &["/admin"]));
        assert_eq!(
            s.evaluate(&mut make_ctx("GET", "/admin/users"), &args),
            Score(100)
        );
    }

    #[test]
    fn path_strips_query() {
        let s = StringMatchSensor;
        let args = s.compile_args(&make_toml("path", "eq", &["/search"]));
        assert_eq!(
            s.evaluate(&mut make_ctx("GET", "/search?q=foo"), &args),
            Score(100)
        );
    }

    #[test]
    fn query_contains_match() {
        let s = StringMatchSensor;
        let args = s.compile_args(&make_toml("query", "contains", &["drop table"]));
        assert_eq!(
            s.evaluate(&mut make_ctx("GET", "/page?q=drop table users"), &args),
            Score(100)
        );
    }

    #[test]
    fn query_empty_when_no_question_mark() {
        let s = StringMatchSensor;
        let args = s.compile_args(&make_toml("query", "eq", &[""]));
        assert_eq!(s.evaluate(&mut make_ctx("GET", "/page"), &args), Score(100));
    }

    #[test]
    fn host_eq_match() {
        let s = StringMatchSensor;
        let args = s.compile_args(&make_toml("host", "eq", &["example.com"]));
        let mut ctx = make_ctx_with_header("GET", "/", "host", "example.com");
        assert_eq!(s.evaluate(&mut ctx, &args), Score(100));
    }

    #[test]
    fn host_missing_returns_zero() {
        let s = StringMatchSensor;
        let args = s.compile_args(&make_toml("host", "eq", &["example.com"]));
        assert_eq!(s.evaluate(&mut make_ctx("GET", "/"), &args), Score(0));
    }

    #[test]
    fn content_type_suffix_match() {
        let s = StringMatchSensor;
        let args = s.compile_args(&make_toml("content-type", "suffix", &["/json"]));
        let mut ctx = make_ctx_with_header("POST", "/api", "content-type", "application/json");
        assert_eq!(s.evaluate(&mut ctx, &args), Score(100));
    }
}
