use prax_query::filter::Filter;
use prax_query::inputs::{CreateArgs, FindManyArgs, FindUniqueArgs, UpsertArgs};
use prax_query::traits::Model;

struct TestModel;
impl Model for TestModel {
    const MODEL_NAME: &'static str = "TestModel";
    const TABLE_NAME: &'static str = "test_models";
    const PRIMARY_KEY: &'static [&'static str] = &["id"];
    const COLUMNS: &'static [&'static str] = &["id"];
}

#[test]
fn find_many_args_default_is_empty() {
    let a: FindManyArgs<TestModel, Filter, (), ()> = FindManyArgs::default();
    assert!(a.r#where.is_none());
    assert!(a.include.is_none());
    assert!(a.select.is_none());
    assert!(a.order_by.is_none());
    assert!(a.cursor.is_none());
    assert_eq!(a.skip, None);
    assert_eq!(a.take, None);
}

#[test]
fn find_unique_args_carries_unique_filter() {
    let a: FindUniqueArgs<TestModel, Filter, (), ()> = FindUniqueArgs {
        r#where: Filter::None,
        include: None,
        select: None,
        _model: std::marker::PhantomData,
    };
    assert!(matches!(a.r#where, Filter::None));
}

#[test]
fn create_args_carries_data() {
    let a: CreateArgs<TestModel, (), (), ()> = CreateArgs {
        data: (),
        include: None,
        select: None,
        _model: std::marker::PhantomData,
    };
    assert_eq!(std::mem::size_of_val(&a.data), 0);
}

#[test]
fn upsert_args_round_trip() {
    let a: UpsertArgs<TestModel, Filter, (), (), (), ()> = UpsertArgs {
        r#where: Filter::None,
        create: (),
        update: (),
        include: None,
        select: None,
        _model: std::marker::PhantomData,
    };
    assert!(matches!(a.r#where, Filter::None));
}
