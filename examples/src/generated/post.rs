//! Auto-generated module for Post model

#[derive(Debug, Clone)]
pub struct Post {
    pub id: i32,
    pub title: String,
    pub content: Option<String>,
    pub status: PostStatus,
    pub published: bool,
    pub views: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub author_id: i32,
    pub author: User,
}

/// Operations for the Post model
pub struct PostOperations<'a, E: prax_query::QueryEngine> {
    engine: &'a E,
}

impl<'a, E: prax_query::QueryEngine> PostOperations<'a, E> {
    pub fn new(engine: &'a E) -> Self {
        Self { engine }
    }

    /// Find many records
    pub fn find_many(&self) -> prax_query::FindManyOperation<'a, E, Post> {
        prax_query::FindManyOperation::new(self.engine, "posts")
    }

    /// Find a unique record
    pub fn find_unique(&self) -> prax_query::FindUniqueOperation<'a, E, Post> {
        prax_query::FindUniqueOperation::new(self.engine, "posts")
    }

    /// Find the first matching record
    pub fn find_first(&self) -> prax_query::FindFirstOperation<'a, E, Post> {
        prax_query::FindFirstOperation::new(self.engine, "posts")
    }

    /// Create a new record
    pub fn create(&self) -> prax_query::CreateOperation<'a, E, Post> {
        prax_query::CreateOperation::new(self.engine, "posts")
    }

    /// Update a record
    pub fn update(&self) -> prax_query::UpdateOperation<'a, E, Post> {
        prax_query::UpdateOperation::new(self.engine, "posts")
    }

    /// Delete a record
    pub fn delete(&self) -> prax_query::DeleteOperation<'a, E, Post> {
        prax_query::DeleteOperation::new(self.engine, "posts")
    }

    /// Count records
    pub fn count(&self) -> prax_query::CountOperation<'a, E> {
        prax_query::CountOperation::new(self.engine, "posts")
    }
}
