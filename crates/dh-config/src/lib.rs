//! # dh-config
//!
//! A from-scratch **layered configuration** library. Configuration is
//! assembled from an ordered stack of *layers* — code defaults, files
//! (JSON/TOML/YAML), environment variables, command-line arguments, and
//! anything else you implement — deep-merged so that later layers override
//! earlier ones.
//!
//! The stack is defined programmatically with [`ConfigBuilder`]:
//!
//! ```no_run
//! use dh_config::{CommandLine, Config, Defaults, Env, File};
//! use serde::Deserialize;
//!
//! #[derive(Deserialize)]
//! struct AppConfig {
//!     server: Server,
//!     verbose: bool,
//! }
//!
//! #[derive(Deserialize)]
//! struct Server {
//!     host: String,
//!     port: u16,
//! }
//!
//! let config = Config::builder()
//!     // 1. Lowest precedence: defaults defined in code.
//!     .with_layer(
//!         Defaults::new()
//!             .set("server.host", "127.0.0.1")
//!             .set("server.port", 8080)
//!             .set("verbose", false),
//!     )
//!     // 2. A required config file; `app.prod.toml` overlays it when the
//!     //    `prod` profile is active.
//!     .with_file("config/app.toml")
//!     // 3. An optional, developer-local file.
//!     .with_file(File::new("config/local.toml").required(false))
//!     // 4. Environment: APP_SERVER__PORT=9000 → server.port.
//!     .with_env("APP")
//!     // 5. Highest precedence: --server.port=7000 --verbose, no clap needed.
//!     .with_layer(CommandLine::from_env())
//!     .profile_from_env("APP_PROFILE")
//!     .build()?;
//!
//! // Typed access…
//! let app: AppConfig = config.deserialize()?;
//! // …or dynamic access.
//! let port: u16 = config.get("server.port")?;
//! # Ok::<(), dh_config::ConfigError>(())
//! ```
//!
//! When the typed struct is all you need, [`ConfigBuilder::extract`]
//! collapses `build()` + `deserialize()` into one call.
//!
//! ## Layering model
//!
//! Every layer produces a [`Value`] tree with a table at its root. The
//! builder merges the trees in the order the layers were added:
//!
//! * **Tables merge recursively** — setting `server.port` in a higher layer
//!   keeps `server.host` from a lower one.
//! * **Scalars and arrays replace** — a higher layer's list wins wholesale;
//!   there is no element-wise splicing.
//! * **Explicit nulls override** — a higher layer can deliberately unset a
//!   value by writing `null`.
//!
//! ## Provenance
//!
//! The merge records which layer supplied every value. [`Config::origin`]
//! answers "who set this?" for one path, [`Config::explain`] dumps the whole
//! resolved configuration with sources (made for startup logs), and type
//! errors automatically name the supplying layer:
//!
//! ```text
//! at `server.port`: expected u16, found boolean (value set by layer `environment (APP_*)`)
//! ```
//!
//! ## Placeholder expansion
//!
//! With [`ConfigBuilder::expand_placeholders`] (off by default), string
//! values may reference other configuration values or environment variables,
//! with optional defaults:
//!
//! ```toml
//! url  = "postgres://${database.host}:${database.port}/app"
//! pass = "${env:DB_PASSWORD:-dev-password}"
//! ```
//!
//! References resolve against the *merged* tree, so a value overridden by a
//! higher layer flows into every string derived from it. See the method docs
//! for the full syntax (type-preserving whole-string references, `$$`
//! escaping, cycle detection).
//!
//! ## Custom layers
//!
//! Implement [`Layer`] for any configuration source; see the trait docs for
//! an example. [`Overrides`] covers the common "I already have the values in
//! code" case without a new type.
//!
//! ## Cargo features
//!
//! `json`, `toml`, and `yaml` gate the file-format parsers and are all
//! enabled by default. With none of them, the file layer is compiled out and
//! only code/env/CLI layers remain.

#![warn(missing_docs)]

mod builder;
mod config;
mod de;
mod error;
mod expand;
#[cfg(any(feature = "json", feature = "toml", feature = "yaml"))]
mod format;
mod layer;
mod ser;
mod value;

pub use builder::ConfigBuilder;
pub use config::Config;
pub use de::from_value;
pub use error::ConfigError;
#[cfg(any(feature = "json", feature = "toml", feature = "yaml"))]
pub use format::Format;
#[cfg(any(feature = "json", feature = "toml", feature = "yaml"))]
pub use layer::File;
pub use layer::{CommandLine, Defaults, Env, Layer, LayerContext, Overrides};
pub use ser::to_value;
pub use value::{Table, Value};
