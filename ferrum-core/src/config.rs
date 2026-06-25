use std::collections::HashMap;
use std::sync::Arc;

use rustc_hash::FxHashMap;

use ferrum_sdk::{ProviderId, Score, Terminator};

/// Action taken when the threshold branch of a rule is followed, or after a hook fires.
#[derive(Clone, Debug, PartialEq)]
pub enum CompiledAction {
    /// Evaluate another rule identified by its [`ProviderId`].
    Rule(ProviderId),
    /// Fire a hook, then continue to the hook's declared `next` action.
    Hook(ProviderId),
    /// Execute the terminator identified by its [`ProviderId`].
    Terminator(ProviderId),
    /// Pass the request to the upstream without any action.
    Pass,
}

/// A single compiled rule ready for evaluation.
#[derive(Clone)]
pub struct CompiledRule {
    /// Unique identifier derived from `id` in the config.
    pub id: ProviderId,
    /// Provider (sensor or transformer) whose score is compared against `threshold`.
    pub input: ProviderId,
    /// Score threshold; the rule fires when `score >= threshold`.
    pub threshold: Score,
    /// Action taken when `score >= threshold`.
    pub if_action: CompiledAction,
    /// Action taken when `score < threshold`.
    pub else_action: CompiledAction,
    /// Whether the rule requires the request body to be buffered.
    pub buffer_body: bool,
}

/// A compiled hook node in the action chain.
///
/// The hook fires (calls its plugin's non-blocking trigger), then execution
/// continues with `next`.
#[derive(Clone)]
pub struct CompiledHook {
    /// Unique identifier derived from the hook name.
    pub id: ProviderId,
    /// Action to take after the hook trigger returns.
    pub next: CompiledAction,
}

/// Network and process settings read from the `[server]` TOML section.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Address the proxy listens on.
    pub listen: String,
    /// Default upstream to forward requests to.
    pub upstream: String,
    /// Address the Prometheus metrics server listens on.
    pub metrics_listen: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen: "0.0.0.0:8080".into(),
            upstream: "127.0.0.1:8081".into(),
            metrics_listen: "0.0.0.0:9090".into(),
        }
    }
}

/// Raw TOML representation of the configuration.
mod raw {
    use std::collections::HashMap;

    use serde::Deserialize;

    use ferrum_sdk::Score;

    #[derive(Debug, Deserialize)]
    pub struct Config {
        #[serde(default)]
        pub server: ServerConfig,
        /// Name of the entry-point node (rule id, hook name, or terminator name).
        /// Defaults to the first `[[rules]]` id when omitted.
        pub entry: Option<String>,
        pub sensors: HashMap<String, PluginConfig>,
        #[serde(default)]
        pub transformers: HashMap<String, PluginConfig>,
        #[serde(default)]
        pub terminators: HashMap<String, PluginConfig>,
        #[serde(default)]
        pub hooks: HashMap<String, HookConfig>,
        #[serde(default)]
        pub rules: Vec<RuleConfig>,
    }

    #[derive(Debug, Deserialize)]
    pub struct ServerConfig {
        #[serde(default = "default_listen")]
        pub listen: String,
        #[serde(default = "default_upstream")]
        pub upstream: String,
        #[serde(default = "default_metrics_listen")]
        pub metrics_listen: String,
    }

    fn default_listen() -> String {
        "0.0.0.0:8080".into()
    }
    fn default_upstream() -> String {
        "127.0.0.1:8081".into()
    }
    fn default_metrics_listen() -> String {
        "0.0.0.0:9090".into()
    }

    impl Default for ServerConfig {
        fn default() -> Self {
            Self {
                listen: default_listen(),
                upstream: default_upstream(),
                metrics_listen: default_metrics_listen(),
            }
        }
    }

    #[derive(Debug, Deserialize)]
    pub struct PluginConfig {
        pub plugin: String,
        #[serde(flatten)]
        pub args: toml::Value,
    }

    /// Raw TOML representation of a `[hooks.*]` block.
    #[derive(Debug, Deserialize)]
    pub struct HookConfig {
        pub plugin: String,
        /// Name of the rule, hook, or terminator to continue to after this hook fires.
        pub next: Option<String>,
        #[serde(flatten)]
        pub args: toml::Value,
    }

    #[derive(Debug, Deserialize)]
    pub struct RuleConfig {
        pub id: String,
        pub threshold: Score,
        pub input: String,
        pub if_action: Option<String>,
        pub else_action: Option<String>,
        #[serde(default)]
        pub buffer_body: bool,
    }
}

