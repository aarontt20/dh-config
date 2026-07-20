//! The programmatic layer stack: [`ConfigBuilder`].

use serde::Serialize;

use crate::config::Config;
use crate::error::ConfigError;
use crate::layer::{Defaults, Env, Layer, LayerContext};
use crate::value::{Table, Value};

/// Where the active profile comes from.
enum ProfileSource {
    None,
    /// Set explicitly with [`ConfigBuilder::profile`].
    Explicit(String),
    /// Read from an environment variable at [`ConfigBuilder::build`] time.
    EnvVar(String),
}

/// Builds a [`Config`] from an ordered stack of layers.
///
/// Layers are merged in the order they are added: **later layers override
/// earlier ones**. The conventional stack, lowest to highest precedence, is
/// defaults → files → environment → command line / overrides.
///
/// ```no_run
/// use dh_config::{CommandLine, Config, Defaults, Env};
///
/// let config = Config::builder()
///     .with_layer(Defaults::new().set("server.port", 8080))
///     .with_file("config/app.toml")
///     .with_env("APP")
///     .with_layer(CommandLine::from_env())
///     .profile_from_env("APP_PROFILE")
///     .build()?;
/// # Ok::<(), dh_config::ConfigError>(())
/// ```
#[must_use]
pub struct ConfigBuilder {
    layers: Vec<Box<dyn Layer>>,
    profile: ProfileSource,
    /// Errors from infallible-looking builder methods (e.g. serializing
    /// defaults), deferred so the chain stays ergonomic and surfaced at
    /// [`ConfigBuilder::build`].
    deferred_error: Option<ConfigError>,
}

impl ConfigBuilder {
    /// Creates a builder with no layers.
    pub fn new() -> Self {
        ConfigBuilder {
            layers: Vec::new(),
            profile: ProfileSource::None,
            deferred_error: None,
        }
    }

    /// Adds any [`Layer`] — built-in or custom — as the next (higher
    /// precedence) layer.
    pub fn with_layer(mut self, layer: impl Layer + 'static) -> Self {
        self.layers.push(Box::new(layer));
        self
    }

    /// Adds a defaults layer serialized from `value`. Serialization errors
    /// are deferred and reported by [`ConfigBuilder::build`].
    pub fn with_defaults<T: Serialize + ?Sized>(mut self, value: &T) -> Self {
        match Defaults::from_serialize(value) {
            Ok(layer) => self.with_layer(layer),
            Err(error) => {
                self.deferred_error.get_or_insert(error);
                self
            }
        }
    }

    /// Adds a file layer (see [`File`](crate::File) for format inference,
    /// extension probing, and profile overlays). Accepts anything convertible
    /// to a `File`, so a plain path and a configured layer both work:
    ///
    /// ```no_run
    /// # use dh_config::{Config, File};
    /// let config = Config::builder()
    ///     .with_file("config/app.toml")
    ///     .with_file(File::new("config/local.toml").required(false))
    ///     .build()?;
    /// # Ok::<(), dh_config::ConfigError>(())
    /// ```
    #[cfg(any(feature = "json", feature = "toml", feature = "yaml"))]
    pub fn with_file(self, file: impl Into<crate::layer::File>) -> Self {
        self.with_layer(file.into())
    }

    /// Adds an environment layer for variables prefixed `<prefix>_` (see
    /// [`Env`]). For a custom separator or unprefixed matching, add a
    /// configured `Env` via [`with_layer`](ConfigBuilder::with_layer).
    pub fn with_env(self, prefix: impl Into<String>) -> Self {
        self.with_layer(Env::prefixed(prefix))
    }

    /// Selects the active profile (e.g. `"prod"`), enabling profile overlay
    /// files. Overrides any [`profile_from_env`](ConfigBuilder::profile_from_env).
    pub fn profile(mut self, profile: impl Into<String>) -> Self {
        self.profile = ProfileSource::Explicit(profile.into());
        self
    }

    /// Reads the active profile from an environment variable at build time.
    /// An unset or empty variable means no profile.
    pub fn profile_from_env(mut self, var: impl Into<String>) -> Self {
        if !matches!(self.profile, ProfileSource::Explicit(_)) {
            self.profile = ProfileSource::EnvVar(var.into());
        }
        self
    }

    /// Builds the configuration and deserializes it straight into `T` — the
    /// shorthand for the common case where the typed struct is all you want.
    ///
    /// ```no_run
    /// # use dh_config::Config;
    /// # use serde::Deserialize;
    /// #[derive(Deserialize)]
    /// struct AppConfig {
    ///     workers: u32,
    /// }
    ///
    /// let app: AppConfig = Config::builder()
    ///     .with_file("config/app.toml")
    ///     .with_env("APP")
    ///     .extract()?;
    /// # Ok::<(), dh_config::ConfigError>(())
    /// ```
    ///
    /// Use [`build`](ConfigBuilder::build) instead when you also need dynamic
    /// lookups, the active profile, or provenance queries.
    pub fn extract<T: serde::de::DeserializeOwned>(self) -> Result<T, ConfigError> {
        self.build()?.deserialize()
    }

    /// Loads every layer, merges them in order, and returns the resolved
    /// [`Config`].
    pub fn build(self) -> Result<Config, ConfigError> {
        if let Some(error) = self.deferred_error {
            return Err(error);
        }

        let profile = match self.profile {
            ProfileSource::None => None,
            ProfileSource::Explicit(profile) => Some(profile),
            ProfileSource::EnvVar(var) => std::env::var(&var).ok().filter(|p| !p.is_empty()),
        };
        let cx = LayerContext {
            profile: profile.as_deref(),
        };

        let mut root = Value::Table(Table::new());
        let mut layer_names = Vec::with_capacity(self.layers.len());
        let mut origins = std::collections::BTreeMap::new();
        for layer in &self.layers {
            let loaded = layer.load(&cx)?;
            let layer_index = layer_names.len();
            layer_names.push(layer.name());
            match loaded {
                // An empty layer contributes nothing.
                Value::Null => {}
                table @ Value::Table(_) => {
                    record_origins(&table, "", layer_index, &mut origins);
                    root.merge_from(table);
                }
                other => {
                    return Err(ConfigError::InvalidRoot {
                        layer: layer.name(),
                        found: other.type_name(),
                    })
                }
            }
        }
        Ok(Config::from_parts(root, profile, layer_names, origins))
    }
}

/// Records, for every leaf (non-table) value in a layer's tree, the index of
/// the layer that supplied it. Later layers overwrite earlier entries, which
/// mirrors the merge: tables merge (so their leaves are tracked one by one)
/// and everything else replaces.
fn record_origins(
    value: &Value,
    prefix: &str,
    layer_index: usize,
    origins: &mut std::collections::BTreeMap<String, usize>,
) {
    let Value::Table(table) = value else {
        return;
    };
    for (key, child) in table {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match child {
            Value::Table(_) => record_origins(child, &path, layer_index, origins),
            _ => {
                origins.insert(path, layer_index);
            }
        }
    }
}

impl Default for ConfigBuilder {
    fn default() -> Self {
        ConfigBuilder::new()
    }
}
