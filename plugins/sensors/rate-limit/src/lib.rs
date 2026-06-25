use std::sync::Arc;
use std::time::Duration;

use ferrum_sdk::{Clock, Plugin, ProviderId, RequestContext, Score, Sensor, SystemClock};
use moka::sync::Cache;

/// Sensor that scores based on per-key request rate within a sliding TTL window.
///
/// The score is `(count * 100 / limit).clamp(0, 100)`.
/// The key is read from `ctx.metadata[source_provider_id]`.
pub struct RateLimitSensor;

/// Compiled arguments for [`RateLimitSensor`].
pub struct RateLimitArgs {
    /// Moka cache mapping string key → hit count.  TTL set to `window_secs`.
    pub cache: Arc<Cache<String, u32>>,
    /// Maximum allowed requests before the score reaches `1.0`.
    pub limit: u32,
    /// Provider whose metadata entry is used as the rate-limit key.
    pub source_provider_id: ProviderId,
    /// Clock used for time operations (swappable for testing).
    pub clock: Arc<dyn Clock>,
}

impl Plugin for RateLimitSensor {}

impl Sensor for RateLimitSensor {
    type Args = RateLimitArgs;

    fn compile_args(&self, args: &ferrum_sdk::toml::Value) -> RateLimitArgs {
        let limit = args
            .get("limit")
            .and_then(|v| v.as_integer())
            .expect("sensor-rate-limit: missing 'limit'") as u32;

        let window_secs = args
            .get("window_secs")
            .and_then(|v| v.as_integer())
            .expect("sensor-rate-limit: missing 'window_secs'") as u64;

        let source_id_str = args
            .get("source_provider_id")
            .and_then(|v| v.as_str())
            .expect("sensor-rate-limit: missing 'source_provider_id'");

        let source_provider_id = name_to_provider_id(source_id_str);

        let cache = Arc::new(
            Cache::builder()
                .time_to_live(Duration::from_secs(window_secs))
                .build(),
        );

        RateLimitArgs {
            cache,
            limit,
            source_provider_id,
            clock: Arc::new(SystemClock),
        }
    }

    fn evaluate(&self, ctx: &mut RequestContext, args: &RateLimitArgs) -> Score {
        let key = match ctx.metadata.get(&args.source_provider_id) {
            Some(k) => k.clone(),
            None => ctx.client_ip.to_string(),
        };

        // Atomically increment the counter.  Moka's `and_upsert_with` gives us a
        // mutable reference to the existing entry or inserts a new one.
        let count = args
            .cache
            .entry(key)
            .and_upsert_with(|opt| opt.map(|e| e.into_value() + 1).unwrap_or(1));

        Score::from(count.into_value() as f32 / args.limit as f32)
    }
}

/// Derives a [`ProviderId`] from an arbitrary sensor name string.
pub fn name_to_provider_id(name: &str) -> ProviderId {
    ProviderId::from(name)
}

ferrum_sdk::inventory::submit! {
    ferrum_sdk::SensorFactory {
        name: "sensor-rate-limit",
        build: |args, own_id| RateLimitSensor.into_entry(args, own_id),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use ferrum_sdk::{MockClock, ProviderId, RequestContext, Score, Sensor};
    use moka::sync::Cache;

    use super::{RateLimitArgs, RateLimitSensor, name_to_provider_id};

    fn make_ctx(ip: &str) -> RequestContext {
        RequestContext::new(ip.parse().unwrap(), "GET".into(), "/".into())
    }

    fn make_args(limit: u32, window_secs: u64, clock: Arc<dyn ferrum_sdk::Clock>) -> RateLimitArgs {
        let cache = Arc::new(
            Cache::builder()
                .time_to_live(Duration::from_secs(window_secs))
                .build(),
        );
        RateLimitArgs {
            cache,
            limit,
            source_provider_id: ProviderId(0),
            clock,
        }
    }

    #[test]
    fn score_increases_with_requests() {
        let sensor = RateLimitSensor;
        let args = make_args(10, 60, Arc::new(ferrum_sdk::SystemClock));
        let mut ctx = make_ctx("1.2.3.4");

        for _ in 0..9 {
            let score = sensor.evaluate(&mut ctx, &args);
            assert!(score < Score(100));
        }
        let score = sensor.evaluate(&mut ctx, &args);
        assert_eq!(score, Score(100));
    }

    #[test]
    fn score_clamps_at_one() {
        let sensor = RateLimitSensor;
        let args = make_args(3, 60, Arc::new(ferrum_sdk::SystemClock));
        let mut ctx = make_ctx("1.2.3.4");

        for _ in 0..5 {
            sensor.evaluate(&mut ctx, &args);
        }
        let score = sensor.evaluate(&mut ctx, &args);
        assert_eq!(score, Score(100));
    }

    #[test]
    fn uses_metadata_key_when_present() {
        let sensor = RateLimitSensor;
        let source_id = name_to_provider_id("geo");
        let cache = Arc::new(
            Cache::builder()
                .time_to_live(Duration::from_secs(60))
                .build(),
        );
        let args = RateLimitArgs {
            cache,
            limit: 2,
            source_provider_id: source_id,
            clock: Arc::new(ferrum_sdk::SystemClock),
        };

        let mut ctx = make_ctx("1.2.3.4");
        ctx.metadata.insert(source_id, "RU".into());

        sensor.evaluate(&mut ctx, &args);
        let score = sensor.evaluate(&mut ctx, &args);
        assert_eq!(score, Score(100));
    }

    /// Fill the counter to `1.0`, then advance `MockClock` past the window.
    /// Moka evicts by TTL, so after expiry the counter resets.
    #[test]
    fn counter_resets_after_window_expires() {
        let sensor = RateLimitSensor;
        let window_secs = 1u64;
        // Use a very short TTL so moka evicts quickly.
        let cache = Arc::new(
            Cache::builder()
                .time_to_live(Duration::from_millis(50))
                .build(),
        );
        let clock = Arc::new(MockClock {
            offset: Duration::ZERO,
        });
        let args = RateLimitArgs {
            cache: cache.clone(),
            limit: 2,
            source_provider_id: ProviderId(0),
            clock,
        };

        let mut ctx = make_ctx("9.9.9.9");

        // Hit the limit.
        sensor.evaluate(&mut ctx, &args);
        let score = sensor.evaluate(&mut ctx, &args);
        assert_eq!(score, Score(100));

        // Wait for TTL to pass (no sleep — real time for moka's background eviction).
        std::thread::sleep(Duration::from_millis(100));
        cache.run_pending_tasks();

        // Counter should have reset.
        let score = sensor.evaluate(&mut ctx, &args);
        assert!(score < Score(100), "expected reset score, got {score}");

        let _ = window_secs;
    }
}
