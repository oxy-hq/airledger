//! Template interpolation — mirrors `template_interpolator.dart`.
//!
//! Each entry in a template is a record-of-cells, some of which are
//! Jinja strings (`"{{ top * 0.50 }}"`). [`TemplateInterpolator::apply`]
//! renders each string against the user-supplied variable map, then
//! coerces back to the dim's native type via `eval::codec::decode`.
//!
//! minijinja is the Rust equivalent of the Dart `jinja` package.
//! Like the Dart side, we register a `round` filter that does real
//! numeric rounding so templates can express `(top * pct) | round`.

use minijinja::{value::Value as JinjaValue, Environment};
use thiserror::Error;

use crate::eval::codec;
use crate::schema::view::ViewSchema;
use crate::value::{CellValue, Record};

#[derive(Debug, Error)]
pub enum InterpolationError {
    #[error("jinja error: {0}")]
    Jinja(String),
}

pub struct TemplateInterpolator {
    env: Environment<'static>,
}

impl Default for TemplateInterpolator {
    fn default() -> Self {
        let mut env = Environment::empty();
        // Custom `round` filter, matching the Dart custom filter that
        // does real numeric rounding (the default minijinja round
        // filter rounds to a precision, not to nearest integer when
        // given a single arg — they're close but we want exact parity
        // with the Dart side).
        env.add_filter("round", |v: JinjaValue| -> JinjaValue {
            if let Ok(f) = f64::try_from(v.clone()) {
                JinjaValue::from(f.round() as i64)
            } else if let Some(s) = v.as_str() {
                if let Ok(f) = s.parse::<f64>() {
                    JinjaValue::from(f.round() as i64)
                } else {
                    v
                }
            } else {
                v
            }
        });
        Self { env }
    }
}

impl TemplateInterpolator {
    /// Render each entry in `template_entries` against `vars`. Each
    /// string value is run through Jinja; non-strings pass through
    /// untouched. The rendered string is then coerced to the dim's
    /// type via [`codec::decode`] so number-typed fields land as
    /// numbers, not numeric strings.
    pub fn apply(
        &self,
        template_entries: &[Record],
        view: &ViewSchema,
        vars: &Record,
    ) -> Result<Vec<Record>, InterpolationError> {
        template_entries
            .iter()
            .map(|entry| self.apply_one(entry, view, vars))
            .collect()
    }

    fn apply_one(
        &self,
        entry: &Record,
        view: &ViewSchema,
        vars: &Record,
    ) -> Result<Record, InterpolationError> {
        let mut out = Record::new();
        for (field, raw) in entry {
            let rendered = self.render(raw, vars)?;
            let coerced = self.coerce(view, field, &rendered);
            out.insert(field.clone(), coerced);
        }
        Ok(out)
    }

    fn render(
        &self,
        value: &CellValue,
        vars: &Record,
    ) -> Result<CellValue, InterpolationError> {
        let CellValue::String(s) = value else {
            return Ok(value.clone());
        };
        // Skip the Jinja round-trip for strings with no template
        // syntax — matches the Dart fast path.
        if !s.contains("{{") && !s.contains("{%") {
            return Ok(value.clone());
        }
        let ctx = record_to_jinja_value(vars);
        let tpl = self
            .env
            .template_from_str(s)
            .map_err(|e| InterpolationError::Jinja(e.to_string()))?;
        let rendered = tpl
            .render(ctx)
            .map_err(|e| InterpolationError::Jinja(e.to_string()))?;
        Ok(CellValue::String(rendered))
    }

    /// Coerce a rendered cell back into the dim's native type. Jinja
    /// emits strings, so a number dim needs `"95"` → `95`.
    fn coerce(
        &self,
        view: &ViewSchema,
        field: &str,
        rendered: &CellValue,
    ) -> CellValue {
        let Some(dim) = view.dimension_by_name(field) else {
            return rendered.clone();
        };
        match rendered {
            CellValue::String(s) => codec::decode(dim.kind, s),
            other => other.clone(),
        }
    }
}

fn record_to_jinja_value(r: &Record) -> JinjaValue {
    use minijinja::value::Value as V;
    let mut m = std::collections::BTreeMap::<String, V>::new();
    for (k, v) in r {
        m.insert(k.clone(), cell_to_jinja(v));
    }
    V::from(m)
}

fn cell_to_jinja(v: &CellValue) -> JinjaValue {
    use minijinja::value::Value as V;
    match v {
        CellValue::Null => V::from(()),
        CellValue::Bool(b) => V::from(*b),
        CellValue::Int(n) => V::from(*n),
        CellValue::Float(n) => V::from(*n),
        CellValue::String(s) => V::from(s.as_str()),
        CellValue::Date(d) => V::from(d.format("%Y-%m-%d").to_string()),
        CellValue::DateTime(dt) => V::from(dt.format("%Y-%m-%dT%H:%M:%S").to_string()),
    }
}
