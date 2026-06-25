/// A score in the range `[0, 100]`.
///
/// Use [`From<f32>`] to convert a fractional weight in `[0.0, 1.0]`
/// (e.g. `Score::from(0.75f32)` → `Score(75)`).
/// Use [`From<bool>`] for binary sensors (`true` → `Score(100)`, `false` → `Score(0)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, serde::Deserialize)]
pub struct Score(pub u8);

impl Score {
    /// Fuzzy AND: `min(self, other)`.
    pub fn and(self, other: Score) -> Score {
        Score(self.0.min(other.0))
    }

    /// Fuzzy OR: `max(self, other)`.
    pub fn or(self, other: Score) -> Score {
        Score(self.0.max(other.0))
    }
}

impl From<u8> for Score {
    fn from(v: u8) -> Self {
        Score(v)
    }
}

impl From<i64> for Score {
    /// Converts an integer to a score, clamping to `[0, 100]`.
    fn from(v: i64) -> Self {
        Score(v.clamp(0, 100) as u8)
    }
}

impl From<f32> for Score {
    /// Converts a fractional weight in `[0.0, 1.0]` to a score in `[0, 100]`.
    ///
    /// Values outside `[0.0, 1.0]` are clamped before conversion.
    fn from(v: f32) -> Self {
        Score((v.clamp(0.0, 1.0) * 100.0).round() as u8)
    }
}

impl From<bool> for Score {
    fn from(v: bool) -> Self {
        if v { Score(100) } else { Score(0) }
    }
}

impl From<Score> for u8 {
    fn from(s: Score) -> Self {
        s.0
    }
}

impl From<Score> for f32 {
    fn from(s: Score) -> Self {
        s.0 as f32
    }
}

impl std::ops::Not for Score {
    type Output = Score;

    /// Fuzzy NOT: `100 - self`.
    fn not(self) -> Score {
        Score(100u8.saturating_sub(self.0))
    }
}

impl From<f64> for Score {
    /// Converts a fractional weight in `[0.0, 1.0]` to a score in `[0, 100]`.
    ///
    /// Values outside `[0.0, 1.0]` are clamped before conversion.
    fn from(v: f64) -> Self {
        Score((v.clamp(0.0, 1.0) * 100.0).round() as u8)
    }
}

impl From<Score> for f64 {
    fn from(s: Score) -> Self {
        s.0 as f64
    }
}

impl std::fmt::Display for Score {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
