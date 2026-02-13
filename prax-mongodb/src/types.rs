//! Type conversions for MongoDB/BSON.

use bson::{Bson, Document, oid::ObjectId};
use prax_query::filter::FilterValue;

use crate::error::{MongoError, MongoResult};

/// Convert a FilterValue to BSON.
pub fn filter_value_to_bson(value: &FilterValue) -> MongoResult<Bson> {
    match value {
        FilterValue::Null => Ok(Bson::Null),
        FilterValue::Bool(b) => Ok(Bson::Boolean(*b)),
        FilterValue::Int(i) => Ok(Bson::Int64(*i)),
        FilterValue::Float(f) => Ok(Bson::Double(*f)),
        FilterValue::String(s) => {
            // Check if string is an ObjectId
            if s.len() == 24 && s.chars().all(|c| c.is_ascii_hexdigit()) {
                if let Ok(oid) = ObjectId::parse_str(s) {
                    return Ok(Bson::ObjectId(oid));
                }
            }
            Ok(Bson::String(s.clone()))
        }
        FilterValue::Json(j) => {
            // Convert JSON value to BSON
            let bson = bson::to_bson(j).map_err(|e| {
                MongoError::serialization(format!("failed to convert JSON to BSON: {}", e))
            })?;
            Ok(bson)
        }
        FilterValue::List(list) => {
            let bson_values: Result<Vec<Bson>, _> = list.iter().map(filter_value_to_bson).collect();
            Ok(Bson::Array(bson_values?))
        }
    }
}

/// Convert filter values to BSON array.
pub fn filter_values_to_bson(values: &[FilterValue]) -> MongoResult<Vec<Bson>> {
    values.iter().map(filter_value_to_bson).collect()
}

/// MongoDB/BSON type mapping utilities.
pub mod mongo_types {
    /// Get the BSON type for a Rust type name.
    pub fn rust_type_to_bson(rust_type: &str) -> Option<&'static str> {
        match rust_type {
            "i8" | "i16" | "i32" => Some("int"),
            "i64" => Some("long"),
            "f32" | "f64" => Some("double"),
            "bool" => Some("bool"),
            "String" | "&str" => Some("string"),
            "Vec<u8>" | "&[u8]" => Some("binData"),
            "chrono::NaiveDate" | "chrono::NaiveDateTime" | "chrono::DateTime<chrono::Utc>" => {
                Some("date")
            }
            "uuid::Uuid" => Some("binData"), // UUID subtype
            "serde_json::Value" => Some("object"),
            "bson::oid::ObjectId" => Some("objectId"),
            "rust_decimal::Decimal" => Some("decimal"),
            _ => None,
        }
    }

    /// Get the Rust type for a BSON type.
    pub fn bson_type_to_rust(bson_type: &str) -> &'static str {
        match bson_type {
            "double" => "f64",
            "string" => "String",
            "object" => "Document",
            "array" => "Vec<Bson>",
            "binData" => "Vec<u8>",
            "objectId" => "bson::oid::ObjectId",
            "bool" => "bool",
            "date" => "chrono::DateTime<chrono::Utc>",
            "null" => "Option<()>",
            "int" => "i32",
            "long" => "i64",
            "decimal" => "rust_decimal::Decimal",
            "timestamp" => "bson::Timestamp",
            "regex" => "bson::Regex",
            _ => "Bson",
        }
    }

    /// Get the Prax schema type for a BSON type.
    pub fn bson_type_to_prax(bson_type: &str) -> &'static str {
        match bson_type {
            "double" => "Float",
            "string" => "String",
            "object" => "Json",
            "array" => "List",
            "binData" => "Bytes",
            "objectId" => "String", // ObjectId maps to String ID in Prax
            "bool" => "Boolean",
            "date" => "DateTime",
            "null" => "Null",
            "int" => "Int",
            "long" => "BigInt",
            "decimal" => "Decimal",
            _ => "Unknown",
        }
    }

    /// Get the BSON type string for a Prax schema type.
    pub fn prax_type_to_bson(prax_type: &str) -> &'static str {
        match prax_type {
            "Boolean" => "bool",
            "Int" => "int",
            "BigInt" => "long",
            "Float" => "double",
            "Decimal" => "decimal",
            "String" => "string",
            "Bytes" => "binData",
            "Date" => "date",
            "Time" => "string", // MongoDB has no native time type
            "DateTime" => "date",
            "Uuid" => "binData", // UUID binary subtype
            "Json" => "object",
            _ => "string",
        }
    }
}

/// Aggregation pipeline stage helpers.
pub mod aggregation {
    use super::*;

    /// Create a $match stage.
    pub fn match_stage(filter: Document) -> Document {
        bson::doc! { "$match": filter }
    }

    /// Create a $project stage.
    pub fn project_stage(projection: Document) -> Document {
        bson::doc! { "$project": projection }
    }

    /// Create a $group stage.
    pub fn group_stage(id: Bson, accumulators: Document) -> Document {
        let mut stage = bson::doc! { "_id": id };
        stage.extend(accumulators);
        bson::doc! { "$group": stage }
    }

    /// Create a $sort stage.
    pub fn sort_stage(sort: Document) -> Document {
        bson::doc! { "$sort": sort }
    }

    /// Create a $limit stage.
    pub fn limit_stage(limit: i64) -> Document {
        bson::doc! { "$limit": limit }
    }

    /// Create a $skip stage.
    pub fn skip_stage(skip: i64) -> Document {
        bson::doc! { "$skip": skip }
    }

