use std::collections::HashMap;
use std::sync::Arc;

use ferrum_sdk::{
    Evaluator, HookEntry, NodeEntry, NodeKind, ProviderId, RequestContext, Score, SensorEntry,
    Terminator, TransformerEntry,
};

/// Type-tagged provider instance stored in the registry.
enum Entry {
    Sensor(SensorEntry),
    Transformer(TransformerEntry),
    Hook(HookEntry),
    Terminator(Arc<dyn Terminator>),
}

/// Central registry mapping [`ProviderId`] to compiled plugin instances.
///
/// Stores all node types: sensors, transformers, hooks, and terminators.
pub struct PluginRegistry {
    entries: HashMap<ProviderId, Entry>,
}

impl PluginRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    fn insert(&mut self, id: ProviderId, entry: Entry) {
        if self.entries.contains_key(&id) {
            panic!("registry: duplicate ProviderId {id:?}");
        }
        self.entries.insert(id, entry);
    }

    /// Registers any node entry by kind.  Panics on duplicate id.
    pub fn register_node(&mut self, id: ProviderId, entry: NodeEntry) {
        match entry {
            NodeEntry::Sensor(e) => self.insert(id, Entry::Sensor(e)),
            NodeEntry::Transformer(e) => self.insert(id, Entry::Transformer(e)),
            NodeEntry::Hook(e) => self.insert(id, Entry::Hook(e)),
            NodeEntry::Terminator(e) => self.insert(id, Entry::Terminator(e)),
        }
    }

    /// Registers a compiled sensor entry.  Panics on duplicate id.
    pub fn register_sensor(&mut self, id: ProviderId, entry: SensorEntry) {
        self.insert(id, Entry::Sensor(entry));
    }

    /// Registers a compiled transformer entry.  Panics on duplicate id.
    pub fn register_transformer(&mut self, id: ProviderId, entry: TransformerEntry) {
        self.insert(id, Entry::Transformer(entry));
    }

    /// Registers a compiled hook entry.  Panics on duplicate id.
    pub fn register_hook(&mut self, id: ProviderId, entry: HookEntry) {
        self.insert(id, Entry::Hook(entry));
    }

    /// Returns the [`NodeKind`] of the registered entry for `id`.
    ///
    /// Panics if `id` is not registered.
    pub fn node_kind(&self, id: ProviderId) -> NodeKind {
        match self.entries.get(&id) {
            Some(Entry::Sensor(_)) => NodeKind::Sensor,
            Some(Entry::Transformer(_)) => NodeKind::Transformer,
            Some(Entry::Hook(_)) => NodeKind::Hook,
            Some(Entry::Terminator(_)) => NodeKind::Terminator,
            None => panic!("registry: unknown ProviderId {id:?}"),
        }
    }

    /// Returns the dependency [`ProviderId`]s declared by transformer `id`.
    ///
    /// Returns an empty `Vec` for sensors.
    /// Panics if `id` is not registered or is a hook/terminator (not valid rule inputs).
    pub fn dep_ids(&self, id: ProviderId) -> Vec<ProviderId> {
        match self.entries.get(&id) {
            Some(Entry::Sensor(_)) => vec![],
            Some(Entry::Transformer(e)) => e.dep_ids.clone(),
            Some(Entry::Hook(_)) => panic!("registry: hook {id:?} cannot be used as a rule input"),
            Some(Entry::Terminator(_)) => {
                panic!("registry: terminator {id:?} cannot be used as a rule input")
            }
            None => panic!("registry: unknown ProviderId {id:?}"),
        }
    }

    /// Evaluates sensor `id` against `ctx`, without caching.
    ///
    /// Panics if `id` is not registered or is not a sensor.
    pub fn evaluate_sensor(&self, id: ProviderId, ctx: &mut RequestContext) -> Score {
        match self.entries.get(&id) {
            Some(Entry::Sensor(e)) => (e.evaluate)(ctx),
            _ => panic!("registry: {id:?} is not a sensor"),
        }
    }

    /// Evaluates transformer `id` using pre-computed scores from `eval`.
    ///
    /// Panics if `id` is not registered or is not a transformer.
    pub fn evaluate_transformer(&self, id: ProviderId, eval: &dyn Evaluator) -> Score {
        match self.entries.get(&id) {
            Some(Entry::Transformer(e)) => (e.evaluate)(eval),
            _ => panic!("registry: {id:?} is not a transformer"),
        }
    }

    /// Fires the hook registered under `id` with a read-only view of `ctx`.
    ///
    /// Non-blocking: the hook's trigger closure sends an event to a channel;
    /// actual I/O happens in a background worker.
    /// Panics if `id` is not registered or is not a hook.
    pub fn trigger_hook(&self, id: ProviderId, ctx: &RequestContext) {
        match self.entries.get(&id) {
            Some(Entry::Hook(e)) => (e.trigger)(ctx),
            _ => panic!("registry: {id:?} is not a hook"),
        }
    }

    /// Returns the terminator registered under `id`, or `None` if not found.
    pub fn get_terminator(&self, id: ProviderId) -> Option<Arc<dyn Terminator>> {
        match self.entries.get(&id) {
            Some(Entry::Terminator(t)) => Some(Arc::clone(t)),
            _ => None,
        }
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}
