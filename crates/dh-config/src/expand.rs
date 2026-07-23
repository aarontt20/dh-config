//! Post-merge placeholder expansion.
//!
//! When enabled with [`ConfigBuilder::expand_placeholders`], string values in
//! the merged tree may reference other configuration values (`${server.host}`)
//! or environment variables (`${env:HOME}`), with optional shell-style
//! defaults (`${server.port:-8080}`). Expansion runs on the *merged* tree, so
//! references see final values: overriding `server.port` from the environment
//! also changes every string derived from it.
//!
//! [`ConfigBuilder::expand_placeholders`]: crate::ConfigBuilder::expand_placeholders

use std::collections::{BTreeMap, BTreeSet};

use crate::error::ConfigError;
use crate::layer::parse_scalar;
use crate::value::{Table, Value};

/// Backstop against pathological reference chains. Real cycles are caught by
/// the resolution stack; this only bounds the length of acyclic chains.
const MAX_DEPTH: usize = 64;

/// The result of expanding a merged tree.
pub(crate) struct Expanded {
    /// The tree with every placeholder resolved.
    pub root: Value,
    /// The paths whose value was changed by expansion (including `$$`
    /// unescaping), used to annotate provenance output.
    pub paths: BTreeSet<String>,
}

/// Expands every placeholder in `root`, resolving references against `root`
/// itself. `origins` and `layer_names` (as recorded by the builder) let
/// errors name the layer that supplied the offending template string.
pub(crate) fn expand(
    root: &Value,
    origins: &BTreeMap<String, usize>,
    layer_names: &[String],
) -> Result<Expanded, ConfigError> {
    let mut expander = Expander {
        root,
        origins,
        layer_names,
        memo: BTreeMap::new(),
        stack: Vec::new(),
        touched: BTreeSet::new(),
    };
    let expanded = expander.expand_value("", root)?;
    Ok(Expanded {
        root: expanded,
        paths: expander.touched,
    })
}

/// One parsed piece of a template string.
enum Segment {
    /// Literal text (with `$$` already unescaped to `$`).
    Literal(String),
    /// A `${…}` placeholder.
    Placeholder {
        target: Target,
        /// The `:-` default, if one was given.
        default: Option<String>,
    },
}

/// What a placeholder refers to.
enum Target {
    /// `${dotted.path}` — a path into the merged configuration.
    Path(String),
    /// `${env:VAR}` — an environment variable.
    Env(String),
}

impl Target {
    /// The placeholder as written, for error messages.
    fn display(&self) -> String {
        match self {
            Target::Path(path) => format!("${{{path}}}"),
            Target::Env(name) => format!("${{env:{name}}}"),
        }
    }
}

struct Expander<'a> {
    /// The pre-expansion merged tree that references resolve against.
    root: &'a Value,
    origins: &'a BTreeMap<String, usize>,
    layer_names: &'a [String],
    /// Fully-expanded values by reference path, so shared references are
    /// resolved once.
    memo: BTreeMap<String, Value>,
    /// The reference paths currently being resolved; a repeat is a cycle.
    stack: Vec<String>,
    touched: BTreeSet<String>,
}

