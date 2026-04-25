//! Attribute definitions for the Prax schema AST.

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use super::{Ident, Span};

/// An attribute argument value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AttributeValue {
    /// A string literal.
    String(String),
    /// An integer literal.
    Int(i64),
    /// A float literal.
    Float(f64),
    /// A boolean literal.
    Boolean(bool),
    /// An identifier/constant reference (e.g., enum value).
    Ident(SmolStr),
    /// A function call (e.g., `now()`, `uuid()`).
    Function(SmolStr, Vec<AttributeValue>),
    /// An array of values.
    Array(Vec<AttributeValue>),
    /// A field reference (e.g., `[field_name]`).
    FieldRef(SmolStr),
    /// A list of field references (e.g., `[field1, field2]`).
    FieldRefList(Vec<SmolStr>),
}

impl AttributeValue {
    /// Try to get the value as a string.
    pub fn as_string(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }

    /// Try to get the value as an integer.
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// Try to get the value as a float.
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Self::Float(f) => Some(*f),
            Self::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    /// Try to get the value as a boolean.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    /// Try to get the value as an identifier.
    pub fn as_ident(&self) -> Option<&str> {
        match self {
            Self::Ident(s) => Some(s),
            _ => None,
        }
    }
}

/// An attribute argument (named or positional).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttributeArg {
    /// Argument name (None for positional arguments).
    pub name: Option<Ident>,
    /// Argument value.
    pub value: AttributeValue,
    /// Source location.
    pub span: Span,
}

impl AttributeArg {
    /// Create a positional argument.
    pub fn positional(value: AttributeValue, span: Span) -> Self {
        Self {
            name: None,
            value,
            span,
        }
    }

    /// Create a named argument.
    pub fn named(name: Ident, value: AttributeValue, span: Span) -> Self {
        Self {
            name: Some(name),
            value,
            span,
        }
    }

    /// Check if this is a positional argument.
    pub fn is_positional(&self) -> bool {
        self.name.is_none()
    }
}

/// An attribute applied to a field, model, or enum.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attribute {
    /// Attribute name (without `@` prefix).
    pub name: Ident,
    /// Attribute arguments.
    pub args: Vec<AttributeArg>,
    /// Source location (including `@`).
    pub span: Span,
}

impl Attribute {
    /// Create a new attribute.
    pub fn new(name: Ident, args: Vec<AttributeArg>, span: Span) -> Self {
        Self { name, args, span }
    }

    /// Create an attribute with no arguments.
    pub fn simple(name: Ident, span: Span) -> Self {
        Self {
            name,
            args: vec![],
            span,
        }
    }

    /// Get the attribute name as a string.
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    /// Check if this attribute has the given name.
    pub fn is(&self, name: &str) -> bool {
        self.name.as_str() == name
    }

    /// Get the first positional argument.
    pub fn first_arg(&self) -> Option<&AttributeValue> {
        self.args.first().map(|a| &a.value)
    }

    /// Get a named argument by name.
    pub fn get_arg(&self, name: &str) -> Option<&AttributeValue> {
        self.args
            .iter()
            .find(|a| a.name.as_ref().map(|n| n.as_str()) == Some(name))
            .map(|a| &a.value)
    }

    /// Check if this is a field-level attribute.
    pub fn is_field_attribute(&self) -> bool {
        matches!(
            self.name(),
            "id" | "auto"
                | "unique"
                | "index"
                | "default"
                | "updated_at"
                | "omit"
                | "map"
                | "db"
                | "relation"
        )
    }

    /// Check if this is a model-level attribute (prefixed with `@@`).
    pub fn is_model_attribute(&self) -> bool {
        matches!(
            self.name(),
            "map" | "index" | "unique" | "id" | "search" | "sql"
        )
    }
}

/// Common field attributes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FieldAttributes {
    /// This field is the primary key.
    pub is_id: bool,
    /// This field auto-increments (for integer IDs).
    pub is_auto: bool,
    /// This field has a unique constraint.
    pub is_unique: bool,
    /// This field is indexed.
    pub is_indexed: bool,
    /// This field is updated automatically on record update.
    pub is_updated_at: bool,
    /// This field should be omitted from default selections.
    pub is_omit: bool,
    /// Default value expression.
    pub default: Option<AttributeValue>,
    /// Database column name mapping.
    pub map: Option<String>,
    /// Native database type (e.g., `@db.VarChar(255)`).
    pub native_type: Option<NativeType>,
    /// Relation attributes.
    pub relation: Option<RelationAttribute>,
}

