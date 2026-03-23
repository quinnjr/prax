//! Type mapping from Prax scalar types to TypeScript and Zod types.

use prax_schema::ScalarType;

/// Maps Prax types to TypeScript / Zod representations.
pub struct TypeMapper;

impl TypeMapper {
    /// Map a Prax scalar type to its TypeScript type string.
    pub fn ts_type(scalar: &ScalarType) -> &'static str {
        match scalar {
            ScalarType::Int => "number",
            ScalarType::BigInt => "bigint",
            ScalarType::Float => "number",
            ScalarType::Decimal => "string",
            ScalarType::String => "string",
            ScalarType::Boolean => "boolean",
            ScalarType::DateTime => "Date",
            ScalarType::Date => "string",
            ScalarType::Time => "string",
            ScalarType::Json => "unknown",
            ScalarType::Bytes => "Uint8Array",
            ScalarType::Uuid => "string",
            ScalarType::Cuid | ScalarType::Cuid2 | ScalarType::NanoId | ScalarType::Ulid => {
                "string"
            }
            ScalarType::Vector(_) | ScalarType::HalfVector(_) => "number[]",
            ScalarType::SparseVector(_) => "Array<[number, number]>",
            ScalarType::Bit(_) => "Uint8Array",
        }
    }

    /// Map a Prax scalar type to its Zod schema expression.
    pub fn zod_type(scalar: &ScalarType) -> &'static str {
        match scalar {
            ScalarType::Int => "z.number().int()",
            ScalarType::BigInt => "z.bigint()",
            ScalarType::Float => "z.number()",
            ScalarType::Decimal => "z.string()",
            ScalarType::String => "z.string()",
            ScalarType::Boolean => "z.boolean()",
            ScalarType::DateTime => "z.coerce.date()",
            ScalarType::Date => "z.string().date()",
            ScalarType::Time => "z.string().time()",
            ScalarType::Json => "z.unknown()",
            ScalarType::Bytes => "z.instanceof(Uint8Array)",
            ScalarType::Uuid => "z.string().uuid()",
            ScalarType::Cuid => "z.string().cuid()",
            ScalarType::Cuid2 => "z.string().cuid2()",
            ScalarType::NanoId => "z.string().nanoid()",
            ScalarType::Ulid => "z.string().ulid()",
            ScalarType::Vector(_) | ScalarType::HalfVector(_) => "z.array(z.number())",
            ScalarType::SparseVector(_) => "z.array(z.tuple([z.number(), z.number()]))",
            ScalarType::Bit(_) => "z.instanceof(Uint8Array)",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ts_scalars() {
        assert_eq!(TypeMapper::ts_type(&ScalarType::Int), "number");
        assert_eq!(TypeMapper::ts_type(&ScalarType::BigInt), "bigint");
        assert_eq!(TypeMapper::ts_type(&ScalarType::String), "string");
        assert_eq!(TypeMapper::ts_type(&ScalarType::Boolean), "boolean");
        assert_eq!(TypeMapper::ts_type(&ScalarType::DateTime), "Date");
        assert_eq!(TypeMapper::ts_type(&ScalarType::Json), "unknown");
        assert_eq!(TypeMapper::ts_type(&ScalarType::Uuid), "string");
        assert_eq!(TypeMapper::ts_type(&ScalarType::Decimal), "string");
        assert_eq!(TypeMapper::ts_type(&ScalarType::Bytes), "Uint8Array");
        assert_eq!(
            TypeMapper::ts_type(&ScalarType::Vector(Some(128))),
            "number[]"
        );
    }

    #[test]
    fn test_zod_scalars() {
        assert_eq!(TypeMapper::zod_type(&ScalarType::Int), "z.number().int()");
        assert_eq!(TypeMapper::zod_type(&ScalarType::String), "z.string()");
        assert_eq!(TypeMapper::zod_type(&ScalarType::Uuid), "z.string().uuid()");
        assert_eq!(
            TypeMapper::zod_type(&ScalarType::DateTime),
            "z.coerce.date()"
        );
        assert_eq!(TypeMapper::zod_type(&ScalarType::BigInt), "z.bigint()");
    }

    #[test]
    fn test_id_types_are_strings() {
        assert_eq!(TypeMapper::ts_type(&ScalarType::Cuid), "string");
        assert_eq!(TypeMapper::ts_type(&ScalarType::Cuid2), "string");
        assert_eq!(TypeMapper::ts_type(&ScalarType::NanoId), "string");
        assert_eq!(TypeMapper::ts_type(&ScalarType::Ulid), "string");
    }
}
