use prax_query::inputs::{
    BoolFieldUpdate, BytesFieldUpdate, IntFieldUpdate, IntNullableFieldUpdate, StringFieldUpdate,
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

#[test]
fn bytes_field_update_json_stays_base64() {
    let u: BytesFieldUpdate = serde_json::from_str(r#"{"set":"AP8="}"#).unwrap();
    assert_eq!(u.set, Some(vec![0x00, 0xFF]));
    assert_eq!(serde_json::to_string(&u).unwrap(), r#"{"set":"AP8="}"#);
}

#[test]
fn bytes_nullable_update_absent_and_null_are_none() {
    use prax_query::inputs::BytesNullableFieldUpdate;
    let absent: BytesNullableFieldUpdate = serde_json::from_str(r#"{"unset":true}"#).unwrap();
    assert_eq!(absent.set, None);
    let null: BytesNullableFieldUpdate = serde_json::from_str(r#"{"set":null}"#).unwrap();
    assert_eq!(null.set, None);
    assert!(serde_json::from_str::<BytesNullableFieldUpdate>(r#"{"set":"!!"}"#).is_err());
}
