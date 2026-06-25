use ferrum_sdk::{Plugin, RequestContext, Score, Sensor};
use regex::Regex;

/// Sensor that inspects a single named cookie from the `Cookie` request header.
pub struct CookieSensor;

/// How to test the cookie value after it is found by name.
pub enum CookieOp {
    /// Cookie is present (its value is not checked).
    Present,
    /// Cookie value equals the configured string exactly.
    Eq(String),
    /// Cookie value matches a compiled regex.
    Regex(Regex),
}

/// Compiled arguments for [`CookieSensor`].
pub struct CookieArgs {
    /// Cookie name to look up (case-sensitive, as per RFC 6265).
    pub name: String,
    /// How to test the cookie once found.
    pub op: CookieOp,
}

impl Plugin for CookieSensor {}

impl Sensor for CookieSensor {
    type Args = CookieArgs;

    fn compile_args(&self, args: &ferrum_sdk::toml::Value) -> CookieArgs {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .expect("sensor-cookie: missing 'name'")
            .to_string();

        let op_str = args.get("op").and_then(|v| v.as_str()).unwrap_or("present");

        let op = match op_str {
            "present" => CookieOp::Present,
            "eq" => {
                let val = args
                    .get("value")
                    .and_then(|v| v.as_str())
                    .expect("sensor-cookie: 'eq' op requires 'value'")
                    .to_string();
                CookieOp::Eq(val)
            }
            "regex" => {
                let pat = args
                    .get("value")
                    .and_then(|v| v.as_str())
                    .expect("sensor-cookie: 'regex' op requires 'value'");
                CookieOp::Regex(Regex::new(pat).expect("sensor-cookie: invalid regex in 'value'"))
            }
            other => panic!("sensor-cookie: unknown op '{other}'"),
        };

        CookieArgs { name, op }
    }

    fn evaluate(&self, ctx: &mut RequestContext, args: &CookieArgs) -> Score {
        let raw = match ctx.headers.get("cookie") {
            Some(v) => v.as_str(),
            None => return Score(0),
        };

        match &args.op {
            CookieOp::Present => find_cookie(raw, &args.name).is_some().into(),
            CookieOp::Eq(val) => (find_cookie(raw, &args.name) == Some(val.as_str())).into(),
            CookieOp::Regex(re) => find_cookie(raw, &args.name)
                .is_some_and(|v| re.is_match(v))
                .into(),
        }
    }
}

ferrum_sdk::inventory::submit! {
    ferrum_sdk::SensorFactory {
        name: "sensor-cookie",
        build: |args, own_id| CookieSensor.into_entry(args, own_id),
    }
}

/// Parses a raw `Cookie` header value and returns the value for `name`, if present.
///
/// Cookie pairs are split on `; ` and each pair on the first `=`.
fn find_cookie<'a>(raw: &'a str, name: &str) -> Option<&'a str> {
    for pair in raw.split(';') {
        let pair = pair.trim();
        match pair.split_once('=') {
            Some((k, v)) if k.trim() == name => return Some(v),
            None if pair == name => return Some(""),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use ferrum_sdk::{RequestContext, Score, Sensor};

    use super::CookieSensor;

    fn make_toml(name: &str, op: &str, value: Option<&str>) -> ferrum_sdk::toml::Value {
        let mut t = toml::value::Table::new();
        t.insert("name".into(), ferrum_sdk::toml::Value::String(name.into()));
        t.insert("op".into(), ferrum_sdk::toml::Value::String(op.into()));
        if let Some(v) = value {
            t.insert("value".into(), ferrum_sdk::toml::Value::String(v.into()));
        }
        ferrum_sdk::toml::Value::Table(t)
    }

    fn make_ctx(cookie: &str) -> RequestContext {
        let mut ctx = RequestContext::new(
            "1.2.3.4".parse::<IpAddr>().unwrap(),
            "GET".into(),
            "/".into(),
        );
        ctx.headers.insert("cookie".into(), cookie.into());
        ctx
    }

    #[test]
    fn present_found() {
        let s = CookieSensor;
        let args = s.compile_args(&make_toml("session", "present", None));
        assert_eq!(
            s.evaluate(&mut make_ctx("session=abc123; other=x"), &args),
            Score(100)
        );
    }

    #[test]
    fn present_not_found() {
        let s = CookieSensor;
        let args = s.compile_args(&make_toml("session", "present", None));
        assert_eq!(s.evaluate(&mut make_ctx("csrf=tok"), &args), Score(0));
    }

    #[test]
    fn no_cookie_header_returns_zero() {
        let s = CookieSensor;
        let args = s.compile_args(&make_toml("session", "present", None));
        let mut ctx = RequestContext::new(
            "1.2.3.4".parse::<IpAddr>().unwrap(),
            "GET".into(),
            "/".into(),
        );
        assert_eq!(s.evaluate(&mut ctx, &args), Score(0));
    }

    #[test]
    fn eq_match() {
        let s = CookieSensor;
        let args = s.compile_args(&make_toml("role", "eq", Some("admin")));
        assert_eq!(s.evaluate(&mut make_ctx("role=admin"), &args), Score(100));
    }

    #[test]
    fn eq_no_match() {
        let s = CookieSensor;
        let args = s.compile_args(&make_toml("role", "eq", Some("admin")));
        assert_eq!(s.evaluate(&mut make_ctx("role=user"), &args), Score(0));
    }

    #[test]
    fn regex_match() {
        let s = CookieSensor;
        let args = s.compile_args(&make_toml("token", "regex", Some(r"^[0-9a-f]{32}$")));
        assert_eq!(
            s.evaluate(
                &mut make_ctx("token=deadbeefdeadbeefdeadbeefdeadbeef"),
                &args
            ),
            Score(100)
        );
    }

    #[test]
    fn regex_no_match() {
        let s = CookieSensor;
        let args = s.compile_args(&make_toml("token", "regex", Some(r"^[0-9a-f]{32}$")));
        assert_eq!(s.evaluate(&mut make_ctx("token=short"), &args), Score(0));
    }

    #[test]
    fn multiple_cookies_finds_correct_one() {
        let s = CookieSensor;
        let args = s.compile_args(&make_toml("b", "eq", Some("2")));
        assert_eq!(
            s.evaluate(&mut make_ctx("a=1; b=2; c=3"), &args),
            Score(100)
        );
    }
}
