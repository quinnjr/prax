//! TimescaleDB schema types.

use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use super::{Attribute, AttributeValue, Model, View};
use crate::error::SchemaError;

const MICROS_PER_MS: i64 = 1_000;
const MICROS_PER_SEC: i64 = 1_000_000;
const MICROS_PER_MIN: i64 = 60 * MICROS_PER_SEC;
const MICROS_PER_HOUR: i64 = 60 * MICROS_PER_MIN;
const MICROS_PER_DAY: i64 = 24 * MICROS_PER_HOUR;

/// Error produced when parsing a [`PgInterval`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum IntervalError {
    /// The input was empty or whitespace only.
    #[error("interval is empty")]
    Empty,
    /// A quantity was not a non-negative whole number.
    #[error("invalid interval quantity '{0}': expected a non-negative whole number")]
    InvalidNumber(String),
    /// A quantity was not followed by a unit.
    #[error("interval quantity '{0}' is missing a unit")]
    MissingUnit(String),
    /// A unit was not recognised.
    #[error("unknown interval unit '{0}'")]
    UnknownUnit(String),
    /// A unit appeared more than once.
    #[error("interval unit '{0}' is repeated")]
    RepeatedUnit(String),
    /// The interval does not fit in the Postgres interval representation.
    #[error("interval is too large")]
    Overflow,
    /// The interval is zero.
    #[error("interval must be greater than zero")]
    Zero,
}

/// A Postgres interval value split into months, days and microseconds.
///
/// Equality is structural: `1 day` and `24 hours` are different values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PgInterval {
    /// Whole months (a year is 12 months).
    pub months: i32,
    /// Whole days (a week is 7 days).
    pub days: i32,
    /// Sub-day time in microseconds.
    pub micros: i64,
}

impl PgInterval {
    /// Create an interval from its components.
    #[must_use]
    pub const fn new(months: i32, days: i32, micros: i64) -> Self {
        Self {
            months,
            days,
            micros,
        }
    }

    /// Parse whitespace-separated `<count> <unit>` pairs, e.g. `1 day 90 minutes`.
    ///
    /// # Errors
    ///
    /// Returns an [`IntervalError`] naming the offending token.
    pub fn parse(s: &str) -> Result<Self, IntervalError> {
        let mut tokens = s.split_whitespace().peekable();
        if tokens.peek().is_none() {
            return Err(IntervalError::Empty);
        }
        let (mut months, mut days, mut micros) = (0i64, 0i64, 0i64);
        let mut seen: HashSet<&'static str> = HashSet::new();
        while let Some(num) = tokens.next() {
            let n: u64 = num
                .parse()
                .map_err(|_| IntervalError::InvalidNumber(num.to_string()))?;
            let n = i64::try_from(n).map_err(|_| IntervalError::Overflow)?;
            let unit_tok = tokens
                .next()
                .ok_or_else(|| IntervalError::MissingUnit(num.to_string()))?;
            let lower = unit_tok.to_ascii_lowercase();
            let singular = lower.strip_suffix('s').unwrap_or(&lower);
            let (name, target, factor): (&'static str, &mut i64, i64) = match singular {
                "microsecond" => ("microsecond", &mut micros, 1),
                "millisecond" => ("millisecond", &mut micros, MICROS_PER_MS),
                "second" => ("second", &mut micros, MICROS_PER_SEC),
                "minute" => ("minute", &mut micros, MICROS_PER_MIN),
                "hour" => ("hour", &mut micros, MICROS_PER_HOUR),
                "day" => ("day", &mut days, 1),
                "week" => ("week", &mut days, 7),
                "month" => ("month", &mut months, 1),
                "year" => ("year", &mut months, 12),
                _ => return Err(IntervalError::UnknownUnit(lower)),
            };
            if !seen.insert(name) {
                return Err(IntervalError::RepeatedUnit(name.to_string()));
            }
            let add = n.checked_mul(factor).ok_or(IntervalError::Overflow)?;
            *target = target.checked_add(add).ok_or(IntervalError::Overflow)?;
        }
        let months = i32::try_from(months).map_err(|_| IntervalError::Overflow)?;
        let days = i32::try_from(days).map_err(|_| IntervalError::Overflow)?;
        if months == 0 && days == 0 && micros == 0 {
            return Err(IntervalError::Zero);
        }
        Ok(Self::new(months, days, micros))
    }

