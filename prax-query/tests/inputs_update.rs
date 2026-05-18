use prax_query::inputs::{
    BoolFieldUpdate, IntFieldUpdate, IntNullableFieldUpdate, StringFieldUpdate,
    StringNullableFieldUpdate,
};

#[test]
fn int_field_update_from_scalar_shortcut() {
    let u: IntFieldUpdate = 5i32.into();
    assert_eq!(u.set, Some(5));
    assert!(u.increment.is_none());
}

#[test]
fn int_field_update_increment_and_set_keeps_both() {
    let u = IntFieldUpdate {
        set: Some(0),
        increment: Some(1),
        ..Default::default()
    };
    assert_eq!(u.set, Some(0));
    assert_eq!(u.increment, Some(1));
}

#[test]
fn string_nullable_field_update_unset_marker() {
    let u = StringNullableFieldUpdate {
        unset: Some(true),
        ..Default::default()
    };
    assert_eq!(u.unset, Some(true));
}

#[test]
fn string_field_update_from_scalar_shortcut() {
    let u: StringFieldUpdate = "Alice".into();
    assert_eq!(u.set.as_deref(), Some("Alice"));
}

#[test]
fn bool_field_update_from_scalar_shortcut() {
    let u: BoolFieldUpdate = true.into();
    assert_eq!(u.set, Some(true));
}

#[test]
fn int_nullable_field_update_unset_marker() {
    let u = IntNullableFieldUpdate {
        unset: Some(true),
        ..Default::default()
    };
    assert_eq!(u.unset, Some(true));
}