    /// Create a $lookup stage (left join).
    pub fn lookup_stage(
        from: &str,
        local_field: &str,
        foreign_field: &str,
        as_field: &str,
    ) -> Document {
        bson::doc! {
            "$lookup": {
                "from": from,
                "localField": local_field,
                "foreignField": foreign_field,
                "as": as_field
            }
        }
    }

    /// Create a $unwind stage.
    pub fn unwind_stage(path: &str, preserve_null_and_empty: bool) -> Document {
        bson::doc! {
            "$unwind": {
                "path": format!("${}", path),
                "preserveNullAndEmptyArrays": preserve_null_and_empty
            }
        }
    }

    /// Create a $count stage.
    pub fn count_stage(field_name: &str) -> Document {
        bson::doc! { "$count": field_name }
    }

    /// Create a $addFields stage.
    pub fn add_fields_stage(fields: Document) -> Document {
        bson::doc! { "$addFields": fields }
    }

    /// Create a $set stage (alias for $addFields in 4.2+).
    pub fn set_stage(fields: Document) -> Document {
        bson::doc! { "$set": fields }
    }

    /// Create a $unset stage (remove fields).
    pub fn unset_stage(fields: Vec<&str>) -> Document {
        bson::doc! { "$unset": fields }
    }

    /// Create a $replaceRoot stage.
    pub fn replace_root_stage(new_root: Bson) -> Document {
        bson::doc! { "$replaceRoot": { "newRoot": new_root } }
    }

    /// Accumulator: $sum
    pub fn sum(expression: impl Into<Bson>) -> Document {
        bson::doc! { "$sum": expression.into() }
    }

    /// Accumulator: $avg
    pub fn avg(expression: impl Into<Bson>) -> Document {
        bson::doc! { "$avg": expression.into() }
    }

    /// Accumulator: $min
    pub fn min(expression: impl Into<Bson>) -> Document {
        bson::doc! { "$min": expression.into() }
    }

    /// Accumulator: $max
    pub fn max(expression: impl Into<Bson>) -> Document {
        bson::doc! { "$max": expression.into() }
    }

    /// Accumulator: $first
    pub fn first(expression: impl Into<Bson>) -> Document {
        bson::doc! { "$first": expression.into() }
    }

    /// Accumulator: $last
    pub fn last(expression: impl Into<Bson>) -> Document {
        bson::doc! { "$last": expression.into() }
    }

    /// Accumulator: $push (collect into array)
    pub fn push(expression: impl Into<Bson>) -> Document {
        bson::doc! { "$push": expression.into() }
    }

    /// Accumulator: $addToSet (collect unique values)
    pub fn add_to_set(expression: impl Into<Bson>) -> Document {
        bson::doc! { "$addToSet": expression.into() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_value_to_bson_primitives() {
        assert_eq!(
            filter_value_to_bson(&FilterValue::Int(42)).unwrap(),
            Bson::Int64(42)
        );
        assert_eq!(
            filter_value_to_bson(&FilterValue::Float(3.14)).unwrap(),
            Bson::Double(3.14)
        );
        assert_eq!(
            filter_value_to_bson(&FilterValue::Bool(true)).unwrap(),
            Bson::Boolean(true)
        );
        assert_eq!(
            filter_value_to_bson(&FilterValue::Null).unwrap(),
            Bson::Null
        );
    }

    #[test]
    fn test_filter_value_to_bson_string() {
        let result = filter_value_to_bson(&FilterValue::String("hello".to_string())).unwrap();
        assert_eq!(result, Bson::String("hello".to_string()));
    }

    #[test]
    fn test_filter_value_to_bson_object_id() {
        let oid = ObjectId::new();
        let result = filter_value_to_bson(&FilterValue::String(oid.to_hex())).unwrap();
        assert_eq!(result, Bson::ObjectId(oid));
    }

    #[test]
    fn test_filter_value_to_bson_list() {
        let list = vec![
            FilterValue::Int(1),
            FilterValue::Int(2),
            FilterValue::Int(3),
        ];
        let result = filter_value_to_bson(&FilterValue::List(list)).unwrap();
        assert_eq!(
            result,
            Bson::Array(vec![Bson::Int64(1), Bson::Int64(2), Bson::Int64(3)])
        );
    }

    #[test]
    fn test_type_mappings() {
        use mongo_types::*;

        assert_eq!(rust_type_to_bson("i32"), Some("int"));
        assert_eq!(rust_type_to_bson("String"), Some("string"));
        assert_eq!(rust_type_to_bson("bool"), Some("bool"));

        assert_eq!(bson_type_to_rust("int"), "i32");
        assert_eq!(bson_type_to_rust("string"), "String");
        assert_eq!(bson_type_to_rust("bool"), "bool");

        assert_eq!(prax_type_to_bson("Int"), "int");
        assert_eq!(prax_type_to_bson("String"), "string");
        assert_eq!(prax_type_to_bson("Boolean"), "bool");
    }

    #[test]
    fn test_aggregation_stages() {
        use aggregation::*;

        let match_doc = match_stage(bson::doc! { "status": "active" });
        assert!(match_doc.contains_key("$match"));

        let sort_doc = sort_stage(bson::doc! { "created_at": -1 });
        assert!(sort_doc.contains_key("$sort"));

        let limit_doc = limit_stage(10);
        assert!(limit_doc.contains_key("$limit"));
    }
}
