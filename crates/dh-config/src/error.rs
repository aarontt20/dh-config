//! The error type shared by every part of the library.

use std::fmt;
use std::path::PathBuf;

/// Everything that can go wrong while building or reading configuration.
#[derive(Debug)]
pub enum ConfigError {
    /// A file layer failed to read its file.
    Io {
        /// The file that could not be read.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// A file's contents could not be parsed.
    Parse {
        /// The format that was being parsed (`"json"`, `"toml"`, `"yaml"`).
        format: &'static str,
        /// The file being parsed, when the source was a file.
        path: Option<PathBuf>,
        /// The parser's error message.
        message: String,
    },
    /// A file's format could not be determined, or the matching cargo
    /// feature is disabled.
    UnknownFormat {
        /// The file whose format was unrecognized.
        path: PathBuf,
    },
    /// A layer produced a root value that is not a table. Layers must produce
    /// maps so they can be merged.
    InvalidRoot {
        /// The name of the offending layer.
        layer: String,
        /// The kind of value the layer produced.
        found: &'static str,
    },
    /// A custom layer failed to load.
    Layer {
        /// The name of the layer that failed.
        layer: String,
        /// The layer's error message.
        message: String,
    },
    /// A requested key path does not exist in the merged configuration.
    NotFound {
        /// The dot-separated key path that was requested.
        path: String,
    },
    /// A value exists but cannot be converted to the requested type.
    TypeMismatch {
        /// The dot-separated path of the offending value (empty for the root).
        path: String,
        /// What the caller asked for.
        expected: String,
        /// The kind of value that was actually there.
        found: &'static str,
        /// The layer that supplied the offending value, when known.
        origin: Option<String>,
    },
    /// A free-form error, mostly produced by serde during (de)serialization.
    Message(String),
}

impl ConfigError {
    /// Annotates serde-produced errors with the path the deserializer is at.
    /// `TypeMismatch` already carries its own (absolute) path, so only an
    /// empty one is filled in; free-form messages are wrapped once.
    pub(crate) fn with_path_context(self, path: &str) -> Self {
        if path.is_empty() {
            return self;
        }
        match self {
            // Nested `deserialize_any` calls each apply context as the error
            // bubbles up; only the innermost (deepest path) wrap is kept.
            ConfigError::Message(message) if !message.starts_with("at `") => {
                ConfigError::Message(format!("at `{path}`: {message}"))
            }
            ConfigError::TypeMismatch {
                path: inner,
                expected,
                found,
                origin,
            } if inner.is_empty() => ConfigError::TypeMismatch {
                path: path.to_string(),
                expected,
                found,
                origin,
            },
            other => other,
        }
    }

    /// Re-roots an error produced while deserializing the subtree at `base`
    /// (a `Config::get` lookup), so its path is absolute: a failure at
    /// `port` inside `get("server")` reports `server.port`.
    pub(crate) fn prefixed_with(self, base: &str) -> Self {
        match self {
            ConfigError::TypeMismatch {
                path,
                expected,
                found,
                origin,
            } => {
                let path = if path.is_empty() {
                    base.to_string()
                } else {
                    format!("{base}.{path}")
                };
                ConfigError::TypeMismatch {
                    path,
                    expected,
                    found,
                    origin,
                }
            }
            ConfigError::Message(message) => {
                ConfigError::Message(format!("at `{base}`: {message}"))
            }
            other => other,
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Io { path, source } => {
                write!(f, "failed to read `{}`: {source}", path.display())
            }
            ConfigError::Parse {
                format,
                path,
                message,
            } => match path {
                Some(path) => write!(
                    f,
                    "failed to parse `{}` as {format}: {message}",
                    path.display()
                ),
                None => write!(f, "failed to parse {format}: {message}"),
            },
            ConfigError::UnknownFormat { path } => write!(
                f,
                "cannot determine the configuration format of `{}` \
                 (unrecognized extension, or the matching cargo feature is disabled)",
                path.display()
            ),
            ConfigError::InvalidRoot { layer, found } => write!(
                f,
                "layer `{layer}` produced a {found} at its root; \
                 layers must produce a table so they can be merged"
            ),
            ConfigError::Layer { layer, message } => {
                write!(f, "layer `{layer}` failed to load: {message}")
            }
            ConfigError::NotFound { path } => {
                write!(f, "configuration key `{path}` was not found")
            }
            ConfigError::TypeMismatch {
                path,
                expected,
                found,
                origin,
            } => {
                if path.is_empty() {
                    write!(f, "expected {expected}, found {found}")?;
                } else {
                    write!(f, "at `{path}`: expected {expected}, found {found}")?;
                }
                if let Some(origin) = origin {
                    write!(f, " (value set by layer `{origin}`)")?;
                }
                Ok(())
            }
            ConfigError::Message(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConfigError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl serde::de::Error for ConfigError {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        ConfigError::Message(msg.to_string())
    }
}

impl serde::ser::Error for ConfigError {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        ConfigError::Message(msg.to_string())
    }
}
