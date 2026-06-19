//! show_when predicate evaluation — mirrors `Dimension.isVisibleGiven`
//! and `_evalPredicate` from `view_schema.dart`.
//!
//! The YAML grammar:
//!
//! ```yaml
//! show_when:
//!   <other_field>: <predicate>      # AND across entries
//! ```
//!
//! A `<predicate>` is one of:
//! - scalar (`string` / `number` / `bool`) — exact-string match
//! - list — implicit "in this list"
//! - map of operators — `eq` / `in` / `not_in` / `in_group` /
//!   `not_in_group`, all AND'd
//!
//! Comparisons are loose: actual values get `.to_display_string()`'d
//! before comparing, so `123` (int) matches `"123"` (string).

use std::collections::{BTreeMap, BTreeSet};

use serde_yaml::{Mapping, Value};

use crate::value::Record;

/// True when the dim should be visible given the current `values` and
/// `groups`. `show_when` is the raw mapping from the dim's overlay.
/// Always-visible (returns `true`) when `show_when` is `None`.
pub fn is_visible_given(
    show_when: Option<&Mapping>,
    values: &Record,
    groups: &BTreeMap<String, BTreeSet<String>>,
) -> bool {
    let Some(sw) = show_when else {
        return true;
    };
    for (k, predicate) in sw {
        let Some(key) = k.as_str() else { continue };
        let actual = values
            .get(key)
            .filter(|v| !v.is_empty())
            .map(|v| v.to_display_string());
        if !eval_predicate(predicate, actual.as_deref(), groups) {
            return false;
        }
    }
    true
}

fn eval_predicate(
    pred: &Value,
    actual: Option<&str>,
    groups: &BTreeMap<String, BTreeSet<String>>,
) -> bool {
    match pred {
        // Null predicate: matches only when there's no value.
        Value::Null => actual.is_none(),
        // Scalar match — stringify both sides and compare.
        Value::String(s) => actual == Some(s.as_str()),
        Value::Number(n) => actual == Some(n.to_string().as_str()),
        Value::Bool(b) => actual == Some(b.to_string().as_str()),
        // List = implicit `in`.
        Value::Sequence(list) => {
            let Some(a) = actual else { return false };
            list.iter().any(|v| value_to_string(v) == a)
        }
        // Operator map.
        Value::Mapping(m) => eval_operator_map(m, actual, groups),
        _ => false,
    }
}

fn eval_operator_map(
    m: &Mapping,
    actual: Option<&str>,
    groups: &BTreeMap<String, BTreeSet<String>>,
) -> bool {
    if let Some(v) = m.get(Value::String("eq".into())) {
        if Some(value_to_string(v).as_str()) != actual {
            return false;
        }
    }
    if let Some(v) = m.get(Value::String("in".into())) {
        let Some(seq) = v.as_sequence() else { return false };
        let set: BTreeSet<String> = seq.iter().map(value_to_string).collect();
        let Some(a) = actual else { return false };
        if !set.contains(a) {
            return false;
        }
    }
    if let Some(v) = m.get(Value::String("not_in".into())) {
        let Some(seq) = v.as_sequence() else { return false };
        let set: BTreeSet<String> = seq.iter().map(value_to_string).collect();
        if let Some(a) = actual {
            if set.contains(a) {
                return false;
            }
        }
    }
    if let Some(v) = m.get(Value::String("in_group".into())) {
        let union = group_union(v, groups);
        let Some(a) = actual else { return false };
        if !union.contains(a) {
            return false;
        }
    }
    if let Some(v) = m.get(Value::String("not_in_group".into())) {
        let union = group_union(v, groups);
        if let Some(a) = actual {
            if union.contains(a) {
                return false;
            }
        }
    }
    true
}

fn group_union(
    v: &Value,
    groups: &BTreeMap<String, BTreeSet<String>>,
) -> BTreeSet<String> {
    let names: Vec<String> = match v {
        Value::String(s) => vec![s.clone()],
        Value::Sequence(seq) => seq.iter().map(value_to_string).collect(),
        _ => return BTreeSet::new(),
    };
    let mut out = BTreeSet::new();
    for n in names {
        if let Some(set) = groups.get(&n) {
            out.extend(set.iter().cloned());
        }
    }
    out
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        other => format!("{other:?}"),
    }
}
