use std::sync::Arc;

use rustc_hash::FxHashMap;

use ferrum_core::config::{
    CompiledAction, CompiledHook, CompiledRule, RawConfig, ServerConfig, compile_rules,
};
use ferrum_core::registry::PluginRegistry;
use ferrum_sdk::{
    HookEntry, NodeEntry, NodeKind, ProviderId, SensorEntry, Terminator, TransformerEntry,
};

/// Everything the proxy needs at runtime.
pub struct FerrumConfig {
    pub rules: FxHashMap<ProviderId, CompiledRule>,
    pub hooks: FxHashMap<ProviderId, CompiledHook>,
    /// Entry point of the DAG — may be a rule, hook, or terminator.
    pub entry: CompiledAction,
    pub registry: PluginRegistry,
    pub server: ServerConfig,
}

/// Loads and compiles a WAF config from `path`.
///
/// Panics on parse errors, unknown plugin names, duplicate providers, or dangling refs.
pub fn load_config(path: &str) -> FerrumConfig {
    let raw_str = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read config '{path}': {e}"));
    let raw: RawConfig =
        toml::from_str(&raw_str).unwrap_or_else(|e| panic!("failed to parse config '{path}': {e}"));

    let terminator_factory = |section: &str, plugin: &str| -> Arc<dyn Terminator> {
        let args = raw
            .terminators
            .get(section)
            .map(|c| c.args.clone())
            .unwrap_or(toml::Value::Table(Default::default()));
        resolve_terminator_plugin(plugin, &args)
    };

    let (rules, hooks, terminators, provider_ids, entry, server) =
        compile_rules(&raw, &terminator_factory);

    let mut registry = PluginRegistry::new();

    // Register terminators from compile_rules output
    for (id, arc) in terminators {
        registry.register_node(id, NodeEntry::Terminator(arc));
    }

    // Unified registration for sensors, transformers, hooks
    let all_providers = raw
        .sensors
        .iter()
        .map(|(n, c)| (n.as_str(), NodeKind::Sensor, c.plugin.as_str(), &c.args))
        .chain(raw.transformers.iter().map(|(n, c)| {
            (
                n.as_str(),
                NodeKind::Transformer,
                c.plugin.as_str(),
                &c.args,
            )
        }))
        .chain(
            raw.hooks
                .iter()
                .map(|(n, c)| (n.as_str(), NodeKind::Hook, c.plugin.as_str(), &c.args)),
        );

    for (name, kind, plugin, args) in all_providers {
        let id = *provider_ids.get(name).unwrap();
        registry.register_node(id, build_node_entry(kind, plugin, args, id));
    }

    FerrumConfig {
        rules,
        hooks,
        entry,
        registry,
        server,
    }
}

fn build_node_entry(kind: NodeKind, plugin: &str, args: &toml::Value, id: ProviderId) -> NodeEntry {
    match kind {
        NodeKind::Sensor => NodeEntry::Sensor(resolve_sensor_plugin(plugin, args, id)),
        NodeKind::Transformer => NodeEntry::Transformer(resolve_transformer_plugin(plugin, args)),
        NodeKind::Hook => NodeEntry::Hook(resolve_hook_plugin(plugin, args)),
        NodeKind::Terminator => {
            unreachable!("terminators are registered from compile_rules output")
        }
    }
}

fn resolve_sensor_plugin(name: &str, args: &toml::Value, own_id: ProviderId) -> SensorEntry {
    let factory =
        ferrum_sdk::get_sensor(name).unwrap_or_else(|| panic!("unknown sensor plugin '{name}'"));
    (factory.build)(args, own_id)
}

fn resolve_transformer_plugin(name: &str, args: &toml::Value) -> TransformerEntry {
    let factory = ferrum_sdk::get_transformer(name)
        .unwrap_or_else(|| panic!("unknown transformer plugin '{name}'"));
    (factory.build)(args)
}

fn resolve_hook_plugin(name: &str, args: &toml::Value) -> HookEntry {
    let factory =
        ferrum_sdk::get_hook(name).unwrap_or_else(|| panic!("unknown hook plugin '{name}'"));
    (factory.build)(args)
}

fn resolve_terminator_plugin(name: &str, args: &toml::Value) -> Arc<dyn Terminator> {
    let factory = ferrum_sdk::get_terminator(name)
        .unwrap_or_else(|| panic!("unknown terminator plugin '{name}'"));
    (factory.build)(args)
}