pub use raw::Config as RawConfig;
pub use raw::HookConfig as RawHookConfig;
pub use raw::PluginConfig as RawPluginConfig;

/// Return type of [`compile_rules`].
pub type CompileResult = (
    FxHashMap<ProviderId, CompiledRule>,
    FxHashMap<ProviderId, CompiledHook>,
    FxHashMap<ProviderId, Arc<dyn Terminator>>,
    HashMap<String, ProviderId>,
    CompiledAction,
    ServerConfig,
);

/// Compiles the raw config into rule, hook, and terminator maps, all keyed by [`ProviderId`].
///
/// `terminator_factory` builds an [`Arc<dyn Terminator>`] from a plugin name and TOML args.
/// It is provided by `ferrum/src/loader.rs` so `ferrum-core` stays free of plugin names.
///
/// Returns:
/// - `FxHashMap<ProviderId, CompiledRule>` — rules keyed by `ProviderId::from(rule.id)`
/// - `FxHashMap<ProviderId, CompiledHook>` — hooks keyed by `ProviderId::from(hook_name)`
/// - `FxHashMap<ProviderId, Arc<dyn Terminator>>` — terminators keyed by `ProviderId::from(name)`
/// - `HashMap<String, ProviderId>` — provider-name → id for sensors and transformers
/// - `CompiledAction` — entry point; resolved from `entry` field or defaults to first rule
/// - `ServerConfig`
///
/// Panics on duplicate provider names, unknown `input`/action references, or when `entry` is
/// omitted and the rules list is empty.
pub fn compile_rules(
    raw: &RawConfig,
    terminator_factory: &impl Fn(&str, &str) -> Arc<dyn Terminator>,
) -> CompileResult {
    assert!(
        raw.entry.is_some() || !raw.rules.is_empty(),
        "config: at least one rule or an explicit 'entry' must be defined"
    );

    // ── Assign ProviderId to every sensor and transformer ─────────────────────
    let mut provider_ids: HashMap<String, ProviderId> = HashMap::new();
    for name in raw.sensors.keys().chain(raw.transformers.keys()) {
        let id = ProviderId::from(name.as_str());
        if provider_ids.insert(name.clone(), id).is_some() {
            panic!("config: duplicate provider name '{name}'");
        }
    }

    // ── Collect hook ids ──────────────────────────────────────────────────────
    let hook_ids: HashMap<String, ProviderId> = raw
        .hooks
        .keys()
        .map(|name| (name.clone(), ProviderId::from(name.as_str())))
        .collect();

    // ── Compile terminators ───────────────────────────────────────────────────
    let mut terminators: FxHashMap<ProviderId, Arc<dyn Terminator>> = FxHashMap::default();
    for (name, cfg) in &raw.terminators {
        let id = ProviderId::from(name.as_str());
        terminators.insert(id, terminator_factory(name, &cfg.plugin));
    }

    // ── Build name → ProviderId index for rules ───────────────────────────────
    let rule_ids: HashMap<String, ProviderId> = raw
        .rules
        .iter()
        .map(|r| (r.id.clone(), ProviderId::from(r.id.as_str())))
        .collect();

    // ── Helper: resolve an action string → CompiledAction ────────────────────
    let mut resolve_action = |action: Option<&String>| -> CompiledAction {
        let name = match action {
            None => return CompiledAction::Pass,
            Some(n) => n,
        };
        if let Some(&id) = rule_ids.get(name) {
            return CompiledAction::Rule(id);
        }
        if let Some(&id) = hook_ids.get(name) {
            return CompiledAction::Hook(id);
        }
        // Try as a named terminator from [terminators.*]
        let term_id = ProviderId::from(name.as_str());
        if terminators.contains_key(&term_id) {
            return CompiledAction::Terminator(term_id);
        }
        // Try as a built-in / inline terminator plugin (e.g. "block", "pass")
        let t = terminator_factory(name, name);
        terminators.entry(term_id).or_insert(t);
        CompiledAction::Terminator(term_id)
    };

    // ── Compile hooks ─────────────────────────────────────────────────────────
    let mut compiled_hooks: FxHashMap<ProviderId, CompiledHook> = FxHashMap::default();
    for (name, cfg) in &raw.hooks {
        let id = ProviderId::from(name.as_str());
        let next = resolve_action(cfg.next.as_ref());
        compiled_hooks.insert(id, CompiledHook { id, next });
    }

    // ── Compile rules ─────────────────────────────────────────────────────────
    let mut rules: FxHashMap<ProviderId, CompiledRule> = FxHashMap::default();
    for r in &raw.rules {
        let id = ProviderId::from(r.id.as_str());
        let input = *provider_ids.get(&r.input).unwrap_or_else(|| {
            panic!(
                "config: rule '{}' references unknown input '{}'",
                r.id, r.input
            )
        });
        let if_action = resolve_action(r.if_action.as_ref());
        let else_action = resolve_action(r.else_action.as_ref());
        rules.insert(
            id,
            CompiledRule {
                id,
                input,
                threshold: r.threshold,
                if_action,
                else_action,
                buffer_body: r.buffer_body,
            },
        );
    }

    let entry = match &raw.entry {
        Some(name) => resolve_action(Some(name)),
        None => CompiledAction::Rule(ProviderId::from(raw.rules[0].id.as_str())),
    };

    let server = ServerConfig {
        listen: raw.server.listen.clone(),
        upstream: raw.server.upstream.clone(),
        metrics_listen: raw.server.metrics_listen.clone(),
    };

    (
        rules,
        compiled_hooks,
        terminators,
        provider_ids,
        entry,
        server,
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use ferrum_sdk::{HttpSession, Plugin, RequestContext, Terminator};

    use super::*;

    struct PassTerminator;

    impl Plugin for PassTerminator {}

    #[async_trait]
    impl Terminator for PassTerminator {
        async fn execute(&self, _ctx: &RequestContext, _session: &mut dyn HttpSession) -> bool {
            false
        }
    }

    fn test_factory(_section: &str, _plugin: &str) -> Arc<dyn Terminator> {
        Arc::new(PassTerminator)
    }

    const SAMPLE_CONFIG: &str = r#"
[sensors.block_ip]
plugin = "sensor-ip"
ips = ["1.2.3.4"]

[sensors.sqli_body]
plugin = "sensor-regex"
patterns = ["(?i)union.*select", "(?i)' or '1'='1"]
target = "body"

[terminators.block_403]
plugin = "block"
status = 403

[[rules]]
id = "check-ip"
input = "block_ip"
threshold = 100
if_action = "block_403"
else_action = "check-sqli"

[[rules]]
id = "check-sqli"
input = "sqli_body"
threshold = 50
if_action = "block_403"
buffer_body = true
"#;

    #[test]
    fn parse_and_compile_sample_config() {
        let raw: RawConfig = toml::from_str(SAMPLE_CONFIG).expect("toml parse failed");
        let (rules, hooks, terminators, provider_ids, entry, server) =
            compile_rules(&raw, &test_factory);

        assert_eq!(provider_ids.len(), 2);
        assert!(provider_ids.contains_key("block_ip"));
        assert!(provider_ids.contains_key("sqli_body"));

        assert_eq!(rules.len(), 2);
        assert!(rules.contains_key(&ProviderId::from("check-ip")));
        assert!(rules.contains_key(&ProviderId::from("check-sqli")));
        assert_eq!(entry, CompiledAction::Rule(ProviderId::from("check-ip")));

        assert!(terminators.contains_key(&ProviderId::from("block_403")));
        assert!(hooks.is_empty());

        assert_eq!(server.listen, "0.0.0.0:8080");
        assert_eq!(server.upstream, "127.0.0.1:8081");
    }

    #[test]
    fn server_section_overrides_defaults() {
        let raw: RawConfig = toml::from_str(
            r#"
[server]
listen = "0.0.0.0:9000"
upstream = "10.0.0.1:80"

[sensors.foo]
plugin = "sensor-ip"
ips = []

[[rules]]
id = "r"
input = "foo"
threshold = 100
if_action = "block"
"#,
        )
        .unwrap();
        let (_, _, _, _, _, server) = compile_rules(&raw, &test_factory);
        assert_eq!(server.listen, "0.0.0.0:9000");
        assert_eq!(server.upstream, "10.0.0.1:80");
    }

    #[test]
    #[should_panic(expected = "unknown input")]
    fn dangling_input_reference_panics() {
        let raw: RawConfig = toml::from_str(
            r#"
[sensors.foo]
plugin = "sensor-ip"
ips = []

[[rules]]
id = "bad"
input = "nonexistent"
threshold = 100
if_action = "block"
"#,
        )
        .unwrap();
        compile_rules(&raw, &test_factory);
    }
}
