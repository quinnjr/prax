use prax_query::filter::{Filter, FilterValue};
use prax_query::inputs::{
    WhereInput,
    relation::{ListRelationFilter, LowerRelationFilter, RelationFilterMeta, SingleRelationFilter},
};
use prax_query::traits::Model;

struct Post;
impl Model for Post {
    const MODEL_NAME: &'static str = "Post";
    const TABLE_NAME: &'static str = "posts";
    const PRIMARY_KEY: &'static [&'static str] = &["id"];
    const COLUMNS: &'static [&'static str] = &["id", "author_id", "published"];
}

#[derive(Default, Clone)]
struct PostWhereInput {
    pub published: Option<prax_query::inputs::BoolFilter>,
}
impl WhereInput for PostWhereInput {
    type Model = Post;
    fn into_ir(self) -> Filter {
        use prax_query::inputs::ScalarFilter;
        match self.published {
            Some(f) => f.into_filter("published"),
            None => Filter::None,
        }
    }
}

// Hand-built relation meta for `User.posts` so we don't need the codegen.
struct UserPostsMeta;
impl RelationFilterMeta for UserPostsMeta {
    const PARENT_TABLE: &'static str = "users";
    const PARENT_PK: &'static str = "id";
    const CHILD_TABLE: &'static str = "posts";
    const CHILD_FK: &'static str = "author_id";
}

#[test]
fn list_relation_some_lowers_to_exists_scalar_subquery() {
    let rf = ListRelationFilter {
        some: Some(PostWhereInput {
            published: Some(prax_query::inputs::BoolFilter::equals(true)),
        }),
        ..Default::default()
    };
    let filter = rf.lower::<UserPostsMeta>();
    match filter {
        Filter::ScalarSubquery { sql, params } => {
            assert!(sql.contains("EXISTS"));
            assert!(sql.contains("posts"));
            assert!(sql.contains("author_id"));
            // The inner filter pulls `published = $?` into the subquery.
            assert!(params.iter().any(|p| matches!(p, FilterValue::Bool(true))));
        }
        other => panic!("expected Filter::ScalarSubquery, got {:?}", other),
    }
}

#[test]
fn list_relation_none_lowers_to_not_exists() {
    let rf = ListRelationFilter::<PostWhereInput> {
        none: Some(PostWhereInput {
            published: Some(prax_query::inputs::BoolFilter::equals(true)),
        }),
        ..Default::default()
    };
    let filter = rf.lower::<UserPostsMeta>();
    match filter {
        Filter::ScalarSubquery { sql, .. } => assert!(sql.starts_with("NOT EXISTS")),
        other => panic!("expected NOT EXISTS subquery, got {:?}", other),
    }
}

#[test]
fn list_relation_every_lowers_to_not_exists_negated() {
    let rf = ListRelationFilter::<PostWhereInput> {
        every: Some(PostWhereInput {
            published: Some(prax_query::inputs::BoolFilter::equals(true)),
        }),
        ..Default::default()
    };
    let filter = rf.lower::<UserPostsMeta>();
    // `every: F` == `NOT EXISTS (child WHERE parent.pk = child.fk AND NOT (F))`.
    match filter {
        Filter::ScalarSubquery { sql, .. } => {
            assert!(sql.starts_with("NOT EXISTS"));
            assert!(sql.contains("NOT ("));
        }
        other => panic!("expected NOT EXISTS subquery, got {:?}", other),
    }
}

#[test]
fn single_relation_is_lowers_to_exists() {
    let rf = SingleRelationFilter::<PostWhereInput> {
        is: Some(PostWhereInput {
            published: Some(prax_query::inputs::BoolFilter::equals(true)),
        }),
        ..Default::default()
    };
    let filter = rf.lower::<UserPostsMeta>();
    match filter {
        Filter::ScalarSubquery { sql, .. } => assert!(sql.starts_with("EXISTS")),
        other => panic!("expected EXISTS subquery, got {:?}", other),
    }
}