impl Expander<'_> {
    /// Returns `value` with every placeholder in it (and below it) expanded.
    fn expand_value(&mut self, path: &str, value: &Value) -> Result<Value, ConfigError> {
        match value {
            Value::String(raw) if raw.contains('$') => {
                let expanded = self.expand_string(path, raw)?;
                if !matches!(&expanded, Value::String(s) if s == raw) {
                    self.touched.insert(path.to_string());
                }
                Ok(expanded)
            }
            Value::Table(table) => {
                let mut out = Table::new();
                for (key, child) in table {
                    let child_path = join(path, key);
                    out.insert(key.clone(), self.expand_value(&child_path, child)?);
                }
                Ok(Value::Table(out))
            }
            Value::Array(items) => {
                let mut out = Vec::with_capacity(items.len());
                for (index, item) in items.iter().enumerate() {
                    let child_path = join(path, &index.to_string());
                    out.push(self.expand_value(&child_path, item)?);
                }
                Ok(Value::Array(out))
            }
            other => Ok(other.clone()),
        }
    }

    /// Expands one template string. A string that is exactly one placeholder
    /// splices the referenced value verbatim, preserving its type; a
    /// placeholder embedded in longer text renders scalars into the string.
    fn expand_string(&mut self, path: &str, raw: &str) -> Result<Value, ConfigError> {
        let segments = parse_template(raw).map_err(|message| self.error(path, message))?;
        if let [Segment::Placeholder { target, default }] = segments.as_slice() {
            return self.resolve(path, target, default.as_deref(), true);
        }
        let mut out = String::new();
        for segment in &segments {
            match segment {
                Segment::Literal(text) => out.push_str(text),
                Segment::Placeholder { target, default } => {
                    let value = self.resolve(path, target, default.as_deref(), false)?;
                    match &value {
                        Value::String(text) => out.push_str(text),
                        Value::Bool(_) | Value::Integer(_) | Value::Float(_) => {
                            out.push_str(&value.to_string())
                        }
                        Value::Null | Value::Array(_) | Value::Table(_) => {
                            return Err(self.error(
                                path,
                                format!(
                                    "placeholder `{}` resolves to {}, which cannot be \
                                     interpolated into a string",
                                    target.display(),
                                    value.type_name(),
                                ),
                            ))
                        }
                    }
                }
            }
        }
        Ok(Value::String(out))
    }

    /// Resolves one placeholder. `whole` is true when the placeholder is the
    /// entire string: the resolved value keeps its type, and defaults (and
    /// `env:` lookups) parse leniently the way env-layer values do.
    fn resolve(
        &mut self,
        at: &str,
        target: &Target,
        default: Option<&str>,
        whole: bool,
    ) -> Result<Value, ConfigError> {
        let scalar = |raw: &str| {
            if whole {
                parse_scalar(raw)
            } else {
                Value::String(raw.to_string())
            }
        };
        let found = match target {
            Target::Env(name) => std::env::var(name).ok().map(|raw| scalar(&raw)),
            Target::Path(reference) => self.resolve_path(reference)?,
        };
        match found {
            Some(value) => Ok(value),
            None => match default {
                Some(text) => Ok(scalar(text)),
                None => Err(self.error(
                    at,
                    match target {
                        Target::Env(name) => {
                            format!(
                                "environment variable `{name}` (from `${{env:{name}}}`) is not set"
                            )
                        }
                        Target::Path(_) => {
                            format!("placeholder `{}` does not resolve", target.display())
                        }
                    },
                )),
            },
        }
    }

    /// Resolves a config-path reference to its fully-expanded value, or
    /// `None` if the path does not exist. Referenced values are expanded
    /// before use (depth-first, memoized), so chains resolve in dependency
    /// order and repeats on the stack are reported as cycles.
    fn resolve_path(&mut self, reference: &str) -> Result<Option<Value>, ConfigError> {
        if let Some(hit) = self.memo.get(reference) {
            return Ok(Some(hit.clone()));
        }
        if let Some(position) = self.stack.iter().position(|p| p == reference) {
            let mut chain = self.stack[position..].to_vec();
            chain.push(reference.to_string());
            return Err(ConfigError::PlaceholderCycle { chain });
        }
        let Some(raw) = self.root.get_path(reference) else {
            return Ok(None);
        };
        if self.stack.len() >= MAX_DEPTH {
            return Err(ConfigError::Expansion {
                path: reference.to_string(),
                message: format!("placeholder references nest deeper than {MAX_DEPTH} levels"),
                origin: None,
            });
        }
        self.stack.push(reference.to_string());
        let result = self.expand_value(reference, raw);
        self.stack.pop();
        let value = result?;
        self.memo.insert(reference.to_string(), value.clone());
        Ok(Some(value))
    }

    fn error(&self, path: &str, message: String) -> ConfigError {
        ConfigError::Expansion {
            path: path.to_string(),
            message,
            origin: self.origin(path).map(str::to_string),
        }
    }

    /// The name of the layer that supplied the value at `path`, walking up to
    /// the nearest recorded ancestor (strings inside arrays are recorded
    /// under the array's own path).
    fn origin(&self, path: &str) -> Option<&str> {
        let mut candidate = path;
        loop {
            if let Some(&index) = self.origins.get(candidate) {
                return self.layer_names.get(index).map(String::as_str);
            }
            candidate = candidate.rsplit_once('.')?.0;
        }
    }
}

fn join(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{prefix}.{key}")
    }
}

