use std::collections::HashSet;
use std::net::IpAddr;
use std::sync::Arc;

use ferrum_sdk::{Plugin, ProviderId, RequestContext, Score, Sensor, SensorEntry};

/// Abstracts AS number lookup to allow test doubles without a real `.mmdb` file.
pub trait AsnResolver: Send + Sync {
    /// Returns the Autonomous System Number for `ip`, or `None` if unknown.
    fn asn(&self, ip: IpAddr) -> Option<u32>;
}

/// Production resolver backed by a MaxMind GeoLite2-ASN `.mmdb` database.
pub struct MaxMindAsnResolver {
    reader: maxminddb::Reader<Vec<u8>>,
}

impl MaxMindAsnResolver {
    /// Opens a GeoLite2-ASN `.mmdb` file from `path`.
    pub fn open(path: &str) -> Self {
        let reader = maxminddb::Reader::open_readfile(path)
            .expect("sensor-asn: failed to open .mmdb database");
        Self { reader }
    }
}

impl AsnResolver for MaxMindAsnResolver {
    fn asn(&self, ip: IpAddr) -> Option<u32> {
        let record: maxminddb::geoip2::Asn = self.reader.lookup(ip).ok()?;
        record.autonomous_system_number
    }
}

/// Sensor that scores `100` when the client IP belongs to one of the configured AS numbers.
///
/// Also writes the resolved ASN as a string into `ctx.metadata` under its own
/// [`ProviderId`], making it available as a key for `sensor-rate-limit`.
pub struct AsnSensor;

/// Compiled arguments for [`AsnSensor`].
pub struct AsnArgs {
    /// ASN resolver (swappable for testing).
    pub resolver: Arc<dyn AsnResolver>,
    /// Set of AS numbers to match.
    pub asns: HashSet<u32>,
    /// This sensor's own [`ProviderId`], used as the metadata key for the resolved ASN.
    pub provider_id: ProviderId,
}

impl Plugin for AsnSensor {}

impl Sensor for AsnSensor {
    type Args = AsnArgs;

    fn compile_args(&self, args: &ferrum_sdk::toml::Value) -> AsnArgs {
        let db_path = args
            .get("db_path")
            .and_then(|v| v.as_str())
            .expect("sensor-asn: missing 'db_path'");

        let asns: HashSet<u32> = args
            .get("asns")
            .and_then(|v| v.as_array())
            .expect("sensor-asn: missing 'asns' array")
            .iter()
            .map(|v| {
                v.as_integer()
                    .expect("sensor-asn: each entry in 'asns' must be an integer")
                    as u32
            })
            .collect();

        AsnArgs {
            resolver: Arc::new(MaxMindAsnResolver::open(db_path)),
            asns,
            provider_id: ProviderId(0), // overwritten by the loader with the real instance id
        }
    }

    fn evaluate(&self, ctx: &mut RequestContext, args: &AsnArgs) -> Score {
        let asn = args.resolver.asn(ctx.client_ip).unwrap_or(0);
        ctx.metadata.insert(args.provider_id, asn.to_string());
        args.asns.contains(&asn).into()
    }
}

ferrum_sdk::inventory::submit! {
    ferrum_sdk::SensorFactory {
        name: "sensor-asn",
        build: |args, own_id| {
            let p = AsnSensor;
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

    use super::{AsnArgs, AsnResolver, AsnSensor};

    const TEST_ID: ProviderId = ProviderId(99);

    struct StubResolver {
        asn: Option<u32>,
    }

    impl AsnResolver for StubResolver {
        fn asn(&self, _ip: IpAddr) -> Option<u32> {
            self.asn
        }
    }

    fn make_ctx() -> RequestContext {
        RequestContext::new(
            "1.2.3.4".parse::<IpAddr>().unwrap(),
            "GET".into(),
            "/".into(),
        )
    }

    fn make_args(resolver: Arc<dyn AsnResolver>, asns: &[u32]) -> AsnArgs {
        AsnArgs {
            resolver,
            asns: asns.iter().copied().collect::<HashSet<_>>(),
            provider_id: TEST_ID,
        }
    }

    #[test]
    fn asn_match() {
        let s = AsnSensor;
        let args = make_args(Arc::new(StubResolver { asn: Some(14061) }), &[14061, 16509]);
        let mut ctx = make_ctx();
        assert_eq!(s.evaluate(&mut ctx, &args), Score(100));
    }

    #[test]
    fn asn_no_match() {
        let s = AsnSensor;
        let args = make_args(Arc::new(StubResolver { asn: Some(12345) }), &[14061]);
        let mut ctx = make_ctx();
        assert_eq!(s.evaluate(&mut ctx, &args), Score(0));
    }

    #[test]
    fn unknown_ip_returns_zero() {
        let s = AsnSensor;
        let args = make_args(Arc::new(StubResolver { asn: None }), &[14061]);
        let mut ctx = make_ctx();
        assert_eq!(s.evaluate(&mut ctx, &args), Score(0));
    }

    #[test]
    fn metadata_written_on_match() {
        let s = AsnSensor;
        let args = make_args(Arc::new(StubResolver { asn: Some(14061) }), &[14061]);
        let mut ctx = make_ctx();
        s.evaluate(&mut ctx, &args);
        assert_eq!(ctx.metadata.get(&TEST_ID), Some(&"14061".to_string()));
    }

    #[test]
    fn metadata_written_as_zero_when_unknown() {
        let s = AsnSensor;
        let args = make_args(Arc::new(StubResolver { asn: None }), &[14061]);
        let mut ctx = make_ctx();
        s.evaluate(&mut ctx, &args);
        assert_eq!(ctx.metadata.get(&TEST_ID), Some(&"0".to_string()));
    }
}
