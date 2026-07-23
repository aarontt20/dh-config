//! End-to-end tests of `${…}` placeholder expansion: cross-layer references,
//! type-preserving splices, env lookups, defaults, escaping, provenance, and
//! the error cases.

use dh_config::{Config, ConfigError, Defaults, Env, Overrides, Value};
use serde::Deserialize;

fn env_source(vars: &[(&str, &str)]) -> Vec<(String, String)> {
    vars.iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn interpolates_across_layers() {
    let config = Config::builder()
        .with_layer(
            Defaults::new()
                .set("database.host", "localhost")
                .set("database.port", 5432)
                .set(
                    "database.url",
                    "postgres://${database.host}:${database.port}/app",
                ),
        )
        .with_layer(
            Env::prefixed("APP").source(env_source(&[("APP_DATABASE__HOST", "db.internal")])),
        )
        .expand_placeholders()
        .build()
        .unwrap();

    let url: String = config.get("database.url").unwrap();
    // The env override of `database.host` flows into the derived URL.
    assert_eq!(url, "postgres://db.internal:5432/app");
}

#[test]
fn whole_string_reference_preserves_type() {
    #[derive(Deserialize)]
    struct App {
        alias_port: u16,
        flag_copy: bool,
    }

    let config = Config::builder()
        .with_layer(
            Defaults::new()
                .set("server.port", 8080)
                .set("flag", true)
                .set("alias_port", "${server.port}")
                .set("flag_copy", "${flag}"),
        )
        .expand_placeholders()
        .build()
        .unwrap();

    assert_eq!(
        config.root().get_path("alias_port"),
        Some(&Value::Integer(8080))
    );
    let app: App = config.deserialize().unwrap();
    assert_eq!(app.alias_port, 8080);
    assert!(app.flag_copy);
}

#[test]
fn whole_string_reference_splices_tables() {
    let config = Config::builder()
        .with_layer(
            Defaults::new()
                .set("primary.host", "a.internal")
                .set("primary.port", 5432),
        )
        .with_layer(Overrides::new().set("replica", "${primary}"))
        .expand_placeholders()
        .build()
        .unwrap();

    assert_eq!(config.get::<String>("replica.host").unwrap(), "a.internal");
    assert_eq!(config.get::<u16>("replica.port").unwrap(), 5432);
    // The spliced leaves are attributed to the layer that wrote the template.
    assert_eq!(config.origin("replica.host"), Some("overrides"));
    // …and explain() marks them as expanded.
    let explain = config.explain();
    assert!(explain.contains("replica.host = \"a.internal\"  [overrides, expanded]"));
    // Untouched values carry no marker.
    assert!(explain.contains("primary.host = \"a.internal\"  [defaults]"));
}

#[test]
fn chained_references_resolve_in_dependency_order() {
    let config = Config::builder()
        .with_layer(
            Defaults::new()
                .set("a", "${b}/a")
                .set("b", "${c}/b")
                .set("c", "root"),
        )
        .expand_placeholders()
        .build()
        .unwrap();

    assert_eq!(config.get::<String>("a").unwrap(), "root/b/a");
    assert_eq!(config.get::<String>("b").unwrap(), "root/b");
}

#[test]
fn env_namespace_reads_process_environment() {
    std::env::set_var("DH_CONFIG_TEST_SECRET", "hunter2");
    std::env::set_var("DH_CONFIG_TEST_PORT", "6432");
    let config = Config::builder()
        .with_layer(
            Defaults::new()
                .set("secret", "${env:DH_CONFIG_TEST_SECRET}")
                .set("port", "${env:DH_CONFIG_TEST_PORT}")
                .set("greeting", "hello ${env:DH_CONFIG_TEST_SECRET}"),
        )
        .expand_placeholders()
        .build()
        .unwrap();

    assert_eq!(config.get::<String>("secret").unwrap(), "hunter2");
    // Whole-string env references parse leniently, like the Env layer.
    assert_eq!(config.root().get_path("port"), Some(&Value::Integer(6432)));
    assert_eq!(config.get::<String>("greeting").unwrap(), "hello hunter2");
}

#[test]
fn defaults_cover_missing_references() {
    let config = Config::builder()
        .with_layer(
            Defaults::new()
                .set("workers", "${env:DH_CONFIG_TEST_UNSET_VAR:-4}")
                .set("log", "${log.level:-info}")
                .set("banner", "level=${log.level:-info}"),
        )
        .expand_placeholders()
        .build()
        .unwrap();

    // A whole-string default parses leniently…
    assert_eq!(config.root().get_path("workers"), Some(&Value::Integer(4)));
    assert_eq!(config.get::<String>("log").unwrap(), "info");
    // …an interpolated one stays text.
    assert_eq!(config.get::<String>("banner").unwrap(), "level=info");
}

#[test]
fn escaping_and_lone_dollars() {
    let config = Config::builder()
        .with_layer(
            Defaults::new()
                .set("literal", "$${not.a.reference}")
                .set("price", "cost: $5"),
        )
        .expand_placeholders()
        .build()
        .unwrap();

    assert_eq!(
        config.get::<String>("literal").unwrap(),
        "${not.a.reference}"
    );
    assert_eq!(config.get::<String>("price").unwrap(), "cost: $5");
}

#[test]
fn expansion_is_off_by_default() {
    let config = Config::builder()
        .with_layer(
            Defaults::new()
                .set("host", "localhost")
                .set("url", "http://${host}/")
                .set("escaped", "$${x}"),
        )
        .build()
        .unwrap();

    // Without the opt-in, strings pass through completely untouched.
    assert_eq!(config.get::<String>("url").unwrap(), "http://${host}/");
    assert_eq!(config.get::<String>("escaped").unwrap(), "$${x}");
}

#[test]
fn strings_inside_arrays_expand() {
    let config = Config::builder()
        .with_layer(
            Defaults::new()
                .set("host", "a.internal")
                .set("peers", vec!["${host}:1", "${host}:2"])
                .set("first_peer", "${peers.0}"),
        )
        .expand_placeholders()
        .build()
        .unwrap();

    let peers: Vec<String> = config.get("peers").unwrap();
    assert_eq!(peers, vec!["a.internal:1", "a.internal:2"]);
    assert_eq!(config.get::<String>("first_peer").unwrap(), "a.internal:1");
}

#[test]
fn dangling_reference_names_path_and_layer() {
    let error = Config::builder()
        .with_layer(Defaults::new().set("url", "http://${server.hots}/"))
        .expand_placeholders()
        .build()
        .unwrap_err();

    let message = error.to_string();
    assert!(
        message.contains("at `url`")
            && message.contains("${server.hots}")
            && message.contains("does not resolve")
            && message.contains("value set by layer `defaults`"),
        "unexpected message: {message}"
    );
}

#[test]
fn missing_env_var_without_default_errors() {
    let error = Config::builder()
        .with_layer(Defaults::new().set("secret", "${env:DH_CONFIG_TEST_UNSET_VAR}"))
        .expand_placeholders()
        .build()
        .unwrap_err();

    let message = error.to_string();
    assert!(
        message.contains("DH_CONFIG_TEST_UNSET_VAR") && message.contains("is not set"),
        "unexpected message: {message}"
    );
}

#[test]
fn cycles_are_reported_with_their_chain() {
    let error = Config::builder()
        .with_layer(
            Defaults::new()
                .set("a.url", "${b.url}")
                .set("b.url", "${a.url}"),
        )
        .expand_placeholders()
        .build()
        .unwrap_err();

    match &error {
        ConfigError::PlaceholderCycle { chain } => {
            assert_eq!(chain.first(), chain.last());
            assert!(chain.contains(&"a.url".to_string()) && chain.contains(&"b.url".to_string()));
        }
        other => panic!("expected PlaceholderCycle, got: {other}"),
    }
    let message = error.to_string();
    assert!(
        message.starts_with("placeholder cycle: "),
        "unexpected message: {message}"
    );
}

#[test]
fn self_reference_is_a_cycle() {
    let error = Config::builder()
        .with_layer(Defaults::new().set("a", "${a}"))
        .expand_placeholders()
        .build()
        .unwrap_err();
    assert!(matches!(error, ConfigError::PlaceholderCycle { .. }));
}

#[test]
fn interpolating_null_or_containers_errors() {
    for (key, value) in [
        ("n", Value::Null),
        ("t", {
            let mut t = Value::Table(Default::default());
            t.set_path("x", Value::Integer(1));
            t
        }),
        ("l", Value::Array(vec![Value::Integer(1)])),
    ] {
        let error = Config::builder()
            .with_layer(
                Defaults::new()
                    .set(key, value)
                    .set("out", format!("v=${{{key}}}")),
            )
            .expand_placeholders()
            .build()
            .unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("cannot be interpolated"),
            "unexpected message: {message}"
        );
    }

    // But a whole-string reference to null splices null, which `Option`
    // fields accept.
    let config = Config::builder()
        .with_layer(Defaults::new().set("n", Value::Null).set("copy", "${n}"))
        .expand_placeholders()
        .build()
        .unwrap();
    assert_eq!(config.get_opt::<String>("copy").unwrap(), None);
}

#[test]
fn malformed_templates_error() {
    for template in ["${unclosed", "${}", "${a${b}}"] {
        let error = Config::builder()
            .with_layer(Defaults::new().set("bad", template))
            .expand_placeholders()
            .build()
            .unwrap_err();
        assert!(
            matches!(error, ConfigError::Expansion { .. }),
            "expected Expansion error for {template:?}, got: {error}"
        );
    }
}

#[test]
fn overrides_retroactively_change_derived_values() {
    // The same stack, built twice: the second time, a higher layer overrides
    // a referenced value, and the derived string follows.
    let stack = |port_override: Option<i64>| {
        let mut builder = Config::builder()
            .with_layer(
                Defaults::new()
                    .set("server.port", 8080)
                    .set("server.url", "http://localhost:${server.port}"),
            )
            .expand_placeholders();
        if let Some(port) = port_override {
            builder = builder.with_layer(Overrides::new().set("server.port", port));
        }
        builder.build().unwrap()
    };

    assert_eq!(
        stack(None).get::<String>("server.url").unwrap(),
        "http://localhost:8080"
    );
    assert_eq!(
        stack(Some(9000)).get::<String>("server.url").unwrap(),
        "http://localhost:9000"
    );
}
