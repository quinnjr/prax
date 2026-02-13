//! Auto-generated module for PostStatus enum

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PostStatus {
    Draft,
    Published,
    Archived,
}

impl Default for PostStatus {
    fn default() -> Self {
        Self::Draft
    }
}
