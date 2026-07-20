//! End-to-end tests of the full layer stack: files in three formats, env
//! vars, CLI args, defaults, custom layers, profiles, and both access styles.

use std::collections::HashMap;
use std::path::PathBuf;

use dh_config::{
    CommandLine, Config, ConfigError, Defaults, Env, File, Layer, LayerContext, Overrides, Value,
};
use serde::{Deserialize, Serialize};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn env_source(vars: &[(&str, &str)]) -> Vec<(String, String)> {
    vars.iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn cli(args: &[&str]) -> CommandLine {
    CommandLine::from_args(args.iter().map(|s| s.to_string()))
}

#[derive(Debug, Deserialize, PartialEq)]
struct AppConfig {
    name: String,
    server: Server,
    log: Log,
    peers: Vec<Peer>,
    #[serde(default)]
    features: HashMap<String, bool>,
}

#[derive(Debug, Deserialize, PartialEq)]
struct Server {
    host: String,
    port: u16,
    #[serde(default)]
    workers: u32,
}

#[derive(Debug, Deserialize, PartialEq)]
struct Log {
    level: Level,
    #[serde(default)]
    format: Option<String>,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Level {
    Debug,
    Info,
    Warn,
}

#[derive(Debug, Deserialize, PartialEq)]
struct Peer {
    host: String,
}

#[test]
fn full_stack_precedence() {
    let config = Config::builder()
        .with_layer(
            Defaults::new()
                .set("server.port", 1) // overridden by app.toml
                .set("log.level", "debug"), // overridden by app.toml
        )
        .with_file(fixture("app.toml"))
        .with_file(fixture("extra.json"))
        .with_file(fixture("extra.yaml"))
        .with_layer(Env::prefixed("T1").source(env_source(&[
            ("T1_SERVER__PORT", "9000"),
            ("T1_LOG__FORMAT", "logfmt"),
        ])))
        .with_layer(cli(&["--server.port=7000"]))
        .build()
        .unwrap();

    // CLI beats env beats yaml/json beats toml beats defaults.
    assert_eq!(config.get::<u16>("server.port").unwrap(), 7000);
    // Env-only override survives.
    assert_eq!(config.get::<String>("log.format").unwrap(), "logfmt");
    // Values only set by lower layers survive merging.
    assert_eq!(config.get::<String>("server.host").unwrap(), "127.0.0.1");
    assert_eq!(config.get::<u32>("server.workers").unwrap(), 4);
    assert!(config.get::<bool>("features.new_ui").unwrap());

    let app: AppConfig = config.deserialize().unwrap();
    assert_eq!(app.name, "fixture-app");
    assert_eq!(app.server.port, 7000);
    assert_eq!(app.log.level, Level::Info);
    assert_eq!(
        app.peers,
        vec![
            Peer {
                host: "a.example".into()
            },
            Peer {
                host: "b.example".into()
            }
        ]
    );
    assert!(!app.features["beta"]);
}

#[test]
fn profile_overlay_applies() {
    let config = Config::builder()
        .with_file(fixture("app.toml"))
        .profile("prod")
        .build()
        .unwrap();

    assert_eq!(config.profile(), Some("prod"));
    // Overridden by app.prod.toml:
    assert_eq!(config.get::<String>("server.host").unwrap(), "0.0.0.0");
    assert_eq!(config.get::<String>("log.level").unwrap(), "warn");
    // Untouched by the overlay:
    assert_eq!(config.get::<u16>("server.port").unwrap(), 8080);
}

#[test]
fn missing_profile_overlay_is_fine() {
    let config = Config::builder()
        .with_file(fixture("app.toml"))
        .profile("staging") // app.staging.toml does not exist
        .build()
        .unwrap();
    assert_eq!(config.get::<String>("server.host").unwrap(), "127.0.0.1");
}

#[test]
fn profile_variants_can_be_disabled() {
    let config = Config::builder()
        .with_layer(File::new(fixture("app.toml")).profile_variants(false))
        .profile("prod")
        .build()
        .unwrap();
    assert_eq!(config.get::<String>("server.host").unwrap(), "127.0.0.1");
}

#[test]
fn extensionless_path_probes_formats() {
    let config = Config::builder()
        .with_file(fixture("app")) // resolves to app.toml
        .build()
        .unwrap();
    assert_eq!(config.get::<String>("name").unwrap(), "fixture-app");
}

#[test]
fn missing_required_file_errors() {
    let error = Config::builder()
        .with_file(fixture("nope.toml"))
        .build()
        .unwrap_err();
    assert!(matches!(error, ConfigError::Io { .. }), "{error}");
}

#[test]
fn missing_optional_file_is_empty() {
    let config = Config::builder()
        .with_layer(File::new(fixture("nope.toml")).required(false))
        .with_layer(Overrides::new().set("ok", true))
        .build()
        .unwrap();
    assert!(config.get::<bool>("ok").unwrap());
}

#[test]
fn parse_errors_name_the_file() {
    let error = Config::builder()
        .with_file(fixture("broken.toml"))
        .build()
        .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("broken.toml"), "{message}");
    assert!(matches!(error, ConfigError::Parse { format: "toml", .. }));
}

#[test]
fn defaults_from_struct() {
    #[derive(Serialize)]
    struct MyDefaults {
        retries: u32,
        endpoints: Vec<String>,
    }

    let config = Config::builder()
        .with_defaults(&MyDefaults {
            retries: 3,
            endpoints: vec!["a".into()],
        })
        .with_layer(Overrides::new().set("retries", 5))
        .build()
        .unwrap();

    assert_eq!(config.get::<u32>("retries").unwrap(), 5);
    assert_eq!(config.get::<Vec<String>>("endpoints").unwrap(), vec!["a"]);
}

#[test]
fn custom_layer_participates_in_merging() {
    struct Fixed;
    impl Layer for Fixed {
        fn name(&self) -> String {
            "fixed".into()
        }
        fn load(&self, _cx: &LayerContext<'_>) -> Result<Value, ConfigError> {
            let mut root = Value::Table(Default::default());
            root.set_path("custom.answer", Value::Integer(42));
            Ok(root)
        }
    }

    let config = Config::builder()
        .with_layer(Fixed)
        .with_layer(Overrides::new().set("custom.other", "x"))
        .build()
        .unwrap();
    assert_eq!(config.get::<i64>("custom.answer").unwrap(), 42);
    assert_eq!(config.get::<String>("custom.other").unwrap(), "x");
}

#[test]
fn custom_layer_sees_the_profile() {
    struct ProfileEcho;
    impl Layer for ProfileEcho {
        fn name(&self) -> String {
            "profile-echo".into()
        }
        fn load(&self, cx: &LayerContext<'_>) -> Result<Value, ConfigError> {
            let mut root = Value::Table(Default::default());
            root.set_path(
                "active_profile",
                Value::String(cx.profile.unwrap_or("none").to_string()),
            );
            Ok(root)
        }
    }

    let config = Config::builder()
        .with_layer(ProfileEcho)
        .profile("prod")
        .build()
        .unwrap();
    assert_eq!(config.get::<String>("active_profile").unwrap(), "prod");
}

#[test]
fn non_table_layer_root_is_rejected() {
    struct Scalar;
    impl Layer for Scalar {
        fn name(&self) -> String {
            "scalar".into()
        }
        fn load(&self, _cx: &LayerContext<'_>) -> Result<Value, ConfigError> {
            Ok(Value::Integer(1))
        }
    }

    let error = Config::builder().with_layer(Scalar).build().unwrap_err();
    assert!(matches!(error, ConfigError::InvalidRoot { .. }), "{error}");
}

#[test]
fn dynamic_access_helpers() {
    let config = Config::builder()
        .with_layer(Overrides::new().set("a.b", 1).set("null_key", Value::Null))
        .build()
        .unwrap();

    assert_eq!(config.get::<i64>("a.b").unwrap(), 1);
    assert!(config.contains("a.b"));
    assert!(!config.contains("a.missing"));

    assert!(matches!(
        config.get::<i64>("a.missing").unwrap_err(),
        ConfigError::NotFound { .. }
    ));
    assert_eq!(config.get_opt::<i64>("a.missing").unwrap(), None);
    assert_eq!(config.get_opt::<i64>("null_key").unwrap(), None);
    assert_eq!(config.get_or("a.missing", 9).unwrap(), 9);
    assert_eq!(config.get_or("a.b", 9).unwrap(), 1);
}

#[test]
fn type_errors_name_the_path() {
    let config = Config::builder()
        .with_layer(Overrides::new().set("server.port", "not-a-number"))
        .build()
        .unwrap();

    let error = config.get::<u16>("server.port").unwrap_err();
    assert!(error.to_string().contains("server.port"), "{error}");

    #[derive(Debug, Deserialize)]
    struct Nested {
        #[allow(dead_code)]
        server: NestedServer,
    }
    #[derive(Debug, Deserialize)]
    struct NestedServer {
        #[allow(dead_code)]
        port: Vec<u8>,
    }
    let error = config.deserialize::<Nested>().unwrap_err();
    assert!(error.to_string().contains("server.port"), "{error}");
}

#[test]
fn stringly_typed_env_values_deserialize_into_typed_fields() {
    #[derive(Deserialize)]
    struct Typed {
        port: u16,
        ratio: f64,
        enabled: bool,
        // An integer-looking env value can still land in a string field.
        version: String,
    }

    let config = Config::builder()
        .with_layer(Env::prefixed("T2").parse_values(false).source(env_source(&[
            ("T2_PORT", "8080"),
            ("T2_RATIO", "0.5"),
            ("T2_ENABLED", "true"),
            ("T2_VERSION", "2024"),
        ])))
        .build()
        .unwrap();

    let typed: Typed = config.deserialize().unwrap();
    assert_eq!(typed.port, 8080);
    assert_eq!(typed.ratio, 0.5);
    assert!(typed.enabled);
    assert_eq!(typed.version, "2024");
}

#[test]
fn enums_deserialize_from_strings_and_tables() {
    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(rename_all = "lowercase")]
    enum Backend {
        Memory,
        Postgres { url: String },
    }

    #[derive(Debug, Deserialize)]
    struct WithEnums {
        simple: Backend,
        with_payload: Backend,
    }

    let mut payload = Value::Table(Default::default());
    payload.set_path("postgres.url", Value::String("pg://x".into()));

    let config = Config::builder()
        .with_layer(
            Overrides::new()
                .set("simple", "memory")
                .set("with_payload", payload),
        )
        .build()
        .unwrap();

    let parsed: WithEnums = config.deserialize().unwrap();
    assert_eq!(parsed.simple, Backend::Memory);
    assert_eq!(
        parsed.with_payload,
        Backend::Postgres {
            url: "pg://x".into()
        }
    );
}

