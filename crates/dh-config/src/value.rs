//! The dynamically-typed value tree that every layer produces and that the
//! builder merges.

use std::collections::BTreeMap;
use std::fmt;

/// A map of string keys to values. `BTreeMap` keeps iteration order stable,
/// which makes merged output and error messages deterministic.
pub type Table = BTreeMap<String, Value>;

/// A configuration value.
///
/// Every layer loads into this tree, and the builder deep-merges the trees in
/// layer order. Tables merge key-by-key; every other kind of value (including
/// arrays) is replaced wholesale by the higher-precedence layer.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Value {
    /// An explicit null (e.g. JSON `null`, empty YAML value).
    #[default]
    Null,
    /// A boolean.
    Bool(bool),
    /// A signed 64-bit integer.
    Integer(i64),
    /// A 64-bit float.
    Float(f64),
    /// A string.
    String(String),
    /// An ordered list of values.
    Array(Vec<Value>),
    /// A string-keyed map of values.
    Table(Table),
}

impl Value {
    /// A human-readable name for the kind of value, used in error messages.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Integer(_) => "integer",
            Value::Float(_) => "float",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Table(_) => "table",
        }
    }

    /// Returns the table if this value is one.
    pub fn as_table(&self) -> Option<&Table> {
        match self {
            Value::Table(t) => Some(t),
            _ => None,
        }
    }

    /// Returns `true` for [`Value::Null`].
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// Looks up a value by a dot-separated path such as `"server.port"`.
    ///
    /// Path segments index into tables by key and into arrays by integer
    /// position (`"peers.0.host"`).
    pub fn get_path(&self, path: &str) -> Option<&Value> {
        let mut current = self;
        for segment in path.split('.') {
            current = match current {
                Value::Table(table) => table.get(segment)?,
                Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
                _ => return None,
            };
        }
        Some(current)
    }

    /// Sets a value at a dot-separated path, creating intermediate tables as
    /// needed. Existing non-table values along the path are replaced by
    /// tables.
    pub fn set_path(&mut self, path: &str, value: Value) {
        let mut current = self;
        let mut segments = path.split('.').peekable();
        while let Some(segment) = segments.next() {
            if !matches!(current, Value::Table(_)) {
                *current = Value::Table(Table::new());
            }
            let Value::Table(table) = current else {
                unreachable!()
            };
            let slot = table.entry(segment.to_string()).or_insert(Value::Null);
            if segments.peek().is_none() {
                *slot = value;
                return;
            }
            current = slot;
        }
    }

    /// Deep-merges `other` into `self`.
    ///
    /// Tables merge recursively; any other pairing means `other` replaces
    /// `self`. An explicit `Null` in `other` also replaces — a higher layer
    /// can deliberately null out a lower layer's value.
    pub fn merge_from(&mut self, other: Value) {
        match (self, other) {
            (Value::Table(base), Value::Table(overlay)) => {
                for (key, value) in overlay {
                    match base.get_mut(&key) {
                        Some(slot) => slot.merge_from(value),
                        None => {
                            base.insert(key, value);
                        }
                    }
                }
            }
            (slot, other) => *slot = other,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => f.write_str("null"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Integer(i) => write!(f, "{i}"),
            Value::Float(x) => write!(f, "{x}"),
            Value::String(s) => f.write_str(s),
            Value::Array(items) => {
                f.write_str("[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{item}")?;
                }
                f.write_str("]")
            }
            Value::Table(table) => {
                f.write_str("{")?;
                for (i, (key, value)) in table.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{key}: {value}")?;
                }
                f.write_str("}")
            }
        }
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}

macro_rules! from_int {
    ($($ty:ty),*) => {
        $(impl From<$ty> for Value {
            fn from(v: $ty) -> Self {
                Value::Integer(v as i64)
            }
        })*
    };
}
from_int!(i8, i16, i32, i64, u8, u16, u32);

impl From<f32> for Value {
    fn from(v: f32) -> Self {
        Value::Float(v as f64)
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Float(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::String(v.to_string())
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::String(v)
    }
}

impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(v: Vec<T>) -> Self {
        Value::Array(v.into_iter().map(Into::into).collect())
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Self {
        match v {
            Some(v) => v.into(),
            None => Value::Null,
        }
    }
}

impl From<Table> for Value {
    fn from(v: Table) -> Self {
        Value::Table(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(entries: &[(&str, Value)]) -> Value {
        Value::Table(
            entries
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
        )
    }

    #[test]
    fn tables_merge_recursively() {
        let mut base = table(&[
            (
                "server",
                table(&[("host", "localhost".into()), ("port", 80.into())]),
            ),
            ("debug", false.into()),
        ]);
        base.merge_from(table(&[("server", table(&[("port", 8080.into())]))]));

        assert_eq!(base.get_path("server.host"), Some(&"localhost".into()));
        assert_eq!(base.get_path("server.port"), Some(&8080.into()));
        assert_eq!(base.get_path("debug"), Some(&false.into()));
    }

    #[test]
    fn scalars_and_arrays_replace() {
        let mut base = table(&[("tags", vec!["a", "b"].into())]);
        base.merge_from(table(&[("tags", vec!["c"].into())]));
        assert_eq!(base.get_path("tags"), Some(&vec!["c"].into()));

        let mut scalar = Value::from(1);
        scalar.merge_from(Value::from("two"));
        assert_eq!(scalar, "two".into());
    }

    #[test]
    fn explicit_null_overrides() {
        let mut base = table(&[("key", "value".into())]);
        base.merge_from(table(&[("key", Value::Null)]));
        assert_eq!(base.get_path("key"), Some(&Value::Null));
    }

    #[test]
    fn path_lookup_indexes_arrays() {
        let root = table(&[(
            "peers",
            Value::Array(vec![
                table(&[("host", "a".into())]),
                table(&[("host", "b".into())]),
            ]),
        )]);
        assert_eq!(root.get_path("peers.1.host"), Some(&"b".into()));
        assert_eq!(root.get_path("peers.2.host"), None);
        assert_eq!(root.get_path("peers.x"), None);
    }

    #[test]
    fn set_path_creates_intermediate_tables() {
        let mut root = Value::Table(Table::new());
        root.set_path("a.b.c", 1.into());
        assert_eq!(root.get_path("a.b.c"), Some(&1.into()));

        root.set_path("a.b", "replaced".into());
        assert_eq!(root.get_path("a.b"), Some(&"replaced".into()));
    }
}
