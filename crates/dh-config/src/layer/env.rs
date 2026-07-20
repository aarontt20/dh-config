//! The environment-variable layer.

use super::{parse_raw_value, Layer, LayerContext};
use crate::error::ConfigError;
use crate::value::Value;

/// Reads configuration from environment variables.
///
/// Variables are filtered by a prefix, and the rest of the name maps to a key
/// path: the nesting separator (default `__`) splits path segments, and
/// segments are lowercased. With prefix `APP`:
///
/// | Variable | Key path |
/// |---|---|
/// | `APP_DEBUG=true` | `debug` |
/// | `APP_SERVER__PORT=8080` | `server.port` |
/// | `APP_DATABASE_URL=…` | `database_url` |
///
/// A single `_` stays part of the key, so `DATABASE_URL` maps to
/// `database_url` — only the double separator descends into nested tables.
///
/// Values are parsed leniently by default: `"true"`/`"false"` become
/// booleans, numeric strings become numbers, everything else stays a string.
/// Disable with [`Env::parse_values`]`(false)`.
///
/// To let a variable carry a whole array (a `Vec` field is otherwise
/// unreachable from the environment), opt into a list separator:
/// [`Env::list_separator`]`(",")` turns `APP_CORS__ORIGINS=a.com, b.com`
/// into `["a.com", "b.com"]`. It is opt-in because commas appear in many
/// legitimate scalar values (URLs, DSNs).
///
/// ```
/// use dh_config::{Config, Env};
///
/// let config = Config::builder()
///     .with_layer(
///         Env::prefixed("APP")
///             .list_separator(",")
///             // `source` replaces `std::env::vars()`; also handy for .env files.
///             .source([("APP_SERVER__PORT", "9000"), ("APP_TAGS", "a, b")]),
///     )
///     .build()?;
/// assert_eq!(config.get::<u16>("server.port")?, 9000);
/// assert_eq!(config.get::<Vec<String>>("tags")?, vec!["a", "b"]);
/// # Ok::<(), dh_config::ConfigError>(())
/// ```
#[derive(Clone, Debug)]
pub struct Env {
    prefix: Option<String>,
    separator: String,
    parse_values: bool,
    list_separator: Option<String>,
    source: Option<Vec<(String, String)>>,
}

impl Env {
    /// Reads **all** environment variables. Prefer [`Env::prefixed`] — an
    /// unprefixed layer pulls in `PATH`, `HOME`, and everything else.
    pub fn raw() -> Self {
        Env {
            prefix: None,
            separator: "__".to_string(),
            parse_values: true,
            list_separator: None,
            source: None,
        }
    }

    /// Reads variables starting with `<prefix>_`, e.g. `Env::prefixed("APP")`
    /// matches `APP_*`. The prefix (and its trailing `_`) is stripped from
    /// the key path.
    pub fn prefixed(prefix: impl Into<String>) -> Self {
        Env {
            prefix: Some(prefix.into()),
            ..Env::raw()
        }
    }

    /// Changes the nesting separator (default `"__"`).
    pub fn separator(mut self, separator: impl Into<String>) -> Self {
        self.separator = separator.into();
        self
    }

    /// Enables or disables lenient scalar parsing (default `true`).
    pub fn parse_values(mut self, parse: bool) -> Self {
        self.parse_values = parse;
        self
    }

    /// Splits values containing `separator` into arrays (elements trimmed),
    /// so `APP_TAGS=a, b` becomes `["a", "b"]`. Off by default.
    pub fn list_separator(mut self, separator: impl Into<String>) -> Self {
        self.list_separator = Some(separator.into());
        self
    }

    /// Replaces `std::env::vars()` with an explicit variable list. Useful for
    /// tests and for feeding in `.env`-file contents.
    pub fn source<K, V>(mut self, vars: impl IntoIterator<Item = (K, V)>) -> Self
    where
        K: Into<String>,
        V: Into<String>,
    {
        self.source = Some(
            vars.into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        );
        self
    }