#[test]
fn optional_fields_and_missing_values() {
    #[derive(Debug, Deserialize)]
    struct Opt {
        present: Option<u32>,
        absent: Option<u32>,
        explicit_null: Option<u32>,
    }

    let config = Config::builder()
        .with_layer(
            Overrides::new()
                .set("present", 1)
                .set("absent", Value::Null)
                .set("explicit_null", Value::Null),
        )
        .build()
        .unwrap();

    let opt: Opt = config.deserialize().unwrap();
    assert_eq!(opt.present, Some(1));
    assert_eq!(opt.absent, None);
    assert_eq!(opt.explicit_null, None);
}

#[test]
fn round_trip_serialize_deserialize() {
    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct Round {
        name: String,
        counts: Vec<i64>,
        nested: HashMap<String, String>,
    }

    let original = Round {
        name: "x".into(),
        counts: vec![1, 2, 3],
        nested: HashMap::from([("k".to_string(), "v".to_string())]),
    };
    let value = dh_config::to_value(&original).unwrap();
    let back: Round = dh_config::from_value(&value).unwrap();
    assert_eq!(back, original);
}

#[test]
fn profile_from_env_reads_variable() {
    // Set an env var name unlikely to collide; tests may run in parallel but
    // only this test touches it.
    std::env::set_var("DH_CONFIG_TEST_PROFILE_XYZZY", "prod");
    let config = Config::builder()
        .with_file(fixture("app.toml"))
        .profile_from_env("DH_CONFIG_TEST_PROFILE_XYZZY")
        .build()
        .unwrap();
    std::env::remove_var("DH_CONFIG_TEST_PROFILE_XYZZY");

    assert_eq!(config.profile(), Some("prod"));
    assert_eq!(config.get::<String>("server.host").unwrap(), "0.0.0.0");
}

#[test]
fn explicit_profile_beats_env_profile() {
    std::env::set_var("DH_CONFIG_TEST_PROFILE_PLUGH", "prod");
    let config = Config::builder()
        .profile_from_env("DH_CONFIG_TEST_PROFILE_PLUGH")
        .profile("dev")
        .build()
        .unwrap();
    std::env::remove_var("DH_CONFIG_TEST_PROFILE_PLUGH");
    assert_eq!(config.profile(), Some("dev"));
}
