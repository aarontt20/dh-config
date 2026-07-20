//! The file layer: JSON/TOML/YAML files with profile overlays.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use super::{Layer, LayerContext};
use crate::error::ConfigError;
use crate::format::Format;
use crate::value::{Table, Value};

/// A configuration file.
///
/// The format is inferred from the file extension unless set explicitly with
/// [`File::format`]. A path *without* an extension probes every enabled
/// format's extensions (`app` tries `app.toml`, `app.json`, `app.yaml`,
/// `app.yml`) and loads the first that exists.
///
/// Files are **required by default** — a missing file is a build error.
/// Optional files (e.g. a developer's `local.toml`) opt out with
/// [`File::required`]`(false)`.
///
/// # Profile overlays
///
/// When the builder has an active profile (see
/// [`ConfigBuilder::profile`](crate::ConfigBuilder::profile)), each file
/// layer also loads a profile-suffixed sibling — `config/app.toml` with
/// profile `prod` additionally loads `config/app.prod.toml` — and merges it
/// over the base file. Overlay files are always optional, and the behavior
/// can be turned off per file with [`File::profile_variants`]`(false)`.
#[derive(Clone, Debug)]
pub struct File {
    path: PathBuf,
    format: Option<Format>,
    required: bool,
    profile_variants: bool,
}

impl File {
    /// Points the layer at `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        File {
            path: path.into(),
            format: None,
            required: true,
            profile_variants: true,
        }
    }

    /// Forces a format instead of inferring it from the extension.
    pub fn format(mut self, format: Format) -> Self {
        self.format = Some(format);
        self
    }

    /// Sets whether a missing file is an error (default `true`).
    pub fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    /// Sets whether profile-suffixed overlay files are loaded when a profile
    /// is active (default `true`).
    pub fn profile_variants(mut self, enabled: bool) -> Self {
        self.profile_variants = enabled;
        self
    }

    /// Resolves the concrete path + format to load. For extensionless paths,
    /// probes each enabled format's extensions and picks the first existing
    /// file (`Ok(None)` if none exists).
    fn resolve(&self) -> Result<Option<(PathBuf, Format)>, ConfigError> {
        if let Some(format) = self.format {
            return Ok(Some((self.path.clone(), format)));
        }
        match self.path.extension().and_then(|e| e.to_str()) {
            Some(extension) => match Format::from_extension(extension) {
                Some(format) => Ok(Some((self.path.clone(), format))),
                None => Err(ConfigError::UnknownFormat {
                    path: self.path.clone(),
                }),
            },
            None => {
                for format in Format::ALL {
                    for extension in format.extensions() {
                        let candidate = self.path.with_extension(extension);
                        if candidate.is_file() {
                            return Ok(Some((candidate, *format)));
                        }
                    }
                }
                Ok(None)
            }
        }
    }

    fn read_and_parse(path: &Path, format: Format) -> Result<Option<Value>, ConfigError> {
        let content = match std::fs::read_to_string(path) {
            Ok(content) => content,
            Err(source) if source.kind() == ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(ConfigError::Io {
                    path: path.to_path_buf(),
                    source,
                })
            }
        };
        format.parse(&content, Some(path)).map(Some)
    }

    /// The overlay path for a profile: `config/app.toml` + `prod` →
    /// `config/app.prod.toml`.
    fn overlay_path(path: &Path, profile: &str) -> Option<PathBuf> {
        let stem = path.file_stem()?.to_str()?;
        let extension = path.extension()?.to_str()?;
        Some(path.with_file_name(format!("{stem}.{profile}.{extension}")))
    }

    fn missing(&self) -> Result<Value, ConfigError> {
        if self.required {
            Err(ConfigError::Io {
                path: self.path.clone(),
                source: std::io::Error::new(
                    ErrorKind::NotFound,
                    "required configuration file not found",
                ),
            })
        } else {
            Ok(Value::Table(Table::new()))
        }
    }
}

impl Layer for File {
    fn name(&self) -> String {
        format!("file ({})", self.path.display())
    }

    fn load(&self, cx: &LayerContext<'_>) -> Result<Value, ConfigError> {
        let Some((path, format)) = self.resolve()? else {
            return self.missing();
        };
        let Some(mut root) = Self::read_and_parse(&path, format)? else {
            return self.missing();
        };

        if self.profile_variants {
            if let Some(profile) = cx.profile {
                if let Some(overlay_path) = Self::overlay_path(&path, profile) {
                    if let Some(overlay) = Self::read_and_parse(&overlay_path, format)? {
                        root.merge_from(overlay);
                    }
                }
            }
        }
        Ok(root)
    }
}