/// Splits a raw string into literal and placeholder segments.
///
/// `$$` escapes a literal `$` (so `$${…}` renders as `${…}`); a lone `$` not
/// followed by `{` or `$` stays literal. An unclosed `${` and a `${` nested
/// inside a placeholder are errors.
fn parse_template(raw: &str) -> Result<Vec<Segment>, String> {
    let mut segments = Vec::new();
    let mut literal = String::new();
    let mut rest = raw;
    while let Some(position) = rest.find('$') {
        literal.push_str(&rest[..position]);
        rest = &rest[position + 1..];
        if let Some(tail) = rest.strip_prefix('$') {
            literal.push('$');
            rest = tail;
        } else if let Some(tail) = rest.strip_prefix('{') {
            let Some(end) = tail.find('}') else {
                return Err("unterminated placeholder: missing closing `}`".to_string());
            };
            let inner = &tail[..end];
            if inner.contains("${") {
                return Err(format!(
                    "nested placeholders are not supported (in `${{{inner}}}`)"
                ));
            }
            if !literal.is_empty() {
                segments.push(Segment::Literal(std::mem::take(&mut literal)));
            }
            segments.push(parse_placeholder(inner)?);
            rest = &tail[end + 1..];
        } else {
            literal.push('$');
        }
    }
    literal.push_str(rest);
    if !literal.is_empty() {
        segments.push(Segment::Literal(literal));
    }
    Ok(segments)
}

/// Parses the inside of a `${…}`: an optional `:-` default, then either the
/// `env:` namespace or a config path.
fn parse_placeholder(inner: &str) -> Result<Segment, String> {
    let (target, default) = match inner.find(":-") {
        Some(position) => (&inner[..position], Some(inner[position + 2..].to_string())),
        None => (inner, None),
    };
    if target.is_empty() {
        return Err("empty placeholder".to_string());
    }
    let target = match target.strip_prefix("env:") {
        Some("") => return Err("empty environment variable name in `${env:}`".to_string()),
        Some(name) => Target::Env(name.to_string()),
        None => Target::Path(target.to_string()),
    };
    Ok(Segment::Placeholder { target, default })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expand_str(raw: &str, root: &Value) -> Result<Value, ConfigError> {
        let origins = BTreeMap::new();
        let mut expander = Expander {
            root,
            origins: &origins,
            layer_names: &[],
            memo: BTreeMap::new(),
            stack: Vec::new(),
            touched: BTreeSet::new(),
        };
        expander.expand_string("test", raw)
    }

    fn root() -> Value {
        let mut root = Value::Table(Table::new());
        root.set_path("server.host", "localhost".into());
        root.set_path("server.port", 8080.into());
        root
    }

    #[test]
    fn interpolates_scalars_into_text() {
        let value = expand_str("http://${server.host}:${server.port}/", &root()).unwrap();
        assert_eq!(value, "http://localhost:8080/".into());
    }

    #[test]
    fn whole_string_placeholder_preserves_type() {
        let value = expand_str("${server.port}", &root()).unwrap();
        assert_eq!(value, 8080.into());
    }

    #[test]
    fn escapes_and_lone_dollars() {
        let root = root();
        assert_eq!(
            expand_str("$${server.host}", &root).unwrap(),
            "${server.host}".into()
        );
        assert_eq!(expand_str("$$", &root).unwrap(), "$".into());
        assert_eq!(expand_str("cost: $5", &root).unwrap(), "cost: $5".into());
        assert_eq!(
            expand_str("trailing $", &root).unwrap(),
            "trailing $".into()
        );
    }

    #[test]
    fn defaults_apply_when_missing() {
        let root = root();
        assert_eq!(
            expand_str("${missing:-fallback}", &root).unwrap(),
            "fallback".into()
        );
        // Whole-string defaults parse leniently, like env values.
        assert_eq!(expand_str("${missing:-42}", &root).unwrap(), 42.into());
        // Interpolated defaults stay text.
        assert_eq!(
            expand_str("v=${missing:-42}", &root).unwrap(),
            "v=42".into()
        );
        // A present value wins over the default.
        assert_eq!(expand_str("${server.port:-1}", &root).unwrap(), 8080.into());
    }

    #[test]
    fn malformed_placeholders_error() {
        let root = root();
        assert!(expand_str("${unclosed", &root).is_err());
        assert!(expand_str("${}", &root).is_err());
        assert!(expand_str("${env:}", &root).is_err());
        assert!(expand_str("${a${b}}", &root).is_err());
    }
}
