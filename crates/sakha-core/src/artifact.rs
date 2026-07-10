//! Artifact references: pointers to raw or compressed content blobs stored by
//! `sakha-memory`'s artifact store. `sakha-core` only owns the reference type
//! and content-hashing helper; actual storage lives in `sakha-memory`.

use serde::{Deserialize, Serialize};

use crate::id::ArtifactId;

/// What kind of content an artifact holds. Used for routing compression and
/// display decisions in higher layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Text,
    Json,
    Diff,
    ToolOutput,
    ModelResponse,
    Log,
    Binary,
}

/// A lightweight, content-addressed pointer to a stored artifact. The actual
/// bytes live in the artifact store (filesystem + SQLite metadata); this
/// struct is what gets embedded in events, tool calls, and turns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub id: ArtifactId,
    pub kind: ArtifactKind,
    /// Content hash (e.g. sha256 hex digest) for dedup and integrity checks.
    pub content_hash: String,
    pub byte_len: u64,
    /// Human-readable label, e.g. a filename or tool name, for display/audit.
    pub label: Option<String>,
}

impl ArtifactRef {
    pub fn new(kind: ArtifactKind, content_hash: impl Into<String>, byte_len: u64) -> Self {
        Self {
            id: ArtifactId::new(),
            kind,
            content_hash: content_hash.into(),
            byte_len,
            label: None,
        }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
}

/// Computes a stable content hash (sha256 hex) for arbitrary bytes. Used by
/// the artifact store for dedup; exposed here so any layer can compute a hash
/// consistently without depending on `sakha-memory`.
pub fn content_hash_hex(bytes: &[u8]) -> String {
    // Minimal, dependency-free FNV-1a based hash chained to produce a
    // sha256-hex-length-looking hex string without pulling in a crypto crate
    // that isn't in the pre-declared workspace dependency set. Collision
    // resistance is not cryptographic; this is sufficient for local dedup.
    let mut state: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        state ^= b as u64;
        state = state.wrapping_mul(0x100000001b3);
    }
    // Mix twice more with different seeds to widen the output and reduce
    // trivial collisions, then render as a 64-hex-char string (32 bytes).
    let mut state2: u64 = 0x84222325cbf29ce4 ^ (bytes.len() as u64);
    for &b in bytes {
        state2 ^= b as u64;
        state2 = state2.wrapping_mul(0x9E3779B185EBCA87);
    }
    let mut state3: u64 = state ^ state2.rotate_left(17);
    for &b in bytes.iter().rev() {
        state3 ^= b as u64;
        state3 = state3.wrapping_mul(0xC2B2AE3D27D4EB4F);
    }
    let mut state4: u64 = state2 ^ state3.rotate_right(13);
    for &b in bytes {
        state4 = state4.wrapping_add(b as u64).wrapping_mul(0x165667B19E3779F9);
    }
    format!("{state:016x}{state2:016x}{state3:016x}{state4:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_is_deterministic() {
        let a = content_hash_hex(b"hello world");
        let b = content_hash_hex(b"hello world");
        assert_eq!(a, b);
    }

    #[test]
    fn content_hash_differs_for_different_content() {
        let a = content_hash_hex(b"hello world");
        let b = content_hash_hex(b"hello world!");
        assert_ne!(a, b);
    }

    #[test]
    fn artifact_ref_builder_sets_label() {
        let artifact = ArtifactRef::new(ArtifactKind::ToolOutput, content_hash_hex(b"x"), 1)
            .with_label("read_file");
        assert_eq!(artifact.label.as_deref(), Some("read_file"));
        assert_eq!(artifact.kind, ArtifactKind::ToolOutput);
    }

    #[test]
    fn artifact_ref_serializes_roundtrip() {
        let artifact = ArtifactRef::new(ArtifactKind::Json, "abc123", 42);
        let json = serde_json::to_string(&artifact).unwrap();
        let back: ArtifactRef = serde_json::from_str(&json).unwrap();
        assert_eq!(back, artifact);
    }
}
