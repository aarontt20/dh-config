//! The file formats the library can parse. Each format is behind a cargo
//! feature (`json`, `toml`, `yaml` — all enabled by default) so consumers
//! only compile the parsers they use.

use std::path::Path;

use crate::error::ConfigError;
use crate::value::{Table, Value};

/// A supported configuration file format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Format {
    /// JSON, via `serde_json`. Extension: `.json`.
    #[cfg(feature = "json")]
    Json,
    /// TOML, via the `toml` crate. Extension: `.toml`.
    #[cfg(feature = "toml")]
    Toml,
    /// YAML, via `serde_yaml`. Extensions: `.yaml`, `.yml`.
    #[cfg(feature = "yaml")]
    Yaml,
}

impl Format {
    /// All formats enabled at compile time, in the order file-extension
    /// probing tries them.
    pub const ALL: &'static [Format] = &[
        #[cfg(feature = "toml")]
        Format::Toml,
        #[cfg(feature = "json")]
        Format::Json,
        #[cfg(feature = "yaml")]
        Format::Yaml,
    ];

    /// Maps a file extension (without the dot, case-insensitive) to a format.
    pub fn from_extension(extension: &str) -> Option<Format> {
        match extension.to_ascii_lowercase().as_str() {
            #[cfg(feature = "json")]
            "json" => Some(Format::Json),
            #[cfg(feature = "toml")]
            "toml" => Some(Format::Toml),
            #[cfg(feature = "yaml")]
            "yaml" | "yml" => Some(Format::Yaml),
            _ => None,
        }
    }

    /// The extensions this format is known by, in probing order.
    pub fn extensions(&self) -> &'static [&'static str] {
        match self {
            #[cfg(feature = "json")]
            Format::Json => &["json"],
            #[cfg(feature = "toml")]
            Format::Toml => &["toml"],
            #[cfg(feature = "yaml")]
            Format::Yaml => &["yaml", "yml"],
        }
    }

    /// The lowercase name used in error messages.
    pub fn name(&self) -> &'static str {
        match self {
            #[cfg(feature = "json")]
            Format::Json => "json",
            #[cfg(feature = "toml")]
            Format::Toml => "toml",
            #[cfg(feature = "yaml")]
            Format::Yaml => "yaml",
        }
    }

    /// Parses `content` into a [`Value`].
    ///
    /// Blank content parses as an empty table in every format, so an empty
    /// config file is valid rather than a syntax error. `path` is only used
    /// to enrich error messages.
    pub fn parse(&self, content: &str, path: Option<&Path>) -> Result<Value, ConfigError> {
        if content.trim().is_empty() {
            return Ok(Value::Table(Table::new()));
        }
        let result: Result<Value, String> = match self {
            #[cfg(feature = "json")]
            Format::Json => serde_json::from_str(content).map_err(|e| e.to_string()),
            #[cfg(feature = "toml")]
            Format::Toml => toml::from_str(content).map_err(|e| e.to_string()),
            #[cfg(feature = "yaml")]
            Format::Yaml => serde_yaml::from_str(content).map_err(|e| e.to_string()),
        };
        match result {
            // An explicitly-null document (e.g. YAML `---`) is an empty layer.
            Ok(Value::Null) => Ok(Value::Table(Table::new())),
            Ok(value) => Ok(value),
            Err(message) => Err(ConfigError::Parse {
                format: self.name(),
                path: path.map(Path::to_path_buf),
                message,
            }),
        }
    }
}
