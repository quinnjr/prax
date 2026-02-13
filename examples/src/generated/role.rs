//! Auto-generated module for Role enum

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Role {
    User,
    Admin,
    Moderator,
}

impl Default for Role {
    fn default() -> Self {
        Self::User
    }
}
