//! TimescaleDB attribute validation (`@@timescale.*`).

use std::collections::{HashMap, HashSet};

use super::Validator;
use crate::ast::*;
use crate::error::SchemaError;

/// Parsed TimescaleDB configurations of a schema, keyed by model / view name.
struct TimescaleMaps<'a> {
    hypertables: HashMap<&'a str, TimescaleModelConfig>,
    continuous: HashMap<&'a str, ContinuousAggregateConfig>,
}

impl Validator {
    /// Push a TimescaleDB attribute error.
    fn timescale_error(&mut self, attribute: &str, message: String) {
        self.errors.push(SchemaError::InvalidAttribute {
            attribute: attribute.to_string(),
            message,
        });
    }

    /// Validate every `@@timescale.*` attribute in the schema.
    pub(super) fn validate_timescale(&mut self, schema: &Schema) {
        let mut maps = TimescaleMaps {
            hypertables: HashMap::new(),
            continuous: HashMap::new(),
        };
        let mut uses_timescale = false;

        // Surface structural errors (unknown attributes, bad arguments,
        // missing `hypertable`) from the typed parsers.
        for model in schema.models.values() {
            if model.attributes.iter().any(|a| a.is("timescale")) {
                self.timescale_error(
                    "timescale",
                    format!(
                        "model `{}`: unknown attribute `@@timescale`; expected `@@timescale.hypertable`, `@@timescale.columnstore` or `@@timescale.retention`",
                        model.name()
                    ),
                );
            }
            uses_timescale |= model.attributes.iter().any(Attribute::is_timescale);
            match TimescaleModelConfig::from_model(model) {
                Ok(Some(cfg)) => {
                    maps.hypertables.insert(model.name(), cfg);
                }
                Ok(None) => {}
                Err(e) => self.errors.push(e),
            }
        }
        for view in schema.views.values() {
            if view.attributes.iter().any(|a| a.is("timescale")) {
                self.timescale_error(
                    "timescale",
                    format!(
                        "view `{}`: unknown attribute `@@timescale`; expected `@@timescale.continuous`",
                        view.name()
                    ),
                );
            }
            uses_timescale |= view.attributes.iter().any(Attribute::is_timescale);
            match ContinuousAggregateConfig::from_view(view) {
                Ok(Some(cfg)) => {
                    maps.continuous.insert(view.name(), cfg);
                }
                Ok(None) => {}
                Err(e) => self.errors.push(e),
            }
        }

        // Using any `@@timescale.*` attribute requires a PostgreSQL datasource
        // that lists the `timescaledb` extension.
        if uses_timescale {
            match schema.datasource.as_ref() {
                Some(ds)
                    if ds.provider == DatabaseProvider::PostgreSQL
                        && ds.has_extension("timescaledb") => {}
                _ => self.timescale_error(
                    "timescale",
                    "`@@timescale.*` requires `provider = \"postgresql\"` and `timescaledb` in the datasource `extensions`".to_string(),
                ),
            }
        }

        for (name, cfg) in &maps.hypertables {
            if let Some(model) = schema.models.get(*name) {
                self.validate_hypertable(model, cfg);
            }
        }

        for view in schema.views.values() {
            let Some(cfg) = maps.continuous.get(view.name()) else {
                continue;
            };
            self.validate_continuous(view, cfg, schema, &maps);
        }
    }

