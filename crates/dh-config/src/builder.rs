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

    /// Adds a required file layer (see [`File`](crate::File) for format
    /// inference, extension probing, and profile overlays). For an optional
    /// file or other tweaks, add a configured `File` via
    /// [`with_layer`](ConfigBuilder::with_layer).
    #[cfg(any(feature = "json", feature = "toml", feature = "yaml"))]
    pub fn with_file(self, path: impl Into<std::path::PathBuf>) -> Self {
        self.with_layer(crate::layer::File::new(path))
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
        for layer in &self.layers {
            let loaded = layer.load(&cx)?;
            match loaded {
                // An empty layer contributes nothing.
                Value::Null => {}
                table @ Value::Table(_) => root.merge_from(table),
                other => {
                    return Err(ConfigError::InvalidRoot {
                        layer: layer.name(),
                        found: other.type_name(),
                    })
                }
            }
        }
        Ok(Config::from_parts(root, profile))
    }
}

impl Default for ConfigBuilder {
    fn default() -> Self {
        ConfigBuilder::new()
    }
}
