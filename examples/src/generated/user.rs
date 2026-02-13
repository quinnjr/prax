//! Auto-generated module for User model

#[derive(Debug, Clone)]
pub struct User {
    pub id: i32,
    pub email: String,
    pub name: Option<String>,
    pub role: Role,
    pub active: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub posts: Vec<Post>,
}

/// Operations for the User model
pub struct UserOperations<'a, E: prax_query::QueryEngine> {
    engine: &'a E,
}

impl<'a, E: prax_query::QueryEngine> UserOperations<'a, E> {
    pub fn new(engine: &'a E) -> Self {
        Self { engine }
    }

    /// Find many records
    pub fn find_many(&self) -> prax_query::FindManyOperation<'a, E, User> {
        prax_query::FindManyOperation::new(self.engine, "users")
    }

    /// Find a unique record
    pub fn find_unique(&self) -> prax_query::FindUniqueOperation<'a, E, User> {
        prax_query::FindUniqueOperation::new(self.engine, "users")
    }

    /// Find the first matching record
    pub fn find_first(&self) -> prax_query::FindFirstOperation<'a, E, User> {
        prax_query::FindFirstOperation::new(self.engine, "users")
    }

    /// Create a new record
    pub fn create(&self) -> prax_query::CreateOperation<'a, E, User> {
        prax_query::CreateOperation::new(self.engine, "users")
    }

    /// Update a record
    pub fn update(&self) -> prax_query::UpdateOperation<'a, E, User> {
        prax_query::UpdateOperation::new(self.engine, "users")
    }

    /// Delete a record
    pub fn delete(&self) -> prax_query::DeleteOperation<'a, E, User> {
        prax_query::DeleteOperation::new(self.engine, "users")
    }

    /// Count records
    pub fn count(&self) -> prax_query::CountOperation<'a, E> {
        prax_query::CountOperation::new(self.engine, "users")
    }
}
