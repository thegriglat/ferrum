/// Identifies the role of a node in the provider DAG.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// Corresponds to [`crate::Sensor`].
    Sensor,
    /// Corresponds to [`crate::Transformer`].
    Transformer,
    /// Corresponds to [`crate::Hook`].
    Hook,
    /// Corresponds to [`crate::Terminator`].
    Terminator,
}
