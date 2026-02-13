//! Filter types for queries

use prax_query::filter::{Filter, ScalarFilter};

/// Filter input for User queries
#[derive(Debug, Default, Clone)]
pub struct UserWhereInput {
    pub id: Option<ScalarFilter<i64>>,
    pub email: Option<ScalarFilter<String>>,
    pub name: Option<ScalarFilter<String>>,
    pub role: Option<ScalarFilter<Role>>,
    pub active: Option<ScalarFilter<bool>>,
    pub created_at: Option<ScalarFilter<chrono::DateTime<chrono::Utc>>>,
    pub updated_at: Option<ScalarFilter<chrono::DateTime<chrono::Utc>>>,
    pub and: Option<Vec<Self>>,
    pub or: Option<Vec<Self>>,
    pub not: Option<Box<Self>>,
}

/// Order by input for User queries
#[derive(Debug, Default, Clone)]
pub struct UserOrderByInput {
    pub id: Option<prax_query::SortOrder>,
    pub email: Option<prax_query::SortOrder>,
    pub name: Option<prax_query::SortOrder>,
    pub role: Option<prax_query::SortOrder>,
    pub active: Option<prax_query::SortOrder>,
    pub created_at: Option<prax_query::SortOrder>,
    pub updated_at: Option<prax_query::SortOrder>,
}

/// Filter input for Post queries
#[derive(Debug, Default, Clone)]
pub struct PostWhereInput {
    pub id: Option<ScalarFilter<i64>>,
    pub title: Option<ScalarFilter<String>>,
    pub content: Option<ScalarFilter<String>>,
    pub status: Option<ScalarFilter<PostStatus>>,
    pub published: Option<ScalarFilter<bool>>,
    pub views: Option<ScalarFilter<i64>>,
    pub created_at: Option<ScalarFilter<chrono::DateTime<chrono::Utc>>>,
    pub updated_at: Option<ScalarFilter<chrono::DateTime<chrono::Utc>>>,
    pub author_id: Option<ScalarFilter<i64>>,
    pub and: Option<Vec<Self>>,
    pub or: Option<Vec<Self>>,
    pub not: Option<Box<Self>>,
}

/// Order by input for Post queries
#[derive(Debug, Default, Clone)]
pub struct PostOrderByInput {
    pub id: Option<prax_query::SortOrder>,
    pub title: Option<prax_query::SortOrder>,
    pub content: Option<prax_query::SortOrder>,
    pub status: Option<prax_query::SortOrder>,
    pub published: Option<prax_query::SortOrder>,
    pub views: Option<prax_query::SortOrder>,
    pub created_at: Option<prax_query::SortOrder>,
    pub updated_at: Option<prax_query::SortOrder>,
    pub author_id: Option<prax_query::SortOrder>,
}

/// Filter input for Comment queries
#[derive(Debug, Default, Clone)]
pub struct CommentWhereInput {
    pub id: Option<ScalarFilter<i64>>,
    pub content: Option<ScalarFilter<String>>,
    pub created_at: Option<ScalarFilter<chrono::DateTime<chrono::Utc>>>,
    pub updated_at: Option<ScalarFilter<chrono::DateTime<chrono::Utc>>>,
    pub author_id: Option<ScalarFilter<i64>>,
    pub post_id: Option<ScalarFilter<i64>>,
    pub and: Option<Vec<Self>>,
    pub or: Option<Vec<Self>>,
    pub not: Option<Box<Self>>,
}

/// Order by input for Comment queries
#[derive(Debug, Default, Clone)]
pub struct CommentOrderByInput {
    pub id: Option<prax_query::SortOrder>,
    pub content: Option<prax_query::SortOrder>,
    pub created_at: Option<prax_query::SortOrder>,
    pub updated_at: Option<prax_query::SortOrder>,
    pub author_id: Option<prax_query::SortOrder>,
    pub post_id: Option<prax_query::SortOrder>,
}

