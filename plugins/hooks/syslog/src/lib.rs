use std::sync::mpsc::SyncSender;

use ferrum_sdk::{Hook, Plugin, RequestContext};
use syslog::{Facility, Formatter3164};

ferrum_sdk::inventory::submit! {
    ferrum_sdk::HookFactory {
        name: "hook-syslog",
        build: |args| SyslogHook.into_entry(args),
    }
}

struct SyslogHook;

impl Plugin for SyslogHook {}

/// Compiled args for `hook-syslog`.
///
/// The sender end of a bounded channel; the background thread owns the syslog writer.
pub struct SyslogArgs {
    sender: SyncSender<String>,
}

impl Hook for SyslogHook {
    type Args = SyslogArgs;

    fn compile_args(&self, args: &ferrum_sdk::toml::Value) -> Self::Args {
        let facility_str = args
            .get("facility")
            .and_then(|v| v.as_str())
            .unwrap_or("daemon");
        let ident = args
            .get("ident")
            .and_then(|v| v.as_str())
            .unwrap_or("ferrum")
            .to_string();
        let severity = args
            .get("severity")
            .and_then(|v| v.as_str())
            .unwrap_or("info")
            .to_string();

        let facility = parse_facility(facility_str);
        let (tx, rx) = std::sync::mpsc::sync_channel::<String>(4096);

        std::thread::spawn(move || {
            let formatter = Formatter3164 {
                facility,
                hostname: None,
                process: ident,
                pid: 0,
            };
            let mut writer =
                syslog::unix(formatter).expect("hook-syslog: failed to open syslog socket");
            for msg in rx {
                let _ = match severity.as_str() {
                    "notice" => writer.notice(msg),
                    "warning" | "warn" => writer.warning(msg),
                    "err" | "error" => writer.err(msg),
                    _ => writer.info(msg),
                };
            }
        });

        SyslogArgs { sender: tx }
    }

    /// Non-blocking: formats the log line and sends it to the background thread.
    fn trigger(&self, ctx: &RequestContext, args: &Self::Args) {
        let ua = ctx
            .headers
            .get("user-agent")
            .map(|s| s.as_str())
            .unwrap_or("-");
        let msg = format!("{} {} {} ua=\"{}\"", ctx.method, ctx.uri, ctx.client_ip, ua);
        let _ = args.sender.try_send(msg);
    }
}

fn parse_facility(s: &str) -> Facility {
    match s {
        "kern" => Facility::LOG_KERN,
        "user" => Facility::LOG_USER,
        "mail" => Facility::LOG_MAIL,
        "auth" => Facility::LOG_AUTH,
        "syslog" => Facility::LOG_SYSLOG,
        "lpr" => Facility::LOG_LPR,
        "news" => Facility::LOG_NEWS,
        "cron" => Facility::LOG_CRON,
        "local0" => Facility::LOG_LOCAL0,
        "local1" => Facility::LOG_LOCAL1,
        "local2" => Facility::LOG_LOCAL2,
        "local3" => Facility::LOG_LOCAL3,
        "local4" => Facility::LOG_LOCAL4,
        "local5" => Facility::LOG_LOCAL5,
        "local6" => Facility::LOG_LOCAL6,
        "local7" => Facility::LOG_LOCAL7,
        _ => Facility::LOG_DAEMON,
    }
}
