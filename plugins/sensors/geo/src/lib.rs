use std::collections::HashSet;
use std::net::IpAddr;
use std::sync::Arc;

use ferrum_sdk::{Plugin, ProviderId, RequestContext, Score, Sensor, SensorEntry};

/// Abstracts GeoIP lookup to allow test doubles without a real `.mmdb` file.
pub trait GeoResolver: Send + Sync {
    /// Returns the ISO 3166-1 alpha-2 country code for `ip`, or `None` if unknown.
    fn country_code(&self, ip: IpAddr) -> Option<String>;
}

/// Production resolver backed by a MaxMind `.mmdb` database.
pub struct MaxMindResolver {
    reader: maxminddb::Reader<Vec<u8>>,
}

impl MaxMindResolver {
    /// Opens a `.mmdb` file from `path`.
    pub fn open(path: &str) -> Self {
        let reader = maxminddb::Reader::open_readfile(path)
            .expect("sensor-geo: failed to open .mmdb database");
        Self { reader }
    }
}

impl GeoResolver for MaxMindResolver {
    fn country_code(&self, ip: IpAddr) -> Option<String> {
        let record: maxminddb::geoip2::Country = self.reader.lookup(ip).ok()?;
        record
            .country
            .and_then(|c| c.iso_code)
            .map(|s| s.to_uppercase())
    }
}

/// Sensor that scores `1.0` if the client IP belongs to one of the configured countries.
pub struct GeoSensor;

/// Compiled arguments for [`GeoSensor`].
pub struct GeoArgs {
    /// GeoIP resolver (swappable for testing).
    pub resolver: Arc<dyn GeoResolver>,
    /// ISO 3166-1 alpha-2 country codes to match (upper-case).
    pub countries: HashSet<String>,
    /// This sensor's own [`ProviderId`], used as the metadata key for the resolved country code.
    ///
    /// Set by the loader from the sensor's instance name in the TOML config —
    /// not hardcoded to the plugin name.
    pub provider_id: ProviderId,
}

impl Plugin for GeoSensor {}

impl Sensor for GeoSensor {
    type Args = GeoArgs;

    fn compile_args(&self, args: &ferrum_sdk::toml::Value) -> GeoArgs {
        let db_path = args
            .get("db_path")
            .and_then(|v| v.as_str())
            .expect("sensor-geo: missing 'db_path'");

        let countries: HashSet<String> = args
            .get("countries")
            .and_then(|v| v.as_array())
            .expect("sensor-geo: missing 'countries' array")
            .iter()
            .map(|v| {
                v.as_str()
                    .expect("sensor-geo: country codes must be strings")
                    .to_uppercase()
            })
            .collect();

        GeoArgs {
            resolver: Arc::new(MaxMindResolver::open(db_path)),
            countries,
            provider_id: ProviderId(0), // overwritten by the loader with the real instance id
        }
    }

    fn evaluate(&self, ctx: &mut RequestContext, args: &GeoArgs) -> Score {
        let code = args
            .resolver
            .country_code(ctx.client_ip)
            .unwrap_or_default();

        ctx.metadata.insert(args.provider_id, code.clone());

        args.countries.contains(&code).into()
    }
}

ferrum_sdk::inventory::submit! {
    ferrum_sdk::SensorFactory {
        name: "sensor-geo",
        build: |args, own_id| {
            let p = GeoSensor;
            let mut compiled = p.compile_args(args);
            compiled.provider_id = own_id;
            SensorEntry {
                evaluate: Box::new(move |ctx| p.evaluate(ctx, &compiled)),
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::net::IpAddr;
    use std::sync::Arc;

    use ferrum_sdk::{ProviderId, RequestContext, Score, Sensor};

    use super::{GeoArgs, GeoResolver, GeoSensor};

    const TEST_ID: ProviderId = ProviderId(42);

    struct StubResolver {
        code: Option<String>,
    }

    impl GeoResolver for StubResolver {
        fn country_code(&self, _ip: IpAddr) -> Option<String> {
            self.code.clone()
        }
    }

    fn make_ctx(ip: &str) -> RequestContext {
        RequestContext::new(ip.parse::<IpAddr>().unwrap(), "GET".into(), "/".into())
    }

    fn make_args(resolver: Arc<dyn GeoResolver>, countries: &[&str]) -> GeoArgs {
        GeoArgs {
            resolver,
            countries: countries
                .iter()
                .map(|s| s.to_string())
                .collect::<HashSet<_>>(),
            provider_id: TEST_ID,
        }
    }

    #[test]
    fn match_country() {
        let sensor = GeoSensor;
        let args = make_args(
            Arc::new(StubResolver {
                code: Some("US".into()),
            }),
            &["US", "CA"],
        );
        let mut ctx = make_ctx("1.2.3.4");
        assert_eq!(sensor.evaluate(&mut ctx, &args), Score(100));
    }

    #[test]
    fn no_match_country() {
        let sensor = GeoSensor;
        let args = make_args(
            Arc::new(StubResolver {
                code: Some("DE".into()),
            }),
            &["US"],
        );
        let mut ctx = make_ctx("1.2.3.4");
        assert_eq!(sensor.evaluate(&mut ctx, &args), Score(0));
    }

    #[test]
    fn unknown_ip_writes_empty_metadata() {
        let sensor = GeoSensor;
        let args = make_args(Arc::new(StubResolver { code: None }), &["US"]);
        let mut ctx = make_ctx("1.2.3.4");
        let score = sensor.evaluate(&mut ctx, &args);
        assert_eq!(score, Score(0));
        assert_eq!(ctx.metadata.get(&TEST_ID), Some(&String::new()));
    }

    #[test]
    fn metadata_written_on_match() {
        let sensor = GeoSensor;
        let args = make_args(
            Arc::new(StubResolver {
                code: Some("RU".into()),
            }),
            &["RU"],
        );
        let mut ctx = make_ctx("1.2.3.4");
        sensor.evaluate(&mut ctx, &args);
        assert_eq!(ctx.metadata.get(&TEST_ID), Some(&"RU".to_string()));
    }
}
