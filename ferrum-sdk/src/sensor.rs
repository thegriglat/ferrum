use crate::{Plugin, ProviderId, RequestContext, Score};

/// Compiled sensor entry stored in the registry.
///
/// Wraps the provider and its pre-compiled args in a single closure so the
/// registry needs no type parameters or `Any` downcasting.
pub struct SensorEntry {
    pub evaluate: Box<dyn Fn(&mut RequestContext) -> Score + Send + Sync>,
}

/// Typed trait for sensor plugin authors.
///
/// Implement this trait; the engine sees only a [`SensorEntry`] closure.
/// `Args` is computed once at config load time and reused for every request.
pub trait Sensor: Plugin {
    /// Compiled, request-independent configuration for this sensor instance.
    type Args: Send + Sync + 'static;

    /// Parse `args` from the TOML sensor block and return compiled `Args`.
    ///
    /// Called once at startup.  Panics are acceptable here (misconfiguration).
    fn compile_args(&self, args: &crate::toml::Value) -> Self::Args;

    /// Evaluate this sensor against `ctx` using the pre-compiled `args`.
    ///
    /// Returns a [`Score`] in `[0, 100]` where `Score(100)` means "definitely matches".
    fn evaluate(&self, ctx: &mut RequestContext, args: &Self::Args) -> Score;

    /// Convenience: compiles args and wraps `self` in a [`SensorEntry`].
    ///
    /// `own_id` is the sensor's runtime [`ProviderId`] — sensors that write to
    /// [`RequestContext::metadata`] (e.g. `sensor-geo`) may need to store it
    /// in their `Args` and should build the entry manually instead.
    fn into_entry(self, args: &crate::toml::Value, _own_id: ProviderId) -> SensorEntry
    where
        Self: Sized + 'static,
    {
        let compiled = self.compile_args(args);
        SensorEntry {
            evaluate: Box::new(move |ctx| self.evaluate(ctx, &compiled)),
        }
    }
}

/// Registration entry for a sensor plugin.
///
/// Pass to [`ferrum_sdk::register`] inside your plugin's `init()` function.
///
/// # Example
///
/// ```ignore
/// pub fn init() {
///     ferrum_sdk::register(SensorFactory {
///         name: "my-sensor",
///         build: |args, own_id| MySensor.into_entry(args, own_id),
///     });
/// }
/// ```
#[derive(Copy, Clone)]
pub struct SensorFactory {
    /// Plugin name as it appears in `ferrum.toml` under `plugin = "..."`.
    pub name: &'static str,
    /// Builds a [`SensorEntry`] from TOML args.
    ///
    /// `own_id` is the sensor's runtime [`ProviderId`] — pass to
    /// [`Sensor::into_entry`] or store in `Args` if the sensor writes metadata.
    pub build: fn(args: &crate::toml::Value, own_id: ProviderId) -> SensorEntry,
}
