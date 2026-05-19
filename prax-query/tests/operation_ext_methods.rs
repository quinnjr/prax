use prax_query::filter::{Filter, FilterValue};
use prax_query::inputs::{StringFilter, WhereInput};
use prax_query::operations::{FindFirstOperation, FindManyOperation, FindUniqueOperation};
use prax_query::traits::{BoxFuture, Model, QueryEngine};

struct U;
impl Model for U {
    const MODEL_NAME: &'static str = "U";
    const TABLE_NAME: &'static str = "users";
    const PRIMARY_KEY: &'static [&'static str] = &["id"];
    const COLUMNS: &'static [&'static str] = &["id", "email"];
}
impl prax_query::row::FromRow for U {
    fn from_row(_row: &impl prax_query::row::RowRef) -> Result<Self, prax_query::row::RowError> {
        Ok(U)
    }
}

#[derive(Clone)]
struct NoopEngine;
impl QueryEngine for NoopEngine {
    fn query_many<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        _: &str,
        _: Vec<FilterValue>,
    ) -> BoxFuture<'_, prax_query::error::QueryResult<Vec<T>>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn query_one<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        _: &str,
        _: Vec<FilterValue>,
    ) -> BoxFuture<'_, prax_query::error::QueryResult<T>> {
        Box::pin(async { Err(prax_query::error::QueryError::not_found("t")) })
    }
    fn query_optional<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        _: &str,
        _: Vec<FilterValue>,
    ) -> BoxFuture<'_, prax_query::error::QueryResult<Option<T>>> {
        Box::pin(async { Ok(None) })
    }
    fn execute_insert<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        _: &str,
        _: Vec<FilterValue>,
    ) -> BoxFuture<'_, prax_query::error::QueryResult<T>> {
        Box::pin(async { Err(prax_query::error::QueryError::not_found("t")) })
    }
    fn execute_update<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        _: &str,
        _: Vec<FilterValue>,
    ) -> BoxFuture<'_, prax_query::error::QueryResult<Vec<T>>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn execute_delete(
        &self,
        _: &str,
        _: Vec<FilterValue>,
    ) -> BoxFuture<'_, prax_query::error::QueryResult<u64>> {
        Box::pin(async { Ok(0) })
    }
    fn execute_raw(
        &self,
        _: &str,
        _: Vec<FilterValue>,
    ) -> BoxFuture<'_, prax_query::error::QueryResult<u64>> {
        Box::pin(async { Ok(0) })
    }
    fn count(
        &self,
        _: &str,
        _: Vec<FilterValue>,
    ) -> BoxFuture<'_, prax_query::error::QueryResult<u64>> {
        Box::pin(async { Ok(0) })
    }
}

#[derive(Default, Clone)]
struct UWhereInput {
    pub email: Option<StringFilter>,
}
impl WhereInput for UWhereInput {
    type Model = U;
    fn into_ir(self) -> Filter {
        use prax_query::inputs::ScalarFilter;
        match self.email {
            Some(f) => f.into_filter("email"),
            None => Filter::None,
        }
    }
}

impl prax_query::inputs::WhereUniqueInput for UWhereInput {
    type Model = U;
    fn into_ir(self) -> Filter {
        use prax_query::inputs::ScalarFilter;
        match self.email {
            Some(f) => f.into_filter("email"),
            None => Filter::None,
        }
    }
}

#[test]
fn find_many_with_where_input_replaces_filter_when_first() {
    let op = FindManyOperation::<NoopEngine, U>::new(NoopEngine).with_where_input(UWhereInput {
        email: Some(StringFilter::contains("@x.com")),
    });
    assert!(!matches!(op.filter_for_test(), Filter::None));
}

#[test]
fn find_many_with_where_input_ands_with_existing() {
    let op = FindManyOperation::<NoopEngine, U>::new(NoopEngine)
        .r#where(Filter::Equals("id".into(), FilterValue::Int(1)))
        .with_where_input(UWhereInput {
            email: Some(StringFilter::contains("@x.com")),
        });
    match op.filter_for_test() {
        Filter::And(parts) => assert_eq!(parts.len(), 2),
        other => panic!("expected Filter::And, got {:?}", other),
    }
}

#[test]
fn find_unique_with_where_input_overwrites_filter() {
    let op = FindUniqueOperation::<NoopEngine, U>::new(NoopEngine).with_where_input(UWhereInput {
        email: Some(StringFilter::equals("x@y.com")),
    });
    assert!(!matches!(op.filter_for_test(), Filter::None));
}

#[test]
fn find_first_with_where_input_ands_with_existing() {
    let op = FindFirstOperation::<NoopEngine, U>::new(NoopEngine)
        .r#where(Filter::Equals("id".into(), FilterValue::Int(1)))
        .with_where_input(UWhereInput {
            email: Some(StringFilter::contains("@x.com")),
        });
    match op.filter_for_test() {
        Filter::And(parts) => assert_eq!(parts.len(), 2),
        other => panic!("expected Filter::And, got {:?}", other),
    }
}
