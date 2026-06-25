mod hook;
mod node;
mod score;
mod sensor;
mod terminator;
mod transformer;

pub use hook::{Hook, HookEntry, HookFactory};
pub use node::NodeKind;
pub use score::Score;
pub use sensor::{Sensor, SensorEntry, SensorFactory};
pub use terminator::{HttpSession, Terminator, TerminatorFactory};
pub use transformer::{Evaluator, Transformer, TransformerEntry, TransformerFactory};

pub use inventory;

pub use toml;

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::net::IpAddr;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use bytes::Bytes;
use rustc_hash::{FxHashMap, FxHasher};

/// Stable opaque identifier for a registered provider.
///
/// Derived from the provider name via [`FxHasher`]; unique within a correctly
/// configured registry (duplicate names panic at startup).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProviderId(pub u64);

impl From<&str> for ProviderId {
    /// Derives a [`ProviderId`] from a human-readable provider name.
    ///
    /// Uses [`FxHasher`] for speed; deterministic for the lifetime of the process.
    /// All components that convert a name to an id must use this impl to guarantee
    /// consistent results across crates.
    fn from(name: &str) -> Self {
        let mut h = FxHasher::default();
        name.hash(&mut h);
        ProviderId(h.finish())
    }
}

/// Marker trait for all WAF plugin implementations.
///
/// Supertrait of [`Sensor`], [`Transformer`], [`Hook`], and [`Terminator`].
/// Provides `Send + Sync` bounds and documents the plugin hierarchy.
pub trait Plugin: Send + Sync {}

/// Type-erased plugin factory stored in the global registry.
pub enum PluginFactory {
    Sensor(SensorFactory),
    Transformer(TransformerFactory),
    Hook(HookFactory),
    Terminator(TerminatorFactory),
}

impl PluginFactory {
    /// Returns the plugin name as declared in the factory.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Sensor(f) => f.name,
            Self::Transformer(f) => f.name,
            Self::Hook(f) => f.name,
            Self::Terminator(f) => f.name,
        }
    }

    /// Returns the [`NodeKind`] that corresponds to this factory's plugin type.
    pub fn kind(&self) -> NodeKind {
        match self {
            Self::Sensor(_) => NodeKind::Sensor,
            Self::Transformer(_) => NodeKind::Transformer,
            Self::Hook(_) => NodeKind::Hook,
            Self::Terminator(_) => NodeKind::Terminator,
        }
    }
}

impl From<SensorFactory> for PluginFactory {
    fn from(f: SensorFactory) -> Self {
        PluginFactory::Sensor(f)
    }
}

impl From<TransformerFactory> for PluginFactory {
    fn from(f: TransformerFactory) -> Self {
        PluginFactory::Transformer(f)
    }
}

impl From<HookFactory> for PluginFactory {
    fn from(f: HookFactory) -> Self {
        PluginFactory::Hook(f)
    }
}

impl From<TerminatorFactory> for PluginFactory {
    fn from(f: TerminatorFactory) -> Self {
        PluginFactory::Terminator(f)
    }
}

/// A fully built plugin entry, ready to be stored in the registry.
///
/// Returned by factory `build` functions and registered via
/// [`ferrum_core::registry::PluginRegistry::register_node`].
pub enum NodeEntry {
    /// A compiled sensor.  See [`Sensor`].
    Sensor(SensorEntry),
    /// A compiled transformer.  See [`Transformer`].
    Transformer(TransformerEntry),
    /// A compiled hook.  See [`Hook`].
    Hook(HookEntry),
    /// A compiled terminator.  See [`Terminator`].
    Terminator(std::sync::Arc<dyn Terminator>),
}

inventory::collect!(SensorFactory);
inventory::collect!(TransformerFactory);
inventory::collect!(HookFactory);
inventory::collect!(TerminatorFactory);

/// Registers all plugins submitted via [`inventory::submit!`] into the global registry.
///
/// Call once at binary startup, before loading config.
pub fn init_from_inventory() {
    for f in inventory::iter::<SensorFactory>() {
        register(PluginFactory::Sensor(*f));
    }
    for f in inventory::iter::<TransformerFactory>() {
        register(PluginFactory::Transformer(*f));
    }
    for f in inventory::iter::<HookFactory>() {
        register(PluginFactory::Hook(*f));
    }
    for f in inventory::iter::<TerminatorFactory>() {
        register(PluginFactory::Terminator(f.clone()));
    }
}

