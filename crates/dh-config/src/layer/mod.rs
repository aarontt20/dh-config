//! The layer abstraction and the built-in layers.
//!
//! A [`Layer`] is anything that can produce a [`Value`] tree. The builder
//! merges layers in the order they were added — later layers win. Implement
//! the trait yourself to source configuration from anywhere (a database, a
//! remote service, hand-parsed CLI flags, …).

mod defaults;
mod env;
#[cfg(any(feature = "json", feature = "toml", feature = "yaml"))]
mod file;
mod overrides;

pub use defaults::Defaults;
pub use env::Env;
#[cfg(any(feature = "json", feature = "toml", feature = "yaml"))]
pub use file::File;
pub use overrides::{CommandLine, Overrides};

use crate::error::ConfigError;
use crate::value::Value;

/// Context passed to every layer when it loads.
///
/// Carries builder-level state that layers may react to — today that is the
/// active profile (used by [`File`] to load `app.<profile>.toml` overlays).
#[derive(Clone, Copy, Debug, Default)]
pub struct LayerContext<'a> {
    /// The active profile (e.g. `"prod"`), if one was selected on the builder.
    pub profile: Option<&'a str>,
}

/// A source of configuration values.
///
/// # Implementing a custom layer
///
/// ```
/// use dh_config::{Config, ConfigError, Layer, LayerContext, Value};
///
/// /// Reads a single key from a (pretend) remote service.
/// struct RemoteFlags;
///
/// impl Layer for RemoteFlags {
///     fn name(&self) -> String {
///         "remote-flags".to_string()
///     }
///
///     fn load(&self, _cx: &LayerContext<'_>) -> Result<Value, ConfigError> {
///         let mut root = Value::Table(Default::default());
///         root.set_path("features.new_ui", Value::Bool(true));
///         Ok(root)
///     }
/// }
///
/// let config = Config::builder().with_layer(RemoteFlags).build()?;
/// assert!(config.get::<bool>("features.new_ui")?);
/// # Ok::<(), ConfigError>(())
/// ```
pub trait Layer {
    /// A human-readable name for the layer, used in error messages.
    fn name(&self) -> String;

    /// Produces this layer's value tree. The root must be a
    /// [`Value::Table`] (or [`Value::Null`], treated as empty) so it can be
    /// merged with other layers.
    fn load(&self, cx: &LayerContext<'_>) -> Result<Value, ConfigError>;
}

/// Parses a scalar string the way the env and CLI layers do: `true`/`false`
/// become booleans, then integers, then floats, and anything else stays a
/// string.
pub(crate) fn parse_scalar(raw: &str) -> Value {
    match raw {
        "true" => return Value::Bool(true),
        "false" => return Value::Bool(false),
        _ => {}
    }
    if let Ok(i) = raw.parse::<i64>() {
        return Value::Integer(i);
    }
    if let Ok(f) = raw.parse::<f64>() {
        return Value::Float(f);
    }
    Value::String(raw.to_string())
}
