use std::collections::HashSet;
use std::net::IpAddr;

use ferrum_sdk::{Plugin, RequestContext, Score, Sensor};

/// Sensor that scores `1.0` if the client IP is in a configured allow/block list.
pub struct IpSensor;

/// Compiled arguments for [`IpSensor`].
pub struct IpArgs {
    /// Set of IP addresses to match against.
    pub allowed: HashSet<IpAddr>,
}

impl Plugin for IpSensor {}

impl Sensor for IpSensor {
    type Args = IpArgs;

    fn compile_args(&self, args: &ferrum_sdk::toml::Value) -> IpArgs {
        let ips = args
            .get("ips")
            .and_then(|v| v.as_array())
            .expect("sensor-ip: missing 'ips' array");

        let allowed = ips
            .iter()
            .map(|v| {
                v.as_str()
                    .expect("sensor-ip: each entry in 'ips' must be a string")
                    .parse::<IpAddr>()
                    .expect("sensor-ip: invalid IP address")
            })
            .collect();

        IpArgs { allowed }
    }

    fn evaluate(&self, ctx: &mut RequestContext, args: &IpArgs) -> Score {
        args.allowed.contains(&ctx.client_ip).into()
    }
}

ferrum_sdk::inventory::submit! {
    ferrum_sdk::SensorFactory {
        name: "sensor-ip",
        build: |args, own_id| IpSensor.into_entry(args, own_id),
    }
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use ferrum_sdk::{RequestContext, Score, Sensor};

    use super::IpSensor;

    fn make_ctx(ip: &str) -> RequestContext {
        RequestContext::new(ip.parse::<IpAddr>().unwrap(), "GET".into(), "/".into())
    }

    fn args(ips: &[&str]) -> ferrum_sdk::toml::Value {
        let list: toml::value::Array = ips
            .iter()
            .map(|s| ferrum_sdk::toml::Value::String(s.to_string()))
            .collect();
        let mut t = toml::value::Table::new();
        t.insert("ips".into(), ferrum_sdk::toml::Value::Array(list));
        ferrum_sdk::toml::Value::Table(t)
    }

    #[test]
    fn ipv4_match() {
        let sensor = IpSensor;
        let compiled = sensor.compile_args(&args(&["1.2.3.4", "5.6.7.8"]));
        assert_eq!(
            sensor.evaluate(&mut make_ctx("1.2.3.4"), &compiled),
            Score(100)
        );
    }

    #[test]
    fn ipv4_no_match() {
        let sensor = IpSensor;
        let compiled = sensor.compile_args(&args(&["1.2.3.4"]));
        assert_eq!(
            sensor.evaluate(&mut make_ctx("9.9.9.9"), &compiled),
            Score(0)
        );
    }

    #[test]
    fn ipv6_match() {
        let sensor = IpSensor;
        let compiled = sensor.compile_args(&args(&["::1", "2001:db8::1"]));
        assert_eq!(sensor.evaluate(&mut make_ctx("::1"), &compiled), Score(100));
    }

    #[test]
    fn ipv6_no_match() {
        let sensor = IpSensor;
        let compiled = sensor.compile_args(&args(&["::1"]));
        assert_eq!(sensor.evaluate(&mut make_ctx("::2"), &compiled), Score(0));
    }
}
