use ferrum_sdk::{Plugin, RequestContext, Score, Sensor};

/// Sensor that returns a proportional score based on the size of the request body.
///
/// Returns `Score(0)` when the body is absent or smaller than `min_bytes`,
/// `Score(100)` when it reaches or exceeds `max_bytes`, and a linear interpolation
/// in between.
///
/// Note: the body must be buffered via `buffer_body = true` in the rule for this
/// sensor to see it; otherwise `ctx.body` is `None` and the sensor returns `Score(0)`.
pub struct BodySizeSensor;

/// Compiled arguments for [`BodySizeSensor`].
pub struct BodySizeArgs {
    /// Lower bound in bytes; bodies at or below this size score `0`.
    pub min_bytes: usize,
    /// Upper bound in bytes; bodies at or above this size score `100`.
    pub max_bytes: usize,
}

impl Plugin for BodySizeSensor {}

impl Sensor for BodySizeSensor {
    type Args = BodySizeArgs;

    fn compile_args(&self, args: &ferrum_sdk::toml::Value) -> BodySizeArgs {
        let min_bytes = args
            .get("min_bytes")
            .and_then(|v| v.as_integer())
            .unwrap_or(0) as usize;

        let max_bytes = args
            .get("max_bytes")
            .and_then(|v| v.as_integer())
            .expect("sensor-body-size: missing 'max_bytes'") as usize;

        assert!(
            max_bytes > min_bytes,
            "sensor-body-size: 'max_bytes' must be greater than 'min_bytes'"
        );

        BodySizeArgs {
            min_bytes,
            max_bytes,
        }
    }

    fn evaluate(&self, ctx: &mut RequestContext, args: &BodySizeArgs) -> Score {
        let size = match &ctx.body {
            Some(b) => b.len(),
            None => return Score(0),
        };

        if size <= args.min_bytes {
            return Score(0);
        }

        if size >= args.max_bytes {
            return Score(100);
        }

        let ratio = (size - args.min_bytes) as f64 / (args.max_bytes - args.min_bytes) as f64;
        Score::from(ratio)
    }
}

ferrum_sdk::inventory::submit! {
    ferrum_sdk::SensorFactory {
        name: "sensor-body-size",
        build: |args, own_id| BodySizeSensor.into_entry(args, own_id),
    }
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use bytes::Bytes;
    use ferrum_sdk::{RequestContext, Score, Sensor};

    use super::BodySizeSensor;

    fn make_toml(min_bytes: Option<i64>, max_bytes: i64) -> ferrum_sdk::toml::Value {
        let mut t = toml::value::Table::new();
        if let Some(min) = min_bytes {
            t.insert("min_bytes".into(), ferrum_sdk::toml::Value::Integer(min));
        }
        t.insert(
            "max_bytes".into(),
            ferrum_sdk::toml::Value::Integer(max_bytes),
        );
        ferrum_sdk::toml::Value::Table(t)
    }

    fn make_ctx(body: Option<&[u8]>) -> RequestContext {
        let mut ctx = RequestContext::new(
            "1.2.3.4".parse::<IpAddr>().unwrap(),
            "POST".into(),
            "/upload".into(),
        );
        ctx.body = body.map(Bytes::copy_from_slice);
        ctx
    }

    #[test]
    fn no_body_returns_zero() {
        let s = BodySizeSensor;
        let args = s.compile_args(&make_toml(None, 1024));
        assert_eq!(s.evaluate(&mut make_ctx(None), &args), Score(0));
    }

    #[test]
    fn empty_body_returns_zero() {
        let s = BodySizeSensor;
        let args = s.compile_args(&make_toml(None, 1024));
        assert_eq!(s.evaluate(&mut make_ctx(Some(b"")), &args), Score(0));
    }

    #[test]
    fn at_max_returns_hundred() {
        let s = BodySizeSensor;
        let args = s.compile_args(&make_toml(None, 4));
        let body = b"abcd";
        assert_eq!(s.evaluate(&mut make_ctx(Some(body)), &args), Score(100));
    }

    #[test]
    fn above_max_returns_hundred() {
        let s = BodySizeSensor;
        let args = s.compile_args(&make_toml(None, 4));
        assert_eq!(s.evaluate(&mut make_ctx(Some(b"abcde")), &args), Score(100));
    }

    #[test]
    fn halfway_returns_fifty() {
        let s = BodySizeSensor;
        // min=0, max=100 → 50 bytes → Score(50)
        let args = s.compile_args(&make_toml(None, 100));
        let body = vec![0u8; 50];
        assert_eq!(s.evaluate(&mut make_ctx(Some(&body)), &args), Score(50));
    }

    #[test]
    fn below_min_returns_zero() {
        let s = BodySizeSensor;
        // min=100, max=200 → 50 bytes → below min → Score(0)
        let args = s.compile_args(&make_toml(Some(100), 200));
        let body = vec![0u8; 50];
        assert_eq!(s.evaluate(&mut make_ctx(Some(&body)), &args), Score(0));
    }

    #[test]
    fn between_min_and_max_is_proportional() {
        let s = BodySizeSensor;
        // min=0, max=200 → 100 bytes → 50%
        let args = s.compile_args(&make_toml(Some(0), 200));
        let body = vec![0u8; 100];
        assert_eq!(s.evaluate(&mut make_ctx(Some(&body)), &args), Score(50));
    }
}
