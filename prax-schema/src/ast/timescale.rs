//! TimescaleDB schema types.

use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

const MICROS_PER_MS: i64 = 1_000;
const MICROS_PER_SEC: i64 = 1_000_000;
const MICROS_PER_MIN: i64 = 60 * MICROS_PER_SEC;
const MICROS_PER_HOUR: i64 = 60 * MICROS_PER_MIN;
const MICROS_PER_DAY: i128 = 86_400_000_000;

/// Error produced when parsing a [`PgInterval`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
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
            .checked_mul(MICROS_PER_DAY as i64)
            .and_then(|d| d.checked_add(self.micros))
    }

    fn total_micros(&self, days_per_month: i128) -> i128 {
        (i128::from(self.months) * days_per_month + i128::from(self.days)) * MICROS_PER_DAY
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