pub(crate) fn plugin_registry() -> &'static Mutex<HashMap<&'static str, PluginFactory>> {
    static PLUGINS: OnceLock<Mutex<HashMap<&'static str, PluginFactory>>> = OnceLock::new();
    PLUGINS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Registers a plugin factory into the global registry.
///
/// Accepts any type that converts `Into<`[`PluginFactory`]`>`.  Panics on duplicate names.
pub fn register(factory: impl Into<PluginFactory>) {
    let factory = factory.into();
    let name = factory.name();
    if plugin_registry()
        .lock()
        .unwrap()
        .insert(name, factory)
        .is_some()
    {
        panic!("ferrum-sdk: duplicate plugin name '{name}'");
    }
}

/// Looks up a registered sensor plugin by name.  Returns `None` if not registered.
pub fn get_sensor(name: &str) -> Option<SensorFactory> {
    match plugin_registry().lock().unwrap().get(name)? {
        PluginFactory::Sensor(f) => Some(*f),
        _ => None,
    }
}

/// Looks up a registered transformer plugin by name.  Returns `None` if not registered.
pub fn get_transformer(name: &str) -> Option<TransformerFactory> {
    match plugin_registry().lock().unwrap().get(name)? {
        PluginFactory::Transformer(f) => Some(*f),
        _ => None,
    }
}

/// Looks up a registered hook plugin by name.  Returns `None` if not registered.
pub fn get_hook(name: &str) -> Option<HookFactory> {
    match plugin_registry().lock().unwrap().get(name)? {
        PluginFactory::Hook(f) => Some(HookFactory {
            name: f.name,
            build: f.build,
        }),
        _ => None,
    }
}

/// Looks up a registered terminator plugin by name.  Returns `None` if not registered.
pub fn get_terminator(name: &str) -> Option<TerminatorFactory> {
    match plugin_registry().lock().unwrap().get(name)? {
        PluginFactory::Terminator(f) => Some(f.clone()),
        _ => None,
    }
}

/// Per-request mutable state passed through the rule engine.
///
/// Plugins read from it (IP, URI, headers, body) and may write to
/// `metadata` for downstream consumers.  `cache` is managed by the engine
/// to avoid redundant evaluations of the same provider.
pub struct RequestContext {
    /// IP address of the connecting client.
    pub client_ip: IpAddr,
    /// Request URI (path + query).
    pub uri: String,
    /// HTTP method (e.g. `"GET"`, `"POST"`).
    pub method: String,
    /// Normalised lowercase header map.
    pub headers: FxHashMap<String, String>,
    /// Request body, or `None` if not yet buffered or body exceeded the limit.
    pub body: Option<Bytes>,
    /// Score cache: populated by the engine on first evaluation of each provider.
    pub cache: FxHashMap<ProviderId, Score>,
    /// String metadata written by sensors for use by downstream providers.
    pub metadata: FxHashMap<ProviderId, String>,
    /// Score of the provider whose evaluation triggered the current action.
    /// Updated by the engine before invoking hooks and terminators.
    pub current_score: Score,
}

impl RequestContext {
    /// Creates a minimal context from the three fields available before headers are parsed.
    pub fn new(client_ip: IpAddr, method: String, uri: String) -> Self {
        Self {
            client_ip,
            uri,
            method,
            headers: FxHashMap::default(),
            body: None,
            cache: FxHashMap::default(),
            metadata: FxHashMap::default(),
            current_score: Score(0),
        }
    }
}

/// Abstraction over wall-clock time, enabling deterministic tests.
pub trait Clock: Send + Sync {
    /// Returns the current instant.
    fn now(&self) -> Instant;
}

/// Production clock backed by [`Instant::now`].
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Test clock with a configurable time offset.
pub struct MockClock {
    /// Amount of time added to [`Instant::now`].
    pub offset: Duration,
}

impl Clock for MockClock {
    fn now(&self) -> Instant {
        Instant::now() + self.offset
    }
}
