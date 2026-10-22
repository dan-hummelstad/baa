//! Data model + persistence (PLAN.md §4). No gpui, no cef.

/// Identifies a storage profile = one CEF request context (PLAN.md §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ProfileId(pub u64);
