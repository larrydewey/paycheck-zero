//! Entity identifier, a newtype over the UUID text representation.

use serde::{Deserialize, Serialize};
use std::fmt;

/// An opaque entity identifier (UUID stored as text).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct Id(String);

impl Id {
    /// Wraps an existing identifier string.
    #[must_use]
    pub fn new(s: impl Into<String>) -> Self {
        Id(s.into())
    }

    /// Generates a fresh random v4 identifier.
    #[must_use]
    pub fn generate() -> Self {
        Id(uuid::Uuid::new_v4().to_string())
    }

    /// The identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for Id {
    fn from(s: String) -> Self {
        Id(s)
    }
}

impl From<&str> for Id {
    fn from(s: &str) -> Self {
        Id(s.to_string())
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_is_unique() {
        let a = Id::generate();
        let b = Id::generate();
        assert_ne!(a, b);
    }

    #[test]
    fn roundtrips_through_string() {
        let id: Id = "abc".into();
        assert_eq!(id.as_str(), "abc");
        assert_eq!(id.to_string(), "abc");
    }
}