    /// Validate one hypertable model: time and space fields, unique keys and
    /// columnstore fields.
    fn validate_hypertable(&mut self, model: &Model, cfg: &TimescaleModelConfig) {
        let m = model.name();
        let time = cfg.hypertable.time_field.as_str();

        match model.fields.get(time) {
            None => self.timescale_error(
                "timescale.hypertable",
                format!("model `{m}`: time field `{time}` does not exist"),
            ),
            Some(f) => {
                if !matches!(
                    f.field_type,
                    FieldType::Scalar(ScalarType::DateTime | ScalarType::Date)
                ) {
                    self.timescale_error(
                        "timescale.hypertable",
                        format!("model `{m}`: time field `{time}` must be DateTime (or Date)"),
                    );
                } else if f.is_optional() {
                    self.timescale_error(
                        "timescale.hypertable",
                        format!("model `{m}`: time field `{time}` must be required, not optional"),
                    );
                }
            }
        }

        let space = cfg.hypertable.space.as_ref().map(|s| s.field.as_str());
        if let Some(sf) = space {
            if sf == time {
                self.timescale_error(
                    "timescale.hypertable",
                    format!("model `{m}`: space field `{sf}` must differ from the time field"),
                );
            }
            match model.fields.get(sf) {
                None => self.timescale_error(
                    "timescale.hypertable",
                    format!("model `{m}`: space field `{sf}` does not exist"),
                ),
                Some(f) if !matches!(f.field_type, FieldType::Scalar(_)) => self.timescale_error(
                    "timescale.hypertable",
                    format!("model `{m}`: space field `{sf}` must be a scalar field"),
                ),
                Some(_) => {}
            }
        }

        // TimescaleDB requires every unique index (primary key included) to
        // contain all partitioning columns, i.e. the time field and the space
        // field when one is configured.
        for set in model.unique_key_sets() {
            for required in std::iter::once(time).chain(space) {
                if !set.fields.iter().any(|f| f == required) {
                    self.timescale_error(
                        "timescale.hypertable",
                        format!(
                            "model `{m}`: `{}` ({}) must include hypertable dimension field `{required}`",
                            set.label,
                            set.fields.join(", ")
                        ),
                    );
                }
            }
        }

        // Columnstore segmentBy / orderBy fields must exist and must not overlap.
        if let Some(cs) = &cfg.columnstore {
            for f in &cs.segment_by {
                if !model.fields.contains_key(f.as_str()) {
                    self.timescale_error(
                        "timescale.columnstore",
                        format!("model `{m}`: segmentBy field `{f}` does not exist"),
                    );
                }
            }
            for o in &cs.order_by {
                if !model.fields.contains_key(o.field.as_str()) {
                    self.timescale_error(
                        "timescale.columnstore",
                        format!("model `{m}`: orderBy field `{}` does not exist", o.field),
                    );
                }
                if cs.segment_by.contains(&o.field) {
                    self.timescale_error(
                        "timescale.columnstore",
                        format!(
                            "model `{m}`: field `{}` appears in both segmentBy and orderBy",
                            o.field
                        ),
                    );
                }
            }
        }
    }

    /// Validate one continuous aggregate view: SQL presence, source, source
    /// cycles and the refresh window against the source's retention policy.
    fn validate_continuous(
        &mut self,
        view: &View,
        cfg: &ContinuousAggregateConfig,
        schema: &Schema,
        maps: &TimescaleMaps<'_>,
    ) {
        let v = view.name();
        let attr = "timescale.continuous";

        if view.resolved_sql(schema).is_none() {
            self.timescale_error(
                attr,
                format!("view `{v}`: continuous aggregate requires SQL (`@@sql`)"),
            );
        }

        let source = cfg.source.as_str();
        let is_hypertable = maps.hypertables.contains_key(source);
        if !is_hypertable && !maps.continuous.contains_key(source) {
            self.timescale_error(
                attr,
                format!(
                    "view `{v}`: source `{source}` must be a hypertable model or a continuous aggregate view"
                ),
            );
        }

        // Cycle: walk the view->view source chain back to this view.
        let mut seen = HashSet::new();
        let mut cur = source;
        while let Some(next) = maps.continuous.get(cur) {
            if cur == v {
                self.timescale_error(
                    attr,
                    format!("view `{v}`: continuous aggregate source graph has a cycle"),
                );
                break;
            }
            if !seen.insert(cur) {
                break;
            }
            cur = next.source.as_str();
        }

        // The refresh window must not reach chunks the source's retention
        // policy has already dropped.
        if let Some(retention) = maps
            .hypertables
            .get(source)
            .and_then(|c| c.retention.as_ref())
        {
            match &cfg.start_offset {
                None => self.timescale_error(
                    attr,
                    format!(
                        "view `{v}`: source `{source}` has a retention policy, so `startOffset` is required"
                    ),
                ),
                Some(start) if start.upper_micros() >= retention.drop_after.lower_micros() => self
                    .timescale_error(
                        attr,
                        format!(
                            "view `{v}`: `startOffset` must be shorter than `dropAfter` of source `{source}`, otherwise refreshes cover dropped chunks"
                        ),
                    ),
                Some(_) => {}
            }
        }
    }
}