    /// Total microseconds if the interval has no month component.
    ///
    /// A day is counted as 24 hours.
    #[must_use]
    pub fn fixed_micros(&self) -> Option<i64> {
        if self.months != 0 {
            return None;
        }
        i64::from(self.days)
            .checked_mul(MICROS_PER_DAY)
            .and_then(|d| d.checked_add(self.micros))
    }

    fn total_micros(&self, days_per_month: i128) -> i128 {
        (i128::from(self.months) * days_per_month + i128::from(self.days))
            * i128::from(MICROS_PER_DAY)
            + i128::from(self.micros)
    }

    /// Upper bound in microseconds, counting a month as 31 days.
    #[must_use]
    pub fn upper_micros(&self) -> i128 {
        self.total_micros(31)
    }

    /// Lower bound in microseconds, counting a month as 28 days.
    #[must_use]
    pub fn lower_micros(&self) -> i128 {
        self.total_micros(28)
    }

    /// Render as a SQL interval literal, e.g. `INTERVAL '1 day'`.
    #[must_use]
    pub fn to_sql(&self) -> String {
        format!("INTERVAL '{self}'")
    }
}

fn unit(n: i64, name: &str) -> String {
    if n == 1 {
        format!("{n} {name}")
    } else {
        format!("{n} {name}s")
    }
}

impl fmt::Display for PgInterval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if self.months != 0 {
            parts.push(unit(i64::from(self.months), "month"));
        }
        if self.days != 0 {
            parts.push(unit(i64::from(self.days), "day"));
        }
        if self.micros != 0 {
            let m = self.micros;
            let (div, name) = [
                (MICROS_PER_HOUR, "hour"),
                (MICROS_PER_MIN, "minute"),
                (MICROS_PER_SEC, "second"),
                (MICROS_PER_MS, "millisecond"),
            ]
            .into_iter()
            .find(|(d, _)| m % d == 0)
            .unwrap_or((1, "microsecond"));
            parts.push(unit(m / div, name));
        }
        f.write_str(&parts.join(" "))
    }
}

impl FromStr for PgInterval {
    type Err = IntervalError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// Default chunk interval for a hypertable (`7 days`).
pub const DEFAULT_CHUNK_INTERVAL: PgInterval = PgInterval::new(0, 7, 0);

/// Typed `@@timescale.*` configuration of a model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TimescaleModelConfig {
    /// Hypertable settings (`@@timescale.hypertable`).
    pub hypertable: HypertableConfig,
    /// Columnstore settings (`@@timescale.columnstore`).
    pub columnstore: Option<ColumnstoreConfig>,
    /// Retention policy (`@@timescale.retention`).
    pub retention: Option<RetentionConfig>,
}

/// Hypertable settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct HypertableConfig {
    /// Time dimension field.
    pub time_field: SmolStr,
    /// Chunk interval (never contains months).
    pub chunk_interval: PgInterval,
    /// Optional hash space dimension.
    pub space: Option<HashDimension>,
}

/// A hash-partitioned space dimension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct HashDimension {
    /// Field to partition on.
    pub field: SmolStr,
    /// Number of partitions (1..=32767).
    pub partitions: i16,
}

/// Columnstore settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ColumnstoreConfig {
    /// Segment-by fields (empty means none).
    pub segment_by: Vec<SmolStr>,
    /// Order-by columns.
    pub order_by: Vec<OrderByColumn>,
    /// Convert chunks older than this; `None` means no conversion policy.
    pub after: Option<PgInterval>,
}

/// One column of a columnstore order-by list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct OrderByColumn {
    /// Field name.
    pub field: SmolStr,
    /// Whether the order is descending.
    pub descending: bool,
}

/// Retention policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RetentionConfig {
    /// Drop chunks older than this.
    pub drop_after: PgInterval,
}

/// Typed `@@timescale.continuous` configuration of a view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ContinuousAggregateConfig {
    /// Model or continuous view the aggregate reads.
    pub source: SmolStr,
    /// Refresh policy schedule interval.
    pub schedule_interval: PgInterval,
    /// Refresh window start offset; `None` means `NULL`.
    pub start_offset: Option<PgInterval>,
    /// Refresh window end offset; `None` means `NULL`.
    pub end_offset: Option<PgInterval>,
    /// Whether to serve materialized data only.
    pub materialized_only: bool,
}

