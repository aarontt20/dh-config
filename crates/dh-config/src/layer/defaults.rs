//! The lowest layer: defaults defined in code.

use serde::Serialize;

use super::{Layer, LayerContext};
use crate::error::ConfigError;
use crate::ser::to_value;
use crate::value::Value;

/// Code-defined default values, usually the first layer in a builder.
///
/// Build one from any `Serialize` type — typically the same struct you later
/// deserialize the merged configuration into:
///
/// ```
/// use dh_config::{Config, Defaults};
/// use serde::Serialize;
///
/// #[derive(Serialize)]
/// struct AppDefaults {
///     workers: u32,
///     verbose: bool,
/// }
///
/// let config = Config::builder()
///     .with_layer(Defaults::from_serialize(&AppDefaults { workers: 4, verbose: false })?)
///     .build()?;
/// assert_eq!(config.get::<u32>("workers")?, 4);
/// # Ok::<(), dh_config::ConfigError>(())
/// ```
///
/// Or assemble it key by key with [`Defaults::set`]:
///
/// ```
/// use dh_config::{Config, Defaults};
///
/// let config = Config::builder()
///     .with_layer(Defaults::new().set("server.port", 8080))
///     .build()?;
/// assert_eq!(config.get::<u16>("server.port")?, 8080);
/// # Ok::<(), dh_config::ConfigError>(())
/// ```
#[derive(Clone, Debug, Default)]
pub struct Defaults {
    root: Value,
}

impl Defaults {
    /// Creates an empty defaults layer.
    pub fn new() -> Self {
        Defaults {
            root: Value::Table(Default::default()),
        }
    }

    /// Serializes `value` (any `Serialize` type) into a defaults layer.
    pub fn from_serialize<T: Serialize + ?Sized>(value: &T) -> Result<Self, ConfigError> {
        Ok(Defaults {
            root: to_value(value)?,
        })
    }

    /// Wraps an already-built [`Value`] tree.
    pub fn from_value(root: impl Into<Value>) -> Self {
        Defaults { root: root.into() }
    }

    /// Sets a default at a dot-separated path.
    pub fn set(mut self, path: &str, value: impl Into<Value>) -> Self {
        self.root.set_path(path, value.into());
        self
    }
}

impl Layer for Defaults {
    fn name(&self) -> String {
        "defaults".to_string()
    }

    fn load(&self, _cx: &LayerContext<'_>) -> Result<Value, ConfigError> {
        Ok(self.root.clone())
    }
}