    fn key_path(&self, name: &str) -> Option<Vec<String>> {
        let unprefixed = match &self.prefix {
            Some(prefix) => {
                let rest = name.strip_prefix(prefix.as_str())?;
                rest.strip_prefix('_')?
            }
            None => name,
        };
        if unprefixed.is_empty() {
            return None;
        }
        let segments: Vec<String> = unprefixed
            .split(self.separator.as_str())
            .map(str::to_lowercase)
            .collect();
        if segments.iter().any(String::is_empty) {
            return None;
        }
        Some(segments)
    }
}

impl Layer for Env {
    fn name(&self) -> String {
        match &self.prefix {
            Some(prefix) => format!("environment ({prefix}_*)"),
            None => "environment".to_string(),
        }
    }

    fn load(&self, _cx: &LayerContext<'_>) -> Result<Value, ConfigError> {
        let vars: Vec<(String, String)> = match &self.source {
            Some(source) => source.clone(),
            None => std::env::vars().collect(),
        };

        let mut root = Value::Table(Default::default());
        for (name, raw) in vars {
            let Some(segments) = self.key_path(&name) else {
                continue;
            };
            let value = parse_raw_value(&raw, self.parse_values, self.list_separator.as_deref());
            root.set_path(&segments.join("."), value);
        }
        Ok(root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(env: Env, vars: &[(&str, &str)]) -> Value {
        env.source(vars.iter().map(|(k, v)| (k.to_string(), v.to_string())))
            .load(&LayerContext::default())
            .unwrap()
    }

    #[test]
    fn prefix_filters_and_strips() {
        let root = load(
            Env::prefixed("APP"),
            &[
                ("APP_DEBUG", "true"),
                ("OTHER_DEBUG", "false"),
                ("APPX_Y", "1"),
            ],
        );
        assert_eq!(root.get_path("debug"), Some(&Value::Bool(true)));
        assert_eq!(root.as_table().unwrap().len(), 1);
    }

    #[test]
    fn double_separator_nests_single_stays() {
        let root = load(
            Env::prefixed("APP"),
            &[("APP_SERVER__PORT", "8080"), ("APP_DATABASE_URL", "pg://x")],
        );
        assert_eq!(root.get_path("server.port"), Some(&Value::Integer(8080)));
        assert_eq!(
            root.get_path("database_url"),
            Some(&Value::String("pg://x".to_string()))
        );
    }

    #[test]
    fn values_parse_leniently() {
        let root = load(
            Env::prefixed("APP"),
            &[
                ("APP_A", "true"),
                ("APP_B", "42"),
                ("APP_C", "2.5"),
                ("APP_D", "plain"),
            ],
        );
        assert_eq!(root.get_path("a"), Some(&Value::Bool(true)));
        assert_eq!(root.get_path("b"), Some(&Value::Integer(42)));
        assert_eq!(root.get_path("c"), Some(&Value::Float(2.5)));
        assert_eq!(
            root.get_path("d"),
            Some(&Value::String("plain".to_string()))
        );
    }

    #[test]
    fn parsing_can_be_disabled() {
        let root = load(Env::prefixed("APP").parse_values(false), &[("APP_B", "42")]);
        assert_eq!(root.get_path("b"), Some(&Value::String("42".to_string())));
    }

    #[test]
    fn list_separator_splits_values() {
        let root = load(
            Env::prefixed("APP").list_separator(","),
            &[
                ("APP_TAGS", "a, b,c"),
                ("APP_PORTS", "1,2"),
                ("APP_ONE", "x"),
            ],
        );
        assert_eq!(
            root.get_path("tags"),
            Some(&Value::Array(vec![
                Value::String("a".into()),
                Value::String("b".into()),
                Value::String("c".into()),
            ]))
        );
        assert_eq!(
            root.get_path("ports"),
            Some(&Value::Array(vec![Value::Integer(1), Value::Integer(2)]))
        );
        assert_eq!(root.get_path("one"), Some(&Value::String("x".into())));
    }

    #[test]
    fn custom_separator() {
        let root = load(
            Env::prefixed("APP").separator("-"),
            &[("APP_SERVER-PORT", "1")],
        );
        assert_eq!(root.get_path("server.port"), Some(&Value::Integer(1)));
    }
}
