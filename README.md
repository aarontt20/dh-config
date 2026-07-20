# dh-config

A from-scratch **layered configuration** library for Rust.

Configuration is assembled from an ordered stack of *layers* — code defaults,
files (JSON / TOML / YAML), environment variables, command-line arguments, and
any custom source you implement — deep-merged so that later layers override
earlier ones. The stack is defined programmatically with a builder; there is
no macro DSL and no required CLI framework.

```rust
use dh_config::{CommandLine, Config, Defaults, Env, File};
use serde::Deserialize;

#[derive(Deserialize)]
struct AppConfig {
    server: Server,
    verbose: bool,
}

#[derive(Deserialize)]
struct Server {
    host: String,
    port: u16,
}

fn main() -> Result<(), dh_config::ConfigError> {
    let config = Config::builder()
        // 1. Lowest precedence: defaults defined in code.
        .with_layer(
            Defaults::new()
                .set("server.host", "127.0.0.1")
                .set("server.port", 8080)
                .set("verbose", false),
        )
        // 2. A required config file; app.prod.toml overlays it when the
        //    `prod` profile is active.
        .with_file("config/app.toml")
        // 3. An optional developer-local file (any supported format).
        .with_file(File::new("config/local.yaml").required(false))
        // 4. Environment: APP_SERVER__PORT=9000 → server.port.
        .with_env("APP")
        // 5. Highest precedence: --server.port=7000 --verbose, no clap needed.
        .with_layer(CommandLine::from_env())
        .profile_from_env("APP_PROFILE")
        .build()?;

    // Typed access…
    let app: AppConfig = config.deserialize()?;
    // …or dynamic access.
    let port: u16 = config.get("server.port")?;
    let _ = (app.verbose, port);
    Ok(())
}
```

When the typed struct is all you need, `extract()` collapses the last two
steps into one:

```rust,ignore
let app: AppConfig = Config::builder()
    .with_file("config/app.toml")
    .with_env("APP")
    .extract()?;
```

## The layering model

Every layer produces a `Value` tree (null / bool / integer / float / string /
array / table) with a table at its root. The builder loads the layers in the
order they were added and deep-merges them:

* **Tables merge recursively** — setting `server.port` in a higher layer
  keeps `server.host` from a lower one.
* **Scalars and arrays replace** — a higher layer's list wins wholesale.
* **Explicit nulls override** — a higher layer can deliberately unset a value.

This matches the conventional precedence stack used by most real-world
services (defaults → shipped file → local file → environment → CLI), but the
order is entirely yours: the builder merges whatever layers you add, in the
order you add them.

## Built-in layers

| Layer | Source | Notes |
|---|---|---|
| `Defaults` | code | From any `Serialize` struct, or key-by-key with `.set("a.b", v)` |
| `File` | JSON / TOML / YAML files | Format inferred from extension; extensionless paths probe all enabled formats; `.required(false)` for optional files |
| `Env` | environment variables | Prefix filtering (`APP_*`), `__` nesting separator, lenient scalar parsing |
| `CommandLine` | process arguments | `--dotted.key=value` and bare `--flag`; no clap or any other dependency |
| `Overrides` | code | Highest-precedence escape hatch for computed values and tests |

### Custom layers

Anything implementing the two-method `Layer` trait participates in merging
like a built-in:

```rust
use dh_config::{ConfigError, Layer, LayerContext, Value};

struct RemoteFlags;

impl Layer for RemoteFlags {
    fn name(&self) -> String {
        "remote-flags".into()
    }

    fn load(&self, _cx: &LayerContext<'_>) -> Result<Value, ConfigError> {
        let mut root = Value::Table(Default::default());
        root.set_path("features.new_ui", Value::Bool(true));
        Ok(root)
    }
}
```

This is also the intended integration point for real CLI parsers: parse
arguments however you like (clap, lexopt, by hand), then emit the results as
a layer.

## Provenance: "who set this?"

The merge records which layer supplied every value:

```rust,ignore
config.origin("server.port");   // Some("environment (APP_*)")
println!("{}", config.explain());
// profile: prod
// server.host = "0.0.0.0"  [file (config/app.toml)]
// server.port = 9000  [environment (APP_*)]
// ...
```

Type errors name the supplying layer automatically, so a bad override is
traced in one read:

```text
at `server.port`: expected u16, found boolean (value set by layer `environment (APP_*)`)
```

## Profiles

Select a profile explicitly (`.profile("prod")`) or from an environment
variable (`.profile_from_env("APP_PROFILE")`). While a profile is active,
every file layer also loads a profile-suffixed sibling — `config/app.toml`
plus `config/app.prod.toml` — and merges the overlay over the base. Overlay
files are always optional and only carry the differences.

## Environment variable mapping

With `Env::prefixed("APP")` (or the builder shorthand `.with_env("APP")`):

| Variable | Key path |
|---|---|
| `APP_DEBUG=true` | `debug` |
| `APP_SERVER__PORT=8080` | `server.port` |
| `APP_DATABASE_URL=…` | `database_url` |

The double separator (`__`, configurable) descends into nested tables; a
single `_` stays part of the key. Values parse leniently (`"true"` → bool,
`"8080"` → integer) unless disabled with `.parse_values(false)`, and the
deserializer is symmetric-lenient in the other direction, so a string-typed
field still accepts a numeric-looking value.

### Lists from env and CLI

`Vec` fields are reachable from single-string sources via an opt-in list
separator — opt-in because commas appear in legitimate scalar values (URLs,
DSNs):

```rust,ignore
.with_layer(Env::prefixed("APP").list_separator(","))          // APP_CORS__ORIGINS=a.com,b.com
.with_layer(CommandLine::from_env().list_separator(","))       // --cors.origins=a.com,b.com
```

A value without the separator stays scalar, and the deserializer coerces a
lone scalar into a one-element sequence, so `APP_TAGS=a` and `APP_TAGS=a,b`
both fill a `Vec<String>`.

## Cargo features

`json`, `toml`, and `yaml` gate the format parsers and are all enabled by
default. Consumers compile only what they use:

```toml
[dependencies]
dh-config = { version = "0.1", default-features = false, features = ["toml"] }
```

With no format features, the file layer is compiled out and the
code/env/CLI layers still work.

## Workspace layout

* `crates/dh-config` — the library.
* `examples/layered-app` — a runnable demo of the full stack:

```sh
cargo run -p layered-app
APP_PROFILE=prod cargo run -p layered-app
APP_SERVER__PORT=9000 cargo run -p layered-app
cargo run -p layered-app -- --server.port=7000 --verbose
```

## License

MIT OR Apache-2.0