/// Which kind of block an attribute is being parsed on.
#[derive(Clone, Copy)]
enum Owner {
    Model,
    View,
}

/// Prefix used in error messages, e.g. ``model `Metric`: ``.
struct Ctx {
    owner: Owner,
    name: String,
}

impl Ctx {
    fn err(&self, attr: &str, msg: impl fmt::Display) -> SchemaError {
        let kind = match self.owner {
            Owner::Model => "model",
            Owner::View => "view",
        };
        SchemaError::InvalidAttribute {
            attribute: format!("@@timescale.{attr}"),
            message: format!("{kind} `{}`: {msg}", self.name),
        }
    }
}

/// Collected named arguments of one attribute.
struct Args<'a> {
    attr: &'a str,
    ctx: &'a Ctx,
    named: Vec<(&'a str, &'a AttributeValue)>,
}

impl<'a> Args<'a> {
    /// Validate names (known, not repeated) and reject unexpected positionals.
    fn collect(
        ctx: &'a Ctx,
        attr_name: &'a str,
        attribute: &'a Attribute,
        known: &[&str],
    ) -> Result<Self, SchemaError> {
        let mut named: Vec<(&str, &AttributeValue)> = Vec::new();
        for (i, arg) in attribute.args.iter().enumerate() {
            match &arg.name {
                None => {
                    let allowed = attr_name == "hypertable" && i == 0;
                    if !allowed {
                        return Err(ctx.err(attr_name, "unexpected positional argument"));
                    }
                }
                Some(n) => {
                    let n = n.as_str();
                    if !known.contains(&n) {
                        return Err(ctx.err(
                            attr_name,
                            format!(
                                "unknown argument `{n}` (expected one of: {})",
                                known.join(", ")
                            ),
                        ));
                    }
                    if named.iter().any(|(k, _)| *k == n) {
                        return Err(ctx.err(attr_name, format!("argument `{n}` is repeated")));
                    }
                    named.push((n, &arg.value));
                }
            }
        }
        Ok(Self {
            attr: attr_name,
            ctx,
            named,
        })
    }

    fn get(&self, name: &str) -> Option<&'a AttributeValue> {
        self.named.iter().find(|(k, _)| *k == name).map(|(_, v)| *v)
    }

    fn err(&self, msg: impl fmt::Display) -> SchemaError {
        self.ctx.err(self.attr, msg)
    }

    fn interval(&self, name: &str) -> Result<Option<PgInterval>, SchemaError> {
        let Some(v) = self.get(name) else {
            return Ok(None);
        };
        let s = v
            .as_string()
            .ok_or_else(|| self.err(format!("`{name}` must be a string interval")))?;
        PgInterval::parse(s)
            .map(Some)
            .map_err(|e| self.err(format!("`{name}`: {e}")))
    }

    fn required_interval(&self, name: &str) -> Result<PgInterval, SchemaError> {
        self.interval(name)?
            .ok_or_else(|| self.err(format!("missing required argument `{name}`")))
    }
}

/// Reject a name that is not a `timescale.*` attribute of this owner kind.
fn check_name(ctx: &Ctx, name: &str, allowed: &[&str]) -> Result<(), SchemaError> {
    if allowed.contains(&name) {
        return Ok(());
    }
    const ALL: [&str; 4] = ["hypertable", "columnstore", "retention", "continuous"];
    if ALL.contains(&name) {
        let other = match ctx.owner {
            Owner::Model => "views",
            Owner::View => "models",
        };
        return Err(ctx.err(
            name,
            format!("`@@timescale.{name}` can only be used on {other}"),
        ));
    }
    Err(ctx.err(
        name,
        format!(
            "unknown attribute `@@timescale.{name}` (expected one of: {})",
            allowed.join(", ")
        ),
    ))
}

fn field_list(
    args: &Args<'_>,
    name: &str,
    v: &AttributeValue,
) -> Result<Vec<SmolStr>, SchemaError> {
    v.as_field_names()
        .map(|names| names.into_iter().map(SmolStr::new).collect())
        .ok_or_else(|| args.err(format!("`{name}` must be a list of field names")))
}

