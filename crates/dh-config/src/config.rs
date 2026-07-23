//! The resolved, merged configuration: [`Config`].

use std::collections::{BTreeMap, BTreeSet};

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
///
/// The merge also records *provenance*: which layer supplied each value. Ask
/// with [`Config::origin`], or dump the whole picture with
/// [`Config::explain`]. Type errors automatically name the supplying layer.
#[derive(Clone, Debug)]
pub struct Config {
    root: Value,
    profile: Option<String>,
    /// Layer names in merge order; indexed by the values in `origins`.
    layer_names: Vec<String>,
    /// Leaf path → index (into `layer_names`) of the layer that supplied it.
    origins: BTreeMap<String, usize>,
    /// Paths whose value was changed by placeholder expansion.
    expanded: BTreeSet<String>,
}

impl Config {
    /// Starts a new [`ConfigBuilder`].
    pub fn builder() -> ConfigBuilder {
        ConfigBuilder::new()
    }

    pub(crate) fn from_parts(
        root: Value,
        profile: Option<String>,
        layer_names: Vec<String>,
        origins: BTreeMap<String, usize>,
        expanded: BTreeSet<String>,
    ) -> Self {
        Config {
            root,
            profile,
            layer_names,
            origins,
            expanded,
        }
    }

    /// Deserializes the entire merged configuration into `T`.
    pub fn deserialize<T: DeserializeOwned>(&self) -> Result<T, ConfigError> {
        from_value(&self.root).map_err(|e| self.attach_origin(e))
    }

    /// Gets the value at a dot-separated path (`"server.port"`,
    /// `"peers.0.host"`), deserialized into `T`. Missing paths are a
    /// [`ConfigError::NotFound`].
    pub fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ConfigError> {
        match self.root.get_path(path) {
            Some(value) => from_value(value).map_err(|e| self.attach_origin(e.prefixed_with(path))),
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
                .map_err(|e| self.attach_origin(e.prefixed_with(path))),
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

    /// The name of the layer that supplied the value at `path` — the answer
    /// to "who set this?".
    ///
    /// Returns `None` for paths that don't exist and for tables (a table is a
    /// composite of many layers; ask about its leaves instead). Paths inside
    /// an array resolve to the layer that supplied the array, since arrays
    /// are replaced wholesale rather than merged.
    pub fn origin(&self, path: &str) -> Option<&str> {
        self.root.get_path(path)?;
        let mut candidate = path;
        loop {
            // Only non-table values carry an origin; a stale entry under a
            // path that is now a table is ignored.
            if let Some(value) = self.root.get_path(candidate) {
                if !matches!(value, Value::Table(_)) {
                    if let Some(&index) = self.origins.get(candidate) {
                        return Some(&self.layer_names[index]);
                    }
                }
            }
            candidate = match candidate.rsplit_once('.') {
                Some((parent, _)) => parent,
                None => return None,
            };
        }
    }

    /// Renders every resolved value with the layer it came from — made for
    /// startup logs and "why is my config wrong?" sessions:
    ///
    /// ```text
    /// profile: prod
    /// server.host = "0.0.0.0"  [file (config/app.toml)]
    /// server.port = 7000  [command line]
    /// database.url = "postgres://db:5432/app"  [file (config/app.toml), expanded]
    /// ```
    ///
    /// Values rewritten by placeholder expansion carry an `expanded` marker.
    pub fn explain(&self) -> String {
        let mut out = String::new();
        if let Some(profile) = &self.profile {
            out.push_str("profile: ");
            out.push_str(profile);
            out.push('\n');
        }
        let mut leaves = Vec::new();
        collect_leaves(&self.root, String::new(), &mut leaves);
        for (path, value) in leaves {
            let origin = self.origin(&path).unwrap_or("unknown");
            let expanded = if self.is_expanded(&path) {
                ", expanded"
            } else {
                ""
            };
            out.push_str(&format!(
                "{path} = {}  [{origin}{expanded}]\n",
                render(value)
            ));
        }
        out
    }

    /// Returns `true` if the value at `path` was rewritten by placeholder
    /// expansion — either directly, or by living inside a spliced table.
    fn is_expanded(&self, path: &str) -> bool {
        let mut candidate = path;
        loop {
            if self.expanded.contains(candidate) {
                return true;
            }
            match candidate.rsplit_once('.') {
                Some((parent, _)) => candidate = parent,
                None => return false,
            }
        }
    }

    /// The raw merged value tree.
    pub fn root(&self) -> &Value {
        &self.root
    }

    /// The profile the configuration was built with, if any.
    pub fn profile(&self) -> Option<&str> {
        self.profile.as_deref()
    }

    /// Adds the supplying layer's name to a type error, using the path the
    /// error itself reports (which may be deeper than the requested one).
    fn attach_origin(&self, error: ConfigError) -> ConfigError {
        match error {
            ConfigError::TypeMismatch {
                path,
                expected,
                found,
                origin: None,
            } => {
                let origin = self.origin(&path).map(str::to_string);
                ConfigError::TypeMismatch {
                    path,
                    expected,
                    found,
                    origin,
                }
            }
            other => other,
        }
    }
}

fn collect_leaves<'a>(value: &'a Value, prefix: String, out: &mut Vec<(String, &'a Value)>) {
    match value {
        Value::Table(table) => {
            for (key, child) in table {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                collect_leaves(child, path, out);
            }
        }
        _ => out.push((prefix, value)),
    }
}

/// Formats a value for [`Config::explain`]: strings quoted so `"8080"` and
/// `8080` are distinguishable, containers rendered recursively.
fn render(value: &Value) -> String {
    match value {
        Value::String(s) => format!("{s:?}"),
        Value::Array(items) => {
            let items: Vec<String> = items.iter().map(render).collect();
            format!("[{}]", items.join(", "))
        }
        Value::Table(table) => {
            let entries: Vec<String> = table
                .iter()
                .map(|(k, v)| format!("{k}: {}", render(v)))
                .collect();
            format!("{{{}}}", entries.join(", "))
        }
        other => other.to_string(),
    }
}
