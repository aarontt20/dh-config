# Placeholder expansion in config files — feasibility & design

*Status: **implemented** (approach A) — see `crates/dh-config/src/expand.rs`,
`ConfigBuilder::expand_placeholders`, and `tests/expansion.rs`. This document
is kept as the design rationale.*

The feature request: let a config value reference other values, so a file can
say

```toml
[database]
host = "db.internal"
port = 5432
url  = "postgres://${database.host}:${database.port}/app"

[auth]
secret = "${env:APP_AUTH_SECRET}"
```

and the resolved config contains the assembled URL and the secret pulled from
the environment.

## Feasibility

**High.** Everything the feature needs already exists in the library:

* All layers converge on one `Value` tree (`value.rs`), so expansion is a
  single tree-to-tree transformation, not N per-format features.
* `Value::get_path` already resolves the dotted-path syntax placeholders
  would use, including array indexing (`peers.0.host`).
* `ConfigBuilder::build` (`builder.rs:142`) has an obvious seam: after the
  merge loop, before `Config::from_parts`.
* The library is dependency-light by design; a hand-written scanner for
  `${…}` is ~50 lines and needs no regex crate.

Estimated size: one new module (~200–300 lines), two new `ConfigError`
variants, one builder method, tests and docs. No new dependencies, no
breaking changes (the feature is opt-in, see below).

The one genuinely interesting design problem is *when* expansion runs
relative to merging, and what it does to the library's flagship feature,
provenance. Both are worked through below.

## Prior art, briefly

| System | Syntax | Resolved against | Notes |
|---|---|---|---|
| HOCON / Typesafe Config | `${a.b}`, `${?a.b}` optional | the **merged** tree | resolution explicitly happens after merging, so overrides retro-affect references |
| Spring Boot | `${prop}` with `:default` | full property environment | `:` doubles as default separator |
| docker-compose | `${VAR}`, `${VAR:-def}`, `${VAR:?err}` | environment only | shell-style operators |
| systemd | `%h`, `%i` specifiers | fixed specifier set | not general-purpose |
| Rust `config-rs` / `figment` | — | — | no built-in interpolation; users layer `shellexpand` or templating on top |

The HOCON lesson is the important one: in a layered system, substitution
must see the *final merged* values, otherwise overrides silently don't
propagate into derived values.

## Approaches

### A. Post-merge resolution pass (recommended)

Run expansion inside `build()`, on the fully merged `root`, after the layer
loop and before `Config::from_parts`.

* **Cross-layer references work**: `app.toml` can say
  `url = "…${server.host}…"` and pick up a `server.host` supplied by env or
  CLI — this is the behavior users coming from HOCON/Spring expect.
* **Overrides compose correctly**: raising `database.port` via
  `APP_DATABASE__PORT=6432` changes the expanded `database.url` too, because
  the reference is resolved after the override lands. Any per-layer scheme
  gets this wrong.
* **Profiles come for free**: overlay files are merged before the pass runs.
* **Fail-fast is preserved**: dangling references and cycles are build-time
  errors, matching the library's existing "all errors at `build()`" posture.
* Cost: one extra walk of the tree, only when enabled.

Sketch:

```rust
// builder.rs, in build(), after the merge loop:
if self.expand {
    crate::expand::expand(&mut root)?;
}
```

The pass itself:

1. Walk the tree collecting leaf strings that contain an unescaped `${`.
2. For each, tokenize into literal/placeholder segments (hand-written
   scanner; `$$` escapes a literal `$`).
3. Resolve each placeholder by `get_path` against the merged root. If the
   referenced value is itself an unexpanded template, resolve it first —
   depth-first with an explicit path stack for cycle detection and a memo
   map so shared references resolve once.
4. Splice results back into the tree.

Borrow-checker note: the easiest correct shape is to resolve against an
immutable snapshot (the pre-expansion root) and build the expanded tree as a
copy, rather than mutating while reading. The tree was already cloned
per-layer during merge; one more clone in an opt-in path is fine.

### B. Per-layer expansion (a `Layer` decorator)

`File::new("app.toml").expand()` or `Expanded(inner_layer)`: expand each
layer's tree as it loads, resolving only against that layer's own values
(plus the environment).

* Composable and local — fits the `Layer` trait aesthetically.
* **Fatal flaw**: no cross-layer references, and overrides don't propagate
  (env raising `server.port` won't be seen by a file-local
  `${server.port}`). A variant that exposes the merged-so-far tree via
  `LayerContext` fixes references *downward* but still not overrides from
  *later* layers, which is exactly the case users hit first.

Worth keeping in mind only if someone later needs expansion scoped to a
single exotic layer; not a foundation.

### C. Lazy expansion at read time

Keep the raw tree; expand inside `get`/`deserialize` when a value is read.

* Pro: `explain()` could show the unexpanded template; no build-time cost.
* Con: errors (typos, cycles) surface at first *read* of the affected key,
  possibly deep into runtime — the opposite of the library's fail-fast
  design. Repeated reads re-pay the cost or need a cache with interior
  mutability. `extract()` reads everything anyway, so laziness buys little.

### D. Full template engine (minijinja/Tera)

