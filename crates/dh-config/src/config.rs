//! The resolved, merged configuration: [`Config`].

use serde::de::DeserializeOwned;

use crate::builder::ConfigBuilder;
use crate::de::from_value;
use crate::error::ConfigError;
use crate::value::Value;

/// A fully-resolved configuration: every layer loaded and merged.
///
/// Read it two ways, in any combination:
///
/// * **Typed** — [`Config::deserialize`] the whole tree into your own
///   `serde::Deserialize` struct.
/// * **Dynamic** — [`Config::get`] individual values by dot-separated path.
#[derive(Clone, Debug)]
pub struct Config {
    root: Value,
    profile: Option<String>,
}

impl Config {
    /// Starts a new [`ConfigBuilder`].
    pub fn builder() -> ConfigBuilder {
        ConfigBuilder::new()
    }

    pub(crate) fn from_parts(root: Value, profile: Option<String>) -> Self {
        Config { root, profile }
    }

    /// Deserializes the entire merged configuration into `T`.
    pub fn deserialize<T: DeserializeOwned>(&self) -> Result<T, ConfigError> {
        from_value(&self.root)
    }

    /// Gets the value at a dot-separated path (`"server.port"`,
    /// `"peers.0.host"`), deserialized into `T`. Missing paths are a
    /// [`ConfigError::NotFound`].
    pub fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ConfigError> {
        match self.root.get_path(path) {
            Some(value) => from_value(value).map_err(|e| e.with_path_context(path)),
            None => Err(ConfigError::NotFound {
                path: path.to_string(),
            }),
        }
    }

    /// Like [`Config::get`], but a missing path yields `Ok(None)` instead of
    /// an error. A present-but-wrong-typed value is still an error.
    pub fn get_opt<T: DeserializeOwned>(&self, path: &str) -> Result<Option<T>, ConfigError> {
        match self.root.get_path(path) {
            Some(Value::Null) | None => Ok(None),
            Some(value) => from_value(value)
                .map(Some)
                .map_err(|e| e.with_path_context(path)),
        }
    }

    /// Like [`Config::get`], but a missing path yields `default`.
    pub fn get_or<T: DeserializeOwned>(&self, path: &str, default: T) -> Result<T, ConfigError> {
        Ok(self.get_opt(path)?.unwrap_or(default))
    }

    /// Returns `true` if a value exists at `path`.
    pub fn contains(&self, path: &str) -> bool {
        self.root.get_path(path).is_some()
    }

    /// The raw merged value tree.
    pub fn root(&self) -> &Value {
        &self.root
    }

    /// The profile the configuration was built with, if any.
    pub fn profile(&self) -> Option<&str> {
        self.profile.as_deref()
    }
}
