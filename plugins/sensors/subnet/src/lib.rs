use std::net::IpAddr;

use ferrum_sdk::{Plugin, RequestContext, Score, Sensor};
use ipnet::IpNet;

/// Sensor that scores `1.0` if the client IP falls within any configured CIDR range.
pub struct SubnetSensor;

/// Compiled arguments for [`SubnetSensor`].
pub struct SubnetArgs {
    /// List of subnets to match against.
    pub nets: Vec<IpNet>,
}

impl Plugin for SubnetSensor {}

impl Sensor for SubnetSensor {
    type Args = SubnetArgs;

    fn compile_args(&self, args: &ferrum_sdk::toml::Value) -> SubnetArgs {
        let cidrs = args
            .get("cidrs")
            .and_then(|v| v.as_array())
            .expect("sensor-subnet: missing 'cidrs' array");

        let nets = cidrs
            .iter()
            .map(|v| {
                v.as_str()
                    .expect("sensor-subnet: each entry in 'cidrs' must be a string")
                    .parse::<IpNet>()
                    .expect("sensor-subnet: invalid CIDR notation")
            })
            .collect();

        SubnetArgs { nets }
    }

    fn evaluate(&self, ctx: &mut RequestContext, args: &SubnetArgs) -> Score {
        let ip: IpAddr = ctx.client_ip;
        args.nets.iter().any(|net| net.contains(&ip)).into()
    }
}

ferrum_sdk::inventory::submit! {
    ferrum_sdk::SensorFactory {
        name: "sensor-subnet",
        build: |args, own_id| SubnetSensor.into_entry(args, own_id),
    }
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use ferrum_sdk::{RequestContext, Score, Sensor};

    use super::SubnetSensor;

    fn make_ctx(ip: &str) -> RequestContext {
        RequestContext::new(ip.parse::<IpAddr>().unwrap(), "GET".into(), "/".into())
    }

    fn args(cidrs: &[&str]) -> ferrum_sdk::toml::Value {
        let list: toml::value::Array = cidrs
            .iter()
            .map(|s| ferrum_sdk::toml::Value::String(s.to_string()))
            .collect();
        let mut t = toml::value::Table::new();
        t.insert("cidrs".into(), ferrum_sdk::toml::Value::Array(list));
        ferrum_sdk::toml::Value::Table(t)
    }

    #[test]
    fn ipv4_inside() {
        let sensor = SubnetSensor;
        let compiled = sensor.compile_args(&args(&["192.168.1.0/24"]));
        assert_eq!(
            sensor.evaluate(&mut make_ctx("192.168.1.100"), &compiled),
            Score(100)
        );
    }

    #[test]
    fn ipv4_outside() {
        let sensor = SubnetSensor;
        let compiled = sensor.compile_args(&args(&["192.168.1.0/24"]));
        assert_eq!(
            sensor.evaluate(&mut make_ctx("192.168.2.1"), &compiled),
            Score(0)
        );
    }

    #[test]
    fn ipv4_network_address() {
        let sensor = SubnetSensor;
        let compiled = sensor.compile_args(&args(&["10.0.0.0/8"]));
        assert_eq!(
            sensor.evaluate(&mut make_ctx("10.0.0.0"), &compiled),
            Score(100)
        );
    }

    #[test]
    fn ipv4_broadcast_address() {
        let sensor = SubnetSensor;
        let compiled = sensor.compile_args(&args(&["10.0.0.0/8"]));
        assert_eq!(
            sensor.evaluate(&mut make_ctx("10.255.255.255"), &compiled),
            Score(100)
        );
    }

    #[test]
    fn ipv6_inside() {
        let sensor = SubnetSensor;
        let compiled = sensor.compile_args(&args(&["2001:db8::/32"]));
        assert_eq!(
            sensor.evaluate(&mut make_ctx("2001:db8::1"), &compiled),
            Score(100)
        );
    }

    #[test]
    fn ipv6_outside() {
        let sensor = SubnetSensor;
        let compiled = sensor.compile_args(&args(&["2001:db8::/32"]));
        assert_eq!(
            sensor.evaluate(&mut make_ctx("2001:db9::1"), &compiled),
            Score(0)
        );
    }
}
