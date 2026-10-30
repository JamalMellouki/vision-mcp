//! Reporting of features that were parsed but not implemented.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// A record of unsupported or ignored page features.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CompatReport {
    /// Unsupported properties, at-rules, selectors, or elements.
    pub unsupported: BTreeSet<String>,
    /// Number of discarded CSS rules.
    pub ignored_rules: usize,
    /// Non-fatal parsing notes.
    pub warnings: Vec<String>,
}

impl CompatReport {
    /// Records an unsupported feature once.
    pub fn unsupported(&mut self, feature: impl Into<String>) {
        self.unsupported.insert(feature.into());
    }

    /// Records a parsing warning.
    pub fn warn(&mut self, message: impl Into<String>) {
        self.warnings.push(message.into());
    }

    /// Returns true when no issues were recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.unsupported.is_empty() && self.ignored_rules == 0 && self.warnings.is_empty()
    }
}
