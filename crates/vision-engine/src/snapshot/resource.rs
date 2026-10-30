//! An image resource attached to a snapshot.

use serde::{Deserialize, Serialize};

/// An image or other binary resource keyed by its original URL.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Resource {
    /// The original `src` or `href`.
    pub url: String,
    /// Raw bytes.
    pub bytes: Vec<u8>,
}
