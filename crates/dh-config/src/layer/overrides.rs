//! Programmatic override layers: explicit key/value overrides and a
//! dependency-free command-line layer.

use super::{parse_scalar, Layer, LayerContext};
use crate::error::ConfigError;
use crate::value::Value;

/// Explicit key/value overrides, usually the highest-precedence layer.
///
/// This is the escape hatch for anything the built-in layers don't cover:
/// values computed at startup, flags from your own argument parser, test
/// overrides, and so on.
///
/// ```
/// use dh_config::{Config, Overrides};
///
/// let config = Config::builder()
///     .with_layer(Overrides::new().set("server.port", 3000))
///     .build()?;
/// assert_eq!(config.get::<u16>("server.port")?, 3000);
/// # Ok::<(), dh_config::ConfigError>(())
/// ```
#[derive(Clone, Debug, Default)]
pub struct Overrides {
    root: Value,
}

impl Overrides {
    /// Creates an empty override layer.
    pub fn new() -> Self {
        Overrides {
            root: Value::Table(Default::default()),
        }
    }

    /// Sets a value at a dot-separated path.
    pub fn set(mut self, path: &str, value: impl Into<Value>) -> Self {
        self.root.set_path(path, value.into());
        self
    }
}

impl Layer for Overrides {
    fn name(&self) -> String {
        "overrides".to_string()
    }

    fn load(&self, _cx: &LayerContext<'_>) -> Result<Value, ConfigError> {
        Ok(self.root.clone())
    }
}

/// A command-line layer that needs no argument-parsing dependency.
///
/// Recognizes `--dotted.key=value` anywhere in the argument list; a bare
/// `--flag` becomes `true`. Everything else (positional arguments, flags for
/// your real CLI parser) is ignored, so this layer can coexist with
/// hand-rolled argument handling. Values get the same lenient scalar parsing
/// as the env layer.
///
/// Space-separated values (`--key value`) are deliberately *not* supported:
/// without a flag schema, `--flag positional` and `--key value` are
/// indistinguishable, and a config layer should never guess. Use `=`.
///
/// ```
/// use dh_config::{CommandLine, Config};
///
/// let args = ["--server.port=9090", "--verbose", "positional"];
/// let config = Config::builder()
///     .with_layer(CommandLine::from_args(args.iter().map(|s| s.to_string())))
///     .build()?;
/// assert_eq!(config.get::<u16>("server.port")?, 9090);
/// assert!(config.get::<bool>("verbose")?);
/// # Ok::<(), dh_config::ConfigError>(())
/// ```
#[derive(Clone, Debug)]
pub struct CommandLine {
    args: Vec<String>,
}

impl CommandLine {
    /// Captures the process arguments (`std::env::args()`, program name
    /// skipped).
    pub fn from_env() -> Self {
        CommandLine {
            args: std::env::args().skip(1).collect(),
        }
    }

    /// Uses an explicit argument list instead of the process arguments.
    pub fn from_args(args: impl IntoIterator<Item = String>) -> Self {
        CommandLine {
            args: args.into_iter().collect(),
        }
    }
}

impl Layer for CommandLine {
    fn name(&self) -> String {
        "command line".to_string()
    }

    fn load(&self, _cx: &LayerContext<'_>) -> Result<Value, ConfigError> {
        let mut root = Value::Table(Default::default());
        for arg in &self.args {
            // `--` conventionally ends option parsing.
            if arg == "--" {
                break;
            }
            let Some(key) = arg.strip_prefix("--") else {
                continue;
            };
            let (key, value) = match key.split_once('=') {
                Some((key, value)) => (key, parse_scalar(value)),
                // Bare `--flag` is a boolean.
                None => (key, Value::Bool(true)),
            };
            if key.is_empty() || key.split('.').any(str::is_empty) {
                continue;
            }
            root.set_path(key, value);
        }
        Ok(root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(args: &[&str]) -> Value {
        CommandLine::from_args(args.iter().map(|s| s.to_string()))
            .load(&LayerContext::default())
            .unwrap()
    }

    #[test]
    fn equals_separated_values() {
        let root = load(&["--a.b=1", "--c.d=two"]);
        assert_eq!(root.get_path("a.b"), Some(&Value::Integer(1)));
        assert_eq!(
            root.get_path("c.d"),
            Some(&Value::String("two".to_string()))
        );
    }

    #[test]
    fn bare_flags_are_true() {
        let root = load(&["--verbose", "--dry.run", "positional", "--next=1"]);
        assert_eq!(root.get_path("verbose"), Some(&Value::Bool(true)));
        // Space-separated values are not supported, so the positional does
        // not become `dry.run`'s value.
        assert_eq!(root.get_path("dry.run"), Some(&Value::Bool(true)));
        assert_eq!(root.get_path("next"), Some(&Value::Integer(1)));
    }

    #[test]
    fn positionals_and_terminator_ignored() {
        let root = load(&["positional", "--a=1", "--", "--b=2"]);
        assert_eq!(root.get_path("a"), Some(&Value::Integer(1)));
        assert_eq!(root.get_path("b"), None);
    }

    #[test]
    fn empty_value_stays_string() {
        let root = load(&["--name="]);
        assert_eq!(root.get_path("name"), Some(&Value::String(String::new())));
    }
}