fn order_by_list(args: &Args<'_>, v: &AttributeValue) -> Result<Vec<OrderByColumn>, SchemaError> {
    let bad = || args.err("`orderBy` must be a list of `field`, `asc(field)` or `desc(field)`");
    let plain = |f: &str| OrderByColumn {
        field: SmolStr::new(f),
        descending: false,
    };
    let items: Vec<OrderByColumn> = if let Some(names) = v.as_field_names() {
        names.into_iter().map(plain).collect()
    } else if let AttributeValue::Array(items) = v {
        items
            .iter()
            .map(|i| match i {
                AttributeValue::Ident(f) | AttributeValue::FieldRef(f) => Ok(plain(f)),
                AttributeValue::Function(func, a) if func == "asc" || func == "desc" => {
                    match a.as_slice() {
                        [AttributeValue::Ident(f)] => Ok(OrderByColumn {
                            field: f.clone(),
                            descending: func == "desc",
                        }),
                        _ => Err(bad()),
                    }
                }
                _ => Err(bad()),
            })
            .collect::<Result<_, _>>()?
    } else {
        return Err(bad());
    };
    if items.is_empty() {
        return Err(args.err("`orderBy` must not be empty"));
    }
    Ok(items)
}

impl TimescaleModelConfig {
    /// Parse the `@@timescale.*` attributes of a model.
    ///
    /// Returns `Ok(None)` when the model has no `@@timescale.*` attribute.
    /// Only structural rules are enforced here; field-existence and
    /// cross-model rules are validated separately.
    ///
    /// # Errors
    ///
    /// Returns [`SchemaError::InvalidAttribute`] for unknown attributes or
    /// arguments, wrong value shapes, bad intervals, or `columnstore` /
    /// `retention` without `hypertable`.
    pub fn from_model(model: &Model) -> Result<Option<Self>, SchemaError> {
        let ctx = Ctx {
            owner: Owner::Model,
            name: model.name().to_string(),
        };
        let mut hypertable = None;
        let mut columnstore = None;
        let mut retention = None;
        let mut any = false;
        for attribute in &model.attributes {
            let Some(name) = attribute.name.as_str().strip_prefix("timescale.") else {
                continue;
            };
            any = true;
            check_name(&ctx, name, &["hypertable", "columnstore", "retention"])?;
            let repeated = match name {
                "hypertable" => hypertable.is_some(),
                "columnstore" => columnstore.is_some(),
                _ => retention.is_some(),
            };
            if repeated {
                return Err(ctx.err(name, format!("`@@timescale.{name}` is repeated")));
            }
            match name {
                "hypertable" => hypertable = Some(parse_hypertable(&ctx, attribute)?),
                "columnstore" => columnstore = Some(attribute),
                _ => retention = Some(parse_retention(&ctx, attribute)?),
            }
        }
        if !any {
            return Ok(None);
        }
        let Some(hypertable) = hypertable else {
            let which = if columnstore.is_some() {
                "columnstore"
            } else {
                "retention"
            };
            return Err(ctx.err(
                which,
                format!("`@@timescale.{which}` requires `@@timescale.hypertable`"),
            ));
        };
        let columnstore = columnstore
            .map(|a| parse_columnstore(&ctx, a, &hypertable.time_field))
            .transpose()?;
        Ok(Some(Self {
            hypertable,
            columnstore,
            retention,
        }))
    }
}

fn parse_hypertable(ctx: &Ctx, attribute: &Attribute) -> Result<HypertableConfig, SchemaError> {
    let args = Args::collect(ctx, "hypertable", attribute, &["chunkInterval", "space"])?;
    let time_field = match attribute.args.first() {
        Some(a) if a.name.is_none() => match &a.value {
            AttributeValue::Ident(s) | AttributeValue::FieldRef(s) => s.clone(),
            _ => return Err(args.err("the time field must be a field name")),
        },
        _ => return Err(args.err("missing required time field (first positional argument)")),
    };
    let chunk_interval = match args.interval("chunkInterval")? {
        Some(i) if i.months != 0 => {
            return Err(
                args.err("`chunkInterval` must not contain months or years; use days or weeks")
            );
        }
        Some(i) => i,
        None => DEFAULT_CHUNK_INTERVAL,
    };
    let space = match args.get("space") {
        None => None,
        Some(AttributeValue::Function(f, a)) if f == "hash" => match a.as_slice() {
            [AttributeValue::Ident(field), AttributeValue::Int(n)] => {
                let partitions = i16::try_from(*n).ok().filter(|p| *p >= 1).ok_or_else(|| {
                    args.err(format!(
                        "hash partitions must be between 1 and 32767, got {n}"
                    ))
                })?;
                Some(HashDimension {
                    field: field.clone(),
                    partitions,
                })
            }
            _ => return Err(args.err("`space` must be `hash(<field>, <partitions>)`")),
        },
        Some(_) => return Err(args.err("`space` must be `hash(<field>, <partitions>)`")),
    };
    Ok(HypertableConfig {
        time_field,
        chunk_interval,
        space,
    })
}

