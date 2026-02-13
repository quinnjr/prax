//! Auto-generated module for Comment model

#[derive(Debug, Clone)]
pub struct Comment {
    pub id: i32,
    pub content: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub author_id: Option<i32>,
    pub author: Option<User>,
    pub post_id: i32,
    pub post: Post,
}

/// Operations for the Comment model
pub struct CommentOperations<'a, E: prax_query::QueryEngine> {
    engine: &'a E,
}

impl<'a, E: prax_query::QueryEngine> CommentOperations<'a, E> {
    pub fn new(engine: &'a E) -> Self {
        Self { engine }
    }

    /// Find many records
    pub fn find_many(&self) -> prax_query::FindManyOperation<'a, E, Comment> {
        prax_query::FindManyOperation::new(self.engine, "comments")
    }

    /// Find a unique record
    pub fn find_unique(&self) -> prax_query::FindUniqueOperation<'a, E, Comment> {
        prax_query::FindUniqueOperation::new(self.engine, "comments")
    }

    /// Find the first matching record
    pub fn find_first(&self) -> prax_query::FindFirstOperation<'a, E, Comment> {
        prax_query::FindFirstOperation::new(self.engine, "comments")
    }

    /// Create a new record
    pub fn create(&self) -> prax_query::CreateOperation<'a, E, Comment> {
        prax_query::CreateOperation::new(self.engine, "comments")
    }

    /// Update a record
    pub fn update(&self) -> prax_query::UpdateOperation<'a, E, Comment> {
        prax_query::UpdateOperation::new(self.engine, "comments")
    }

    /// Delete a record
    pub fn delete(&self) -> prax_query::DeleteOperation<'a, E, Comment> {
        prax_query::DeleteOperation::new(self.engine, "comments")
    }

    /// Count records
    pub fn count(&self) -> prax_query::CountOperation<'a, E> {
        prax_query::CountOperation::new(self.engine, "comments")
    }
}
