//! Runtime payload shapes for typed write inputs.
//!
//! Phase 5a wires the codegen-emitted `<Model>CreateInput` and
//! `<Model>UpdateInput` structs into the existing `CreateOperation` /
//! `UpdateOperation` / `UpdateManyOperation` / `UpsertOperation`
//! builders. The translation is purely additive: each input lowers
//! to a list of column-keyed assignments that the builder appends
//! to its existing internal state.
//!
//! - `CreateInput::Data` is fixed to [`CreatePayload`] — the same
//!   `Vec<(column, value)>` shape that the existing
//!   `CreateOperation::set_many` already accepts.
//! - `UpdateInput::Data` is fixed to [`UpdatePayload`] — a list of
//!   `(column, WriteOp)` pairs that carry the atomic-operator
//!   information `IntFieldUpdate { increment, decrement, multiply,
//!   divide, set }`, `StringNullableFieldUpdate { unset }`, etc.
//!   express.
//!
//! Nested writes (relation operators inside `data:`) are deferred to
//! phase 5b and have no payload here — phase 5a's codegen rejects
//! relation keys with a clear "phase 5b" diagnostic before any
//! lowering reaches the runtime.

use crate::filter::FilterValue;

/// Flat column-value payload for a single-row create.
///
/// Each tuple is `(column_name, value)`. Codegen emits this as the
/// `Data` associated type for every per-model `<Model>CreateInput`
/// in phase 5a.
pub type CreatePayload = Vec<(String, FilterValue)>;

/// Flat column-operator payload for an update.
///
/// Each tuple is `(column_name, WriteOp)`. Codegen emits this as the
/// `Data` associated type for every per-model `<Model>UpdateInput`
/// in phase 5a. Atomic operators (`increment`, `decrement`,
/// `multiply`, `divide`) are preserved as distinct variants so the
/// SQL builder can emit `col = col + $n` rather than `col = $n`.
pub type UpdatePayload = Vec<(String, WriteOp)>;

/// One scalar update operator applied to a column.
///
/// Mirrors the `*FieldUpdate` wrapper structs in
/// [`crate::inputs::scalar_update`]. Each wrapper lowers exactly one
/// of its set-only / increment / decrement / multiply / divide / unset
/// fields to the matching variant here.
#[derive(Debug, Clone, PartialEq)]
pub enum WriteOp {
    /// `col = value` — the default form.
    Set(FilterValue),
    /// `col = col + value` — numeric scalars only.
    Increment(FilterValue),
    /// `col = col - value` — numeric scalars only.
    Decrement(FilterValue),
    /// `col = col * value` — numeric scalars only.
    Multiply(FilterValue),
    /// `col = col / value` — numeric scalars only.
    Divide(FilterValue),
    /// `col = NULL` — nullable scalars only.
    Unset,
}

impl WriteOp {
    /// True when the operator targets a numeric column.
    ///
    /// Used by the SQL emitter to pick between `col = $n` (Set/Unset)
    /// and `col = col <op> $n` (the arithmetic variants).
    pub fn is_arithmetic(&self) -> bool {
        matches!(
            self,
            WriteOp::Increment(_)
                | WriteOp::Decrement(_)
                | WriteOp::Multiply(_)
                | WriteOp::Divide(_)
        )
    }

    /// Render this operator as a SET-clause fragment.
    ///
    /// Given the column name and a `1..` placeholder offset, returns
    /// the textual fragment (`col = $1`, `col = col + $1`, `col = NULL`)
    /// and the value to push to the parameter list. `Unset` returns
    /// `None` for the value — the caller skips the parameter slot.
    pub fn to_set_fragment(
        &self,
        column: &str,
        placeholder: &str,
    ) -> (String, Option<FilterValue>) {
        match self {
            WriteOp::Set(v) => (format!("{column} = {placeholder}"), Some(v.clone())),
            WriteOp::Increment(v) => (
                format!("{column} = {column} + {placeholder}"),
                Some(v.clone()),
            ),
            WriteOp::Decrement(v) => (
                format!("{column} = {column} - {placeholder}"),
                Some(v.clone()),
            ),
            WriteOp::Multiply(v) => (
                format!("{column} = {column} * {placeholder}"),
                Some(v.clone()),
            ),
            WriteOp::Divide(v) => (
                format!("{column} = {column} / {placeholder}"),
                Some(v.clone()),
            ),
            WriteOp::Unset => (format!("{column} = NULL"), None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_op_set_fragment() {
        let op = WriteOp::Set(FilterValue::Int(7));
        let (frag, val) = op.to_set_fragment("age", "$1");
        assert_eq!(frag, "age = $1");
        assert_eq!(val, Some(FilterValue::Int(7)));
    }

    #[test]
    fn write_op_increment_fragment() {
        let op = WriteOp::Increment(FilterValue::Int(1));
        let (frag, val) = op.to_set_fragment("count", "$2");
        assert_eq!(frag, "count = count + $2");
        assert_eq!(val, Some(FilterValue::Int(1)));
    }

    #[test]
    fn write_op_unset_skips_param() {
        let op = WriteOp::Unset;
        let (frag, val) = op.to_set_fragment("name", "$1");
        assert_eq!(frag, "name = NULL");
        assert!(val.is_none());
    }

    #[test]
    fn write_op_is_arithmetic() {
        assert!(WriteOp::Increment(FilterValue::Int(1)).is_arithmetic());
        assert!(WriteOp::Decrement(FilterValue::Int(1)).is_arithmetic());
        assert!(WriteOp::Multiply(FilterValue::Int(1)).is_arithmetic());
        assert!(WriteOp::Divide(FilterValue::Int(1)).is_arithmetic());
        assert!(!WriteOp::Set(FilterValue::Int(1)).is_arithmetic());
        assert!(!WriteOp::Unset.is_arithmetic());
    }
}