/// Native database type specification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeType {
    /// Type name (e.g., "VarChar", "Text", "Decimal").
    pub name: SmolStr,
    /// Type arguments (e.g., length, precision, scale).
    pub args: Vec<AttributeValue>,
}

impl NativeType {
    /// Create a new native type.
    pub fn new(name: impl Into<SmolStr>, args: Vec<AttributeValue>) -> Self {
        Self {
            name: name.into(),
            args,
        }
    }
}

/// Relation attribute details.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationAttribute {
    /// Relation name (for disambiguation).
    pub name: Option<String>,
    /// Fields on this model that reference the other model.
    pub fields: Vec<SmolStr>,
    /// Fields on the other model being referenced.
    pub references: Vec<SmolStr>,
    /// On delete action.
    pub on_delete: Option<ReferentialAction>,
    /// On update action.
    pub on_update: Option<ReferentialAction>,
    /// Custom foreign key constraint name in the database.
    pub map: Option<String>,
}

/// Referential actions for relations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReferentialAction {
    /// Cascade the operation.
    Cascade,
    /// Restrict the operation (error if references exist).
    Restrict,
    /// No action (deferred check).
    NoAction,
    /// Set to null.
    SetNull,
    /// Set to default value.
    SetDefault,
}

impl ReferentialAction {
    /// Parse from string.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "Cascade" => Some(Self::Cascade),
            "Restrict" => Some(Self::Restrict),
            "NoAction" => Some(Self::NoAction),
            "SetNull" => Some(Self::SetNull),
            "SetDefault" => Some(Self::SetDefault),
            _ => None,
        }
    }

    /// Get the action name.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cascade => "CASCADE",
            Self::Restrict => "RESTRICT",
            Self::NoAction => "NO ACTION",
            Self::SetNull => "SET NULL",
            Self::SetDefault => "SET DEFAULT",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== AttributeValue Tests ====================

    #[test]
    fn test_attribute_value_string() {
        let val = AttributeValue::String("hello".into());
        assert_eq!(val.as_string(), Some("hello"));
        assert_eq!(val.as_int(), None);
        assert_eq!(val.as_bool(), None);
        assert_eq!(val.as_ident(), None);
    }

    #[test]
    fn test_attribute_value_int() {
        let val = AttributeValue::Int(42);
        assert_eq!(val.as_int(), Some(42));
        assert_eq!(val.as_string(), None);
        assert_eq!(val.as_bool(), None);
    }

    #[test]
    fn test_attribute_value_int_negative() {
        let val = AttributeValue::Int(-100);
        assert_eq!(val.as_int(), Some(-100));
    }

    #[test]
    #[allow(clippy::approx_constant)]
    fn test_attribute_value_float() {
        let val = AttributeValue::Float(3.14);
        assert_eq!(val.as_int(), None);
        assert_eq!(val.as_string(), None);
    }

    #[test]
    fn test_attribute_value_boolean_true() {
        let val = AttributeValue::Boolean(true);
        assert_eq!(val.as_bool(), Some(true));
        assert_eq!(val.as_int(), None);
    }

    #[test]
    fn test_attribute_value_boolean_false() {
        let val = AttributeValue::Boolean(false);
        assert_eq!(val.as_bool(), Some(false));
    }

    #[test]
    fn test_attribute_value_ident() {
        let val = AttributeValue::Ident("MyEnum".into());
        assert_eq!(val.as_ident(), Some("MyEnum"));
        assert_eq!(val.as_string(), None);
    }

    #[test]
    fn test_attribute_value_function() {
        let val = AttributeValue::Function("now".into(), vec![]);
        assert_eq!(val.as_string(), None);
        assert_eq!(val.as_int(), None);

        // Check it's the right variant
        if let AttributeValue::Function(name, args) = val {
            assert_eq!(name.as_str(), "now");
            assert!(args.is_empty());
        } else {
            panic!("Expected Function variant");
        }
    }

    #[test]
    fn test_attribute_value_function_with_args() {
        let val = AttributeValue::Function("uuid".into(), vec![AttributeValue::Int(4)]);

        if let AttributeValue::Function(name, args) = val {
            assert_eq!(name.as_str(), "uuid");
            assert_eq!(args.len(), 1);
        } else {
            panic!("Expected Function variant");
        }
    }

    #[test]
    fn test_attribute_value_array() {
        let val = AttributeValue::Array(vec![
            AttributeValue::String("a".into()),
            AttributeValue::String("b".into()),
        ]);

        if let AttributeValue::Array(items) = val {
            assert_eq!(items.len(), 2);
        } else {
            panic!("Expected Array variant");
        }
    }

    #[test]
    fn test_attribute_value_field_ref() {
        let val = AttributeValue::FieldRef("user_id".into());
        assert_eq!(val.as_ident(), None);

        if let AttributeValue::FieldRef(name) = val {
            assert_eq!(name.as_str(), "user_id");
        } else {
            panic!("Expected FieldRef variant");
        }
    }

    #[test]
    fn test_attribute_value_field_ref_list() {
        let val = AttributeValue::FieldRefList(vec!["id".into(), "name".into()]);

        if let AttributeValue::FieldRefList(fields) = val {
            assert_eq!(fields.len(), 2);
            assert_eq!(fields[0].as_str(), "id");
            assert_eq!(fields[1].as_str(), "name");
        } else {
            panic!("Expected FieldRefList variant");
        }
    }

    #[test]
    fn test_attribute_value_equality() {
        let val1 = AttributeValue::Int(42);
        let val2 = AttributeValue::Int(42);
        let val3 = AttributeValue::Int(43);

        assert_eq!(val1, val2);
        assert_ne!(val1, val3);
    }

    // ==================== AttributeArg Tests ====================

    #[test]
    fn test_attribute_arg_positional() {
        let arg = AttributeArg::positional(AttributeValue::Int(42), Span::new(0, 2));

        assert!(arg.is_positional());
        assert!(arg.name.is_none());
        assert_eq!(arg.value.as_int(), Some(42));
    }

    #[test]
    fn test_attribute_arg_named() {
        let arg = AttributeArg::named(
            Ident::new("length", Span::new(0, 6)),
            AttributeValue::Int(255),
            Span::new(0, 10),
        );

        assert!(!arg.is_positional());
        assert!(arg.name.is_some());
        assert_eq!(arg.name.as_ref().unwrap().as_str(), "length");
        assert_eq!(arg.value.as_int(), Some(255));
    }

    #[test]
    fn test_attribute_arg_equality() {
        let arg1 = AttributeArg::positional(AttributeValue::Int(42), Span::new(0, 2));
        let arg2 = AttributeArg::positional(AttributeValue::Int(42), Span::new(0, 2));
        let arg3 = AttributeArg::positional(AttributeValue::Int(43), Span::new(0, 2));

        assert_eq!(arg1, arg2);
        assert_ne!(arg1, arg3);
    }

    // ==================== Attribute Tests ====================

    #[test]
    fn test_attribute_new() {
        let attr = Attribute::new(
            Ident::new("default", Span::new(0, 7)),
            vec![AttributeArg::positional(
                AttributeValue::Int(0),
                Span::new(8, 9),
            )],
            Span::new(0, 10),
        );

        assert_eq!(attr.name(), "default");
        assert_eq!(attr.args.len(), 1);
    }

    #[test]
    fn test_attribute_simple() {
        let attr = Attribute::simple(Ident::new("id", Span::new(0, 2)), Span::new(0, 3));

        assert_eq!(attr.name(), "id");
        assert!(attr.args.is_empty());
    }

    #[test]
    fn test_attribute_is() {
        let attr = Attribute::simple(Ident::new("unique", Span::new(0, 6)), Span::new(0, 7));

        assert!(attr.is("unique"));
        assert!(!attr.is("id"));
        assert!(!attr.is("UNIQUE")); // case sensitive
    }

    #[test]
    fn test_attribute_first_arg() {
        let attr = Attribute::new(
            Ident::new("default", Span::new(0, 7)),
            vec![
                AttributeArg::positional(AttributeValue::Int(42), Span::new(8, 10)),
                AttributeArg::positional(AttributeValue::String("extra".into()), Span::new(12, 19)),
            ],
            Span::new(0, 20),
        );

        assert_eq!(attr.first_arg().unwrap().as_int(), Some(42));
    }

    #[test]
    fn test_attribute_first_arg_none() {
        let attr = Attribute::simple(Ident::new("id", Span::new(0, 2)), Span::new(0, 3));
        assert!(attr.first_arg().is_none());
    }

    #[test]
    fn test_attribute_get_arg() {
        let attr = Attribute::new(
            Ident::new("relation", Span::new(0, 8)),
            vec![
                AttributeArg::named(
                    Ident::new("fields", Span::new(9, 15)),
                    AttributeValue::FieldRefList(vec!["user_id".into()]),
                    Span::new(9, 30),
                ),
                AttributeArg::named(
                    Ident::new("references", Span::new(32, 42)),
                    AttributeValue::FieldRefList(vec!["id".into()]),
                    Span::new(32, 50),
                ),
            ],
            Span::new(0, 51),
        );

        let fields = attr.get_arg("fields").unwrap();
        if let AttributeValue::FieldRefList(f) = fields {
            assert_eq!(f[0].as_str(), "user_id");
        } else {
            panic!("Expected FieldRefList");
        }

        assert!(attr.get_arg("onDelete").is_none());
    }

    #[test]
    fn test_attribute_is_field_attribute() {
        let id_attr = Attribute::simple(Ident::new("id", Span::new(0, 2)), Span::new(0, 3));
        let auto_attr = Attribute::simple(Ident::new("auto", Span::new(0, 4)), Span::new(0, 5));
        let unique_attr = Attribute::simple(Ident::new("unique", Span::new(0, 6)), Span::new(0, 7));
        let index_attr = Attribute::simple(Ident::new("index", Span::new(0, 5)), Span::new(0, 6));
        let default_attr =
            Attribute::simple(Ident::new("default", Span::new(0, 7)), Span::new(0, 8));
        let updated_at_attr =
            Attribute::simple(Ident::new("updated_at", Span::new(0, 10)), Span::new(0, 11));
        let omit_attr = Attribute::simple(Ident::new("omit", Span::new(0, 4)), Span::new(0, 5));
        let map_attr = Attribute::simple(Ident::new("map", Span::new(0, 3)), Span::new(0, 4));
        let db_attr = Attribute::simple(Ident::new("db", Span::new(0, 2)), Span::new(0, 3));
        let relation_attr =
            Attribute::simple(Ident::new("relation", Span::new(0, 8)), Span::new(0, 9));
        let unknown_attr =
            Attribute::simple(Ident::new("unknown", Span::new(0, 7)), Span::new(0, 8));

        assert!(id_attr.is_field_attribute());
        assert!(auto_attr.is_field_attribute());
        assert!(unique_attr.is_field_attribute());
        assert!(index_attr.is_field_attribute());
        assert!(default_attr.is_field_attribute());
        assert!(updated_at_attr.is_field_attribute());
        assert!(omit_attr.is_field_attribute());
        assert!(map_attr.is_field_attribute());
        assert!(db_attr.is_field_attribute());
        assert!(relation_attr.is_field_attribute());
        assert!(!unknown_attr.is_field_attribute());
    }

    #[test]
    fn test_attribute_is_model_attribute() {
        let map_attr = Attribute::simple(Ident::new("map", Span::new(0, 3)), Span::new(0, 4));
        let index_attr = Attribute::simple(Ident::new("index", Span::new(0, 5)), Span::new(0, 6));
        let unique_attr = Attribute::simple(Ident::new("unique", Span::new(0, 6)), Span::new(0, 7));
        let id_attr = Attribute::simple(Ident::new("id", Span::new(0, 2)), Span::new(0, 3));
        let search_attr = Attribute::simple(Ident::new("search", Span::new(0, 6)), Span::new(0, 7));
        let sql_attr = Attribute::simple(Ident::new("sql", Span::new(0, 3)), Span::new(0, 4));
        let unknown_attr =
            Attribute::simple(Ident::new("unknown", Span::new(0, 7)), Span::new(0, 8));

        assert!(map_attr.is_model_attribute());
        assert!(index_attr.is_model_attribute());
        assert!(unique_attr.is_model_attribute());
        assert!(id_attr.is_model_attribute());
        assert!(search_attr.is_model_attribute());
        assert!(sql_attr.is_model_attribute());
        assert!(!unknown_attr.is_model_attribute());
    }

    // ==================== FieldAttributes Tests ====================

    #[test]
    fn test_field_attributes_default() {
        let attrs = FieldAttributes::default();

        assert!(!attrs.is_id);
        assert!(!attrs.is_auto);
        assert!(!attrs.is_unique);
        assert!(!attrs.is_indexed);
        assert!(!attrs.is_updated_at);
        assert!(!attrs.is_omit);
        assert!(attrs.default.is_none());
        assert!(attrs.map.is_none());
        assert!(attrs.native_type.is_none());
        assert!(attrs.relation.is_none());
    }

    #[test]
    fn test_field_attributes_with_values() {
        let attrs = FieldAttributes {
            is_id: true,
            is_auto: true,
            is_unique: false,
            is_indexed: false,
            is_updated_at: false,
            is_omit: false,
            default: Some(AttributeValue::Function("auto".into(), vec![])),
            map: Some("user_id".to_string()),
            native_type: None,
            relation: None,
        };

        assert!(attrs.is_id);
        assert!(attrs.is_auto);
        assert!(attrs.default.is_some());
        assert_eq!(attrs.map, Some("user_id".to_string()));
    }

    // ==================== NativeType Tests ====================

    #[test]
    fn test_native_type_new() {
        let nt = NativeType::new("VarChar", vec![AttributeValue::Int(255)]);

        assert_eq!(nt.name.as_str(), "VarChar");
        assert_eq!(nt.args.len(), 1);
        assert_eq!(nt.args[0].as_int(), Some(255));
    }

    #[test]
    fn test_native_type_no_args() {
        let nt = NativeType::new("Text", vec![]);

        assert_eq!(nt.name.as_str(), "Text");
        assert!(nt.args.is_empty());
    }

    #[test]
    fn test_native_type_multiple_args() {
        let nt = NativeType::new(
            "Decimal",
            vec![AttributeValue::Int(10), AttributeValue::Int(2)],
        );

        assert_eq!(nt.name.as_str(), "Decimal");
        assert_eq!(nt.args.len(), 2);
    }

    #[test]
    fn test_native_type_equality() {
        let nt1 = NativeType::new("VarChar", vec![AttributeValue::Int(255)]);
        let nt2 = NativeType::new("VarChar", vec![AttributeValue::Int(255)]);
        let nt3 = NativeType::new("VarChar", vec![AttributeValue::Int(100)]);

        assert_eq!(nt1, nt2);
        assert_ne!(nt1, nt3);
    }

    // ==================== RelationAttribute Tests ====================

    #[test]
    fn test_relation_attribute() {
        let rel = RelationAttribute {
            name: Some("UserPosts".to_string()),
            fields: vec!["author_id".into()],
            references: vec!["id".into()],
            on_delete: Some(ReferentialAction::Cascade),
            on_update: Some(ReferentialAction::Cascade),
            map: Some("fk_post_author".to_string()),
        };

        assert_eq!(rel.name, Some("UserPosts".to_string()));
        assert_eq!(rel.fields[0].as_str(), "author_id");
        assert_eq!(rel.references[0].as_str(), "id");
        assert_eq!(rel.on_delete, Some(ReferentialAction::Cascade));
        assert_eq!(rel.map, Some("fk_post_author".to_string()));
    }

    #[test]
    fn test_relation_attribute_minimal() {
        let rel = RelationAttribute {
            name: None,
            fields: vec![],
            references: vec![],
            on_delete: None,
            on_update: None,
            map: None,
        };

        assert!(rel.name.is_none());
        assert!(rel.fields.is_empty());
        assert!(rel.map.is_none());
    }

    // ==================== ReferentialAction Tests ====================

    #[test]
    fn test_referential_action_from_str_cascade() {
        assert_eq!(
            ReferentialAction::from_str("Cascade"),
            Some(ReferentialAction::Cascade)
        );
    }

    #[test]
    fn test_referential_action_from_str_restrict() {
        assert_eq!(
            ReferentialAction::from_str("Restrict"),
            Some(ReferentialAction::Restrict)
        );
    }

    #[test]
    fn test_referential_action_from_str_no_action() {
        assert_eq!(
            ReferentialAction::from_str("NoAction"),
            Some(ReferentialAction::NoAction)
        );
    }

    #[test]
    fn test_referential_action_from_str_set_null() {
        assert_eq!(
            ReferentialAction::from_str("SetNull"),
            Some(ReferentialAction::SetNull)
        );
    }

    #[test]
    fn test_referential_action_from_str_set_default() {
        assert_eq!(
            ReferentialAction::from_str("SetDefault"),
            Some(ReferentialAction::SetDefault)
        );
    }

    #[test]
    fn test_referential_action_from_str_unknown() {
        assert_eq!(ReferentialAction::from_str("Unknown"), None);
        assert_eq!(ReferentialAction::from_str("cascade"), None); // case sensitive
        assert_eq!(ReferentialAction::from_str(""), None);
    }

    #[test]
    fn test_referential_action_as_str() {
        assert_eq!(ReferentialAction::Cascade.as_str(), "CASCADE");
        assert_eq!(ReferentialAction::Restrict.as_str(), "RESTRICT");
        assert_eq!(ReferentialAction::NoAction.as_str(), "NO ACTION");
        assert_eq!(ReferentialAction::SetNull.as_str(), "SET NULL");
        assert_eq!(ReferentialAction::SetDefault.as_str(), "SET DEFAULT");
    }

    #[test]
    fn test_referential_action_equality() {
        assert_eq!(ReferentialAction::Cascade, ReferentialAction::Cascade);
        assert_ne!(ReferentialAction::Cascade, ReferentialAction::Restrict);
    }

    #[test]
    fn test_referential_action_copy() {
        let action = ReferentialAction::Cascade;
        let copy = action;
        assert_eq!(action, copy);
    }
}
