//! Demonstrates the full dh-config layer stack.
//!
//! Try, from the workspace root:
//!
//! ```text
//! cargo run -p layered-app
//! APP_PROFILE=prod cargo run -p layered-app
//! APP_SERVER__PORT=9000 cargo run -p layered-app
//! cargo run -p layered-app -- --server.port=7000 --verbose
//! ```

use std::path::{Path, PathBuf};

use dh_config::{
    CommandLine, Config, ConfigError, Defaults, Env, File, Layer, LayerContext, Value,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct AppConfig {
    verbose: bool,
    server: Server,
    database: Database,
    log: Log,
}

#[derive(Debug, Deserialize)]
struct Server {
    host: String,
    port: u16,
}

#[derive(Debug, Deserialize)]
struct Database {
    url: String,
    pool_size: u32,
}

#[derive(Debug, Deserialize)]
struct Log {
    level: LogLevel,
    format: String,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

/// A custom layer: computes values at startup that no static source knows.
struct RuntimeInfo;

impl Layer for RuntimeInfo {
    fn name(&self) -> String {
        "runtime-info".to_string()
    }

    fn load(&self, _cx: &LayerContext<'_>) -> Result<Value, ConfigError> {
        let mut root = Value::Table(Default::default());
        root.set_path("runtime.pid", Value::Integer(std::process::id().into()));
        Ok(root)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config_dir = config_dir();

    let config = Config::builder()
        .with_layer(
            Defaults::new()
                .set("verbose", false)
                .set("server.host", "127.0.0.1")
                .set("server.port", 8080),
        )
        .with_file(config_dir.join("app.toml"))
        .with_layer(File::new(config_dir.join("local.yaml")).required(false))
        .with_layer(Env::prefixed("APP"))
        .with_layer(RuntimeInfo)
        .with_layer(CommandLine::from_env())
        .profile_from_env("APP_PROFILE")
        .build()?;

    println!("profile: {}", config.profile().unwrap_or("(none)"));

    let app: AppConfig = config.deserialize()?;
    println!("typed:   {app:#?}");

    let port: u16 = config.get("server.port")?;
    let pid: u32 = config.get("runtime.pid")?;
    println!(
        "dynamic: server.port={port} runtime.pid={pid} verbose={}",
        app.verbose
    );
    println!(
        "summary: listening on {}:{}, db {} (pool {}), log {:?}/{}",
        app.server.host,
        app.server.port,
        app.database.url,
        app.database.pool_size,
        app.log.level,
        app.log.format,
    );
    Ok(())
}

/// Finds the example's config directory whether run from the workspace root
/// or the crate directory.
fn config_dir() -> PathBuf {
    let manifest_relative = Path::new(env!("CARGO_MANIFEST_DIR")).join("config");
    if manifest_relative.is_dir() {
        manifest_relative
    } else {
        PathBuf::from("config")
    }
}