Loops and conditionals in config files. Heavy dependency, invites logic
into configuration, and the escaping/typing story gets much worse. Out of
scope for this library's ethos; a user who wants this can preprocess files
themselves and hand the result to a custom `Layer`.

**Verdict: A.** Eager, post-merge, opt-in.

## Proposed semantics

### Syntax

* `${dotted.path}` — reference into the merged config tree
  (`get_path` syntax, so `peers.0.host` works).
* `${env:VAR}` — direct environment lookup. Namespaced with `:` so future
  sources (`file:`, `secret:`) can be added without breaking anything.
* `${…:-default}` — shell-style default, used when the path/var is missing.
  `:-` (not bare `:`) so it cannot collide with the `env:` namespace
  separator. The default is a literal string (parsed leniently like env
  values); no nesting inside defaults in v1.
* `$$` — escape: `"$${HOME}"` yields the literal `${HOME}`. Escaping works
  even when expansion is disabled? No — when disabled, strings pass through
  completely untouched, so enabling expansion is the only thing that
  changes meaning.

### Typing

* **Whole-string placeholder** — `port = "${database.port}"` where the
  placeholder is the entire string: the referenced `Value` is spliced in
  verbatim, preserving its type (integer stays integer; tables and arrays
  may be referenced too, which doubles as a cheap "anchor/alias" feature).
* **Interpolation** — placeholder embedded in a longer string: scalars are
  rendered with their `Display` form and concatenated. Referencing a
  `null`, table, or array in interpolation context is an error (silently
  producing `"{host: …}"` inside a URL helps nobody).

### Errors

Two new `ConfigError` variants, in the house style (full paths, named
sources):

```text
at `database.url`: placeholder `${database.hots}` does not resolve
(value set by layer `file (config/app.toml)`)

placeholder cycle: `a.url` -> `b.url` -> `a.url`
```

The first reuses the existing origins map to name the layer that supplied
the offending template — the provenance machinery pays off here.

### Provenance interaction

Expansion changes values, not paths, so the `origins` map stays valid as-is:
an expanded value is attributed to the layer that supplied the *template*
string. That is the honest answer to "who set this?" — the template author
chose to derive the value. Two refinements worth doing:

* `explain()` gains nothing automatically; a nice follow-up is annotating
  expanded leaves, e.g. `database.url = "postgres://db:5432/app"
  [file (config/app.toml), expanded]`. Requires the pass to report which
  paths it touched (a `BTreeSet<String>` handed to `Config`).
* Type errors already flow through `attach_origin`, so a `u16` field fed a
  bad expanded string still names the template's layer with no extra work.

### Opt-in

```rust
let config = Config::builder()
    .with_file("config/app.toml")
    .with_env("APP")
    .expand_placeholders()   // off by default
    .build()?;
```

Off by default because `${…}` occurs in legitimate scalar values (shell
snippets, Grafana/CI template strings stored in config), and because a
silent behavior change in existing trees is unacceptable. A boolean builder
method is enough for v1; if knobs accumulate (disable `env:`, custom max
depth), graduate to `.expand(Expansion::new().…)` — same seam, no breakage.

Expansion applies to the whole merged tree, i.e. env- and CLI-supplied
strings expand too. That is a deliberate simplification: scoping to "file
layers only" would require threading origin data through the pass, and
consistency ("every layer feeds one tree, one rule set") is the library's
existing story. Documented, with `$$` as the escape hatch.

## Edge cases the implementation must nail

* **Cycles** — explicit stack during depth-first resolution; report the full
  chain. A depth cap (~64) as a belt-and-suspenders backstop.
* **Chained references** — `a = "${b}"`, `b = "${c}"`: memoized DFS
  resolves each path once, in dependency order.
* **Null** — whole-string reference to null splices `Null` (which
  `get_opt`/`Option` fields already handle); null in interpolation is an
  error.
* **Missing `env:` var** — error unless a `:-default` is given, mirroring
  required-file behavior.
* **Unterminated `${`** — error at the containing path, not silent
  passthrough (silent passthrough hides typos).
* **Arrays** — strings inside arrays expand; array *elements* are reachable
  as references via existing index paths.
* **List separators** — env `list_separator` splitting happens at layer load
  time, before expansion; an expansion that *introduces* a comma is not
  re-split. Correct and documented.
* **No nested placeholders** — `${a${b}}` is rejected; keeps the scanner
  trivial and the files readable.

## Recommendation

1. Implement **Approach A**: eager post-merge pass, opt-in via
   `.expand_placeholders()`, new `expand.rs` module, no new dependencies.
2. Syntax: `${config.path}`, `${env:VAR}`, `${…:-default}`, `$$` escape.
3. Whole-string references splice with type preserved; interpolation
   stringifies scalars and rejects null/containers.
4. Build-time errors for dangling references (naming the supplying layer via
   the existing origins map) and cycles (printing the chain).
5. Follow-ups, in order of value: mark expanded leaves in `explain()`;
   `${?opt.path}`-style optional references (HOCON-like: vanish if absent);
   pluggable resolver namespaces (`file:`, `secret:`) behind the same
   `name:arg` syntax.