fn parse_columnstore(
    ctx: &Ctx,
    attribute: &Attribute,
    time_field: &SmolStr,
) -> Result<ColumnstoreConfig, SchemaError> {
    let args = Args::collect(
        ctx,
        "columnstore",
        attribute,
        &["segmentBy", "orderBy", "after"],
    )?;
    let segment_by = match args.get("segmentBy") {
        Some(v) => field_list(&args, "segmentBy", v)?,
        None => Vec::new(),
    };
    let order_by = match args.get("orderBy") {
        Some(v) => order_by_list(&args, v)?,
        None => vec![OrderByColumn {
            field: time_field.clone(),
            descending: true,
        }],
    };
    let after = args.interval("after")?;
    Ok(ColumnstoreConfig {
        segment_by,
        order_by,
        after,
    })
}

fn parse_retention(ctx: &Ctx, attribute: &Attribute) -> Result<RetentionConfig, SchemaError> {
    let args = Args::collect(ctx, "retention", attribute, &["dropAfter"])?;
    Ok(RetentionConfig {
        drop_after: args.required_interval("dropAfter")?,
    })
}

impl ContinuousAggregateConfig {
    /// Parse the `@@timescale.continuous` attribute of a view.
    ///
    /// Returns `Ok(None)` when the view has no `@@timescale.*` attribute.
    ///
    /// # Errors
    ///
    /// Returns [`SchemaError::InvalidAttribute`] for unknown attributes or
    /// arguments, wrong value shapes, missing required arguments, or bad
    /// intervals.
    pub fn from_view(view: &View) -> Result<Option<Self>, SchemaError> {
        let ctx = Ctx {
            owner: Owner::View,
            name: view.name().to_string(),
        };
        let mut found: Option<&Attribute> = None;
        for attribute in &view.attributes {
            let Some(name) = attribute.name.as_str().strip_prefix("timescale.") else {
                continue;
            };
            check_name(&ctx, name, &["continuous"])?;
            if found.is_some() {
                return Err(ctx.err(name, "`@@timescale.continuous` is repeated"));
            }
            found = Some(attribute);
        }
        let Some(attribute) = found else {
            return Ok(None);
        };
        let args = Args::collect(
            &ctx,
            "continuous",
            attribute,
            &[
                "source",
                "scheduleInterval",
                "startOffset",
                "endOffset",
                "materializedOnly",
            ],
        )?;
        let source = match args.get("source") {
            Some(AttributeValue::Ident(s)) => s.clone(),
            Some(_) => return Err(args.err("`source` must be a model or view name")),
            None => return Err(args.err("missing required argument `source`")),
        };
        let materialized_only = match args.get("materializedOnly") {
            None => true,
            Some(AttributeValue::Boolean(b)) => *b,
            Some(_) => return Err(args.err("`materializedOnly` must be true or false")),
        };
        Ok(Some(Self {
            source,
            schedule_interval: args.required_interval("scheduleInterval")?,
            start_offset: args.interval("startOffset")?,
            end_offset: args.interval("endOffset")?,
            materialized_only,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interval_parse_units() {
        assert_eq!(
            PgInterval::parse("1 day").unwrap(),
            PgInterval::new(0, 1, 0)
        );
        assert_eq!(
            PgInterval::parse("2 weeks").unwrap(),
            PgInterval::new(0, 14, 0)
        );
        assert_eq!(
            PgInterval::parse("1 year 2 months").unwrap(),
            PgInterval::new(14, 0, 0)
        );
        assert_eq!(
            PgInterval::parse("90 minutes").unwrap(),
            PgInterval::new(0, 0, 5_400_000_000)
        );
        assert_eq!(
            PgInterval::parse("1 hour").unwrap(),
            PgInterval::new(0, 0, 3_600_000_000)
        );
        assert_eq!(
            PgInterval::parse("5 seconds").unwrap(),
            PgInterval::new(0, 0, 5_000_000)
        );
        assert_eq!(
            PgInterval::parse("3 milliseconds").unwrap(),
            PgInterval::new(0, 0, 3_000)
        );
        assert_eq!(
            PgInterval::parse("7 microseconds").unwrap(),
            PgInterval::new(0, 0, 7)
        );
        assert_eq!(
            PgInterval::parse("1 week 1 day").unwrap(),
            PgInterval::new(0, 8, 0)
        );
    }

    #[test]
    fn test_interval_parse_is_case_and_space_insensitive() {
        assert_eq!(
            PgInterval::parse("1 Day").unwrap(),
            PgInterval::new(0, 1, 0)
        );
        assert_eq!(
            PgInterval::parse("  7   DAYS  ").unwrap(),
            PgInterval::new(0, 7, 0)
        );
    }

    #[test]
    fn test_interval_rejects() {
        assert_eq!(PgInterval::parse(""), Err(IntervalError::Empty));
        assert_eq!(PgInterval::parse("   "), Err(IntervalError::Empty));
        assert_eq!(
            PgInterval::parse("day"),
            Err(IntervalError::InvalidNumber("day".into()))
        );
        assert_eq!(
            PgInterval::parse("1"),
            Err(IntervalError::MissingUnit("1".into()))
        );
        assert_eq!(
            PgInterval::parse("-1 day"),
            Err(IntervalError::InvalidNumber("-1".into()))
        );
        assert_eq!(
            PgInterval::parse("1.5 days"),
            Err(IntervalError::InvalidNumber("1.5".into()))
        );
        assert_eq!(
            PgInterval::parse("1 fortnight"),
            Err(IntervalError::UnknownUnit("fortnight".into()))
        );
        assert_eq!(
            PgInterval::parse("1 day 2 days"),
            Err(IntervalError::RepeatedUnit("day".into()))
        );
        assert_eq!(PgInterval::parse("0 days"), Err(IntervalError::Zero));
        assert_eq!(
            PgInterval::parse("99999999999 years"),
            Err(IntervalError::Overflow)
        );
        assert!(
            PgInterval::parse("1 fortnight")
                .unwrap_err()
                .to_string()
                .contains("fortnight")
        );
    }

    #[test]
    fn test_interval_equality_is_structural() {
        let day = PgInterval::parse("1 day").unwrap();
        let hours = PgInterval::parse("24 hours").unwrap();
        assert_ne!(day, hours);
        assert_eq!(day.fixed_micros(), Some(86_400_000_000));
        assert_eq!(hours.fixed_micros(), Some(86_400_000_000));
        assert_eq!(PgInterval::parse("1 month").unwrap().fixed_micros(), None);
    }

    #[test]
    fn test_interval_bounds() {
        let month = PgInterval::parse("1 month").unwrap();
        assert_eq!(month.upper_micros(), 31 * 86_400_000_000);
        assert_eq!(month.lower_micros(), 28 * 86_400_000_000);
        let day = PgInterval::parse("1 day").unwrap();
        assert_eq!(day.upper_micros(), day.lower_micros());
    }

    #[test]
    fn test_interval_display_and_sql() {
        assert_eq!(
            PgInterval::new(0, 1, 5_400_000_000).to_string(),
            "1 day 90 minutes"
        );
        assert_eq!(PgInterval::new(14, 0, 0).to_string(), "14 months");
        assert_eq!(PgInterval::new(0, 2, 0).to_string(), "2 days");
        assert_eq!(
            PgInterval::new(0, 0, 1_500_000).to_string(),
            "1500 milliseconds"
        );
        assert_eq!(
            PgInterval::new(0, 1, 5_400_000_000).to_sql(),
            "INTERVAL '1 day 90 minutes'"
        );
    }

    proptest::proptest! {
        #[test]
        fn prop_interval_display_round_trips(months in 0i32..1000, days in 0i32..10_000, micros in 0i64..1_000_000_000_000) {
            proptest::prop_assume!(months != 0 || days != 0 || micros != 0);
            let i = PgInterval::new(months, days, micros);
            proptest::prop_assert_eq!(PgInterval::parse(&i.to_string()).unwrap(), i);
        }
    }
}

#[cfg(test)]
mod config_tests {
    use super::*;
    use crate::parser::parse_schema;

    const METRIC: &str = r#"
model Metric {
  time     DateTime
  deviceId String
  value    Float
  @@id([deviceId, time])
  @@timescale.hypertable(time, chunkInterval: "1 day", space: hash(deviceId, 4))
  @@timescale.columnstore(segmentBy: [deviceId], orderBy: [desc(time)], after: "7 days")
  @@timescale.retention(dropAfter: "90 days")
}
"#;

    fn model_with(attrs: &str) -> Model {
        let src = format!(
            "model Metric {{\n  time DateTime\n  deviceId String\n  value Float\n  @@id([deviceId, time])\n  {attrs}\n}}\n"
        );
        parse_schema(&src)
            .unwrap()
            .models
            .values()
            .next()
            .unwrap()
            .clone()
    }

    fn view(attrs: &str) -> View {
        let src =
            format!("view HourlyMetric {{\n  bucket DateTime\n  deviceId String\n  {attrs}\n}}\n");
        parse_schema(&src)
            .unwrap()
            .views
            .values()
            .next()
            .unwrap()
            .clone()
    }

    fn err_msg<T: std::fmt::Debug>(r: Result<T, SchemaError>) -> String {
        match r.unwrap_err() {
            SchemaError::InvalidAttribute { message, .. } => message,
            e => panic!("unexpected error {e:?}"),
        }
    }

    #[test]
    fn test_hypertable_defaults() {
        let c = TimescaleModelConfig::from_model(&model_with("@@timescale.hypertable(time)"))
            .unwrap()
            .unwrap();
        assert_eq!(c.hypertable.time_field, "time");
        assert_eq!(c.hypertable.chunk_interval, DEFAULT_CHUNK_INTERVAL);
        assert_eq!(c.hypertable.space, None);
        assert_eq!(c.columnstore, None);
        assert_eq!(c.retention, None);
    }

    #[test]
    fn test_hypertable_full() {
        let schema = parse_schema(METRIC).unwrap();
        let m = schema.models.get("Metric").unwrap();
        let c = m.timescale().unwrap();
        assert_eq!(c.hypertable.chunk_interval, PgInterval::new(0, 1, 0));
        assert_eq!(
            c.hypertable.space,
            Some(HashDimension {
                field: "deviceId".into(),
                partitions: 4
            })
        );
        let cs = c.columnstore.unwrap();
        assert_eq!(cs.segment_by, vec![SmolStr::from("deviceId")]);
        assert_eq!(
            cs.order_by,
            vec![OrderByColumn {
                field: "time".into(),
                descending: true
            }]
        );
        assert_eq!(cs.after, Some(PgInterval::new(0, 7, 0)));
        assert_eq!(c.retention.unwrap().drop_after, PgInterval::new(0, 90, 0));
    }

    #[test]
    fn test_columnstore_defaults() {
        let m = model_with("@@timescale.hypertable(time)\n@@timescale.columnstore()");
        let cs = TimescaleModelConfig::from_model(&m)
            .unwrap()
            .unwrap()
            .columnstore
            .unwrap();
        assert!(cs.segment_by.is_empty());
        assert_eq!(
            cs.order_by,
            vec![OrderByColumn {
                field: "time".into(),
                descending: true
            }]
        );
        assert_eq!(cs.after, None);
    }

    #[test]
    fn test_columnstore_order_by_forms() {
        let ob = |form: &str| {
            let m = model_with(&format!(
                "@@timescale.hypertable(time)\n@@timescale.columnstore(orderBy: {form})"
            ));
            TimescaleModelConfig::from_model(&m)
                .unwrap()
                .unwrap()
                .columnstore
                .unwrap()
                .order_by
        };
        let c = |f: &str, d: bool| OrderByColumn {
            field: f.into(),
            descending: d,
        };
        assert_eq!(ob("[time]"), vec![c("time", false)]);
        assert_eq!(
            ob("[desc(time), asc(deviceId)]"),
            vec![c("time", true), c("deviceId", false)]
        );
        assert_eq!(
            ob("[time, desc(value)]"),
            vec![c("time", false), c("value", true)]
        );
    }

    #[test]
    fn test_continuous_defaults_and_full() {
        let v = view(r#"@@timescale.continuous(source: Metric, scheduleInterval: "1 hour")"#);
        let c = v.continuous_aggregate().unwrap();
        assert_eq!(c.source, "Metric");
        assert_eq!(c.schedule_interval, PgInterval::new(0, 0, 3_600_000_000));
        assert!(c.materialized_only);
        assert_eq!((c.start_offset, c.end_offset), (None, None));

        let v = view(
            r#"@@timescale.continuous(source: Metric, scheduleInterval: "1 hour", startOffset: "3 days", endOffset: "1 hour")"#,
        );
        let c = v.continuous_aggregate().unwrap();
        assert_eq!(c.start_offset, Some(PgInterval::new(0, 3, 0)));
        assert_eq!(c.end_offset, Some(PgInterval::new(0, 0, 3_600_000_000)));

        let v = view(
            r#"@@timescale.continuous(source: Metric, scheduleInterval: "1 hour", materializedOnly: false)"#,
        );
        assert!(!v.continuous_aggregate().unwrap().materialized_only);
    }

    #[test]
    fn test_unknown_timescale_attribute_rejected() {
        let m = model_with("@@timescale.hypertabel(time)");
        assert!(err_msg(TimescaleModelConfig::from_model(&m)).contains("hypertabel"));
    }

    #[test]
    fn test_unknown_argument_rejected() {
        let m = model_with(r#"@@timescale.hypertable(time, chunkIntervl: "1 day")"#);
        assert!(err_msg(TimescaleModelConfig::from_model(&m)).contains("chunkIntervl"));
    }

    #[test]
    fn test_repeated_argument_rejected() {
        let m = model_with(
            r#"@@timescale.hypertable(time, chunkInterval: "1 day", chunkInterval: "2 days")"#,
        );
        assert!(err_msg(TimescaleModelConfig::from_model(&m)).contains("chunkInterval"));
    }

    #[test]
    fn test_columnstore_without_hypertable_rejected() {
        let m = model_with("@@timescale.columnstore()");
        assert!(err_msg(TimescaleModelConfig::from_model(&m)).contains("hypertable"));
    }

    #[test]
    fn test_retention_without_hypertable_rejected() {
        let m = model_with(r#"@@timescale.retention(dropAfter: "1 day")"#);
        assert!(err_msg(TimescaleModelConfig::from_model(&m)).contains("hypertable"));
    }

    #[test]
    fn test_chunk_interval_months_rejected() {
        let m = model_with(r#"@@timescale.hypertable(time, chunkInterval: "1 month")"#);
        assert!(err_msg(TimescaleModelConfig::from_model(&m)).contains("month"));
    }

    #[test]
    fn test_partitions_out_of_range_rejected() {
        for n in [0, 40000] {
            let m = model_with(&format!(
                "@@timescale.hypertable(time, space: hash(deviceId, {n}))"
            ));
            assert!(TimescaleModelConfig::from_model(&m).is_err(), "{n}");
        }
    }

    #[test]
    fn test_wrong_block_kind_rejected() {
        let m = model_with(r#"@@timescale.continuous(source: Metric, scheduleInterval: "1 hour")"#);
        assert!(TimescaleModelConfig::from_model(&m).is_err());
        let v = view("@@timescale.hypertable(bucket)");
        assert!(ContinuousAggregateConfig::from_view(&v).is_err());
    }

    #[test]
    fn test_bad_interval_names_token() {
        let m = model_with(
            "@@timescale.hypertable(time)\n@@timescale.retention(dropAfter: \"90 dayz\")",
        );
        assert!(err_msg(TimescaleModelConfig::from_model(&m)).contains("dayz"));
    }

    #[test]
    fn test_error_messages_name_block() {
        let m = model_with("@@timescale.hypertabel(time)");
        assert!(err_msg(TimescaleModelConfig::from_model(&m)).starts_with("model `Metric`: "));
        let v = view("@@timescale.hypertable(bucket)");
        assert!(
            err_msg(ContinuousAggregateConfig::from_view(&v)).starts_with("view `HourlyMetric`: ")
        );
    }

    #[test]
    fn test_view_resolved_sql_inline_and_top_level() {
        let schema = parse_schema(
            r#"
view A {
  id Int
  @@sql("SELECT 1")
}
view HourlyMetric {
  id Int
}
@@sql("HourlyMetric", """SELECT 2""")
"#,
        )
        .unwrap();
        assert_eq!(
            schema.views.get("A").unwrap().resolved_sql(&schema),
            Some("SELECT 1")
        );
        assert!(
            schema
                .views
                .get("HourlyMetric")
                .unwrap()
                .resolved_sql(&schema)
                .unwrap()
                .contains("SELECT 2")
        );
    }
}
