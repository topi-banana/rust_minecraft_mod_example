# rust_minecraft_mod_example

[日本語版 README](./README_jp.md)

[![CI](https://github.com/topi-banana/rust_minecraft_mod_example/actions/workflows/ci.yml/badge.svg)](https://github.com/topi-banana/rust_minecraft_mod_example/actions/workflows/ci.yml)

A Fabric Minecraft mod toolchain that builds `.class` files and the mod
jar entirely from Rust — no Java toolchain, no Gradle, no Mixin Gradle
plugin. Server-side hooks are written as plain `cdylib` Rust functions
annotated with a single `#[inject(...)]` attribute. The host side loads
the bundled `.so` / `.dll` / `.dylib` at runtime and calls into it over
JNI — no third-party runtime such as wasmer-java required.

## How it works

```
crates/native-payloads/src/lib.rs
  ┌────────────────────────────────────────────────────────────┐
  │ #[inject(target = "Lnet/.../MinecraftServer;",             │
  │         method = "runServer", at = "HEAD",                 │
  │         class  = "hello_Mixin")]                           │
  │ fn hello() -> &'static str { "Hello from native!" }        │
  └────────────────────────────────────────────────────────────┘
                              │
                  cargo build -p native-payloads --release
                              ▼
   target/release/libnative_payloads.{so,dll,dylib}
     ├── exported JNI symbol  Java_com_example_mixin_hello_1Mixin_hello
     └── section `.inject_meta` (or `__injmeta` / `.injmta`)
                              │
                       cargo run -p builder
                              ▼
   out/hello-native-mod.jar
     ├─ fabric.mod.json
     ├─ hello-native-mod.mixins.json
     ├─ com/example/mixin/hello_Mixin.class          ← crustf-generated Mixin
     ├─ com/example/runtime/NativeLoader.class       ← crustf-generated loader
     └─ native/<platform>/<libname>                  ← per-OS native libraries
```

1. The `#[inject]` proc macro emits two artifacts per annotated function:
   - A JNI wrapper named `Java_<pkg>_<class>_<method>` that the JVM
     resolves automatically once the library is loaded.
   - A `(fn_name, target, method, at, class)` record in a small custom
     section (`.inject_meta` on ELF, `__DATA,__injmeta` on Mach-O,
     `.injmta` on PE) that `builder` reads later.
2. The host bin (`builder`) uses [`cargo_metadata`] to find the
   `native-payloads` cdylib, [`object`] to read its `inject_meta`
   section, and [`crustf`] to emit one Mixin class per record plus a
   shared `NativeLoader` utility class.
3. At runtime, the Mixin class triggers `NativeLoader.ensureLoaded()`,
   which extracts the bundled native lib for the current
   `os.name`/`os.arch` pair into a temp file, registers it for cleanup
   with `deleteOnExit`, and calls `System.load(...)`. After that, the
   `native String <fn_name>()` declaration on the Mixin class is bound
   by the JVM to the Rust-side JNI wrapper.

## Prerequisites

- Rust 1.95+
- A Minecraft + [Fabric Loader] 0.15+ install on Minecraft 1.20+ for the
  end-to-end test.

[Fabric Loader]: https://fabricmc.net/

## Build

```
cargo run -p builder
```

This compiles `native-payloads` for the **host** target only (so the
resulting jar carries one platform's `.so` / `.dll` / `.dylib`) and
writes `out/hello-native-mod.jar`. Drop it into `<minecraft>/mods/`. On
server start the Mixin injects at `MinecraftServer#runServer
@At("HEAD")` and you should see `Hello from native!` in the log.

Multi-platform jars are produced by CI — see
[`.github/workflows/ci.yml`](./.github/workflows/ci.yml). The `build-natives`
matrix builds the cdylib natively on 6 GitHub-hosted runners covering
the `{linux, windows, macos} × {x86_64, aarch64}` matrix
(`ubuntu-latest`, `ubuntu-22.04-arm`, `windows-latest`, `windows-11-arm`,
`macos-13`, `macos-14`). The `package` job collects all 6 artifacts and
reruns `cargo run -p builder` with `NATIVE_LIB_DIRS=linux-x86_64=...,
linux-aarch64=...,windows-x86_64=...,windows-aarch64=...,macos-x86_64=...,
macos-aarch64=...` so the final jar runs on every supported platform.

The `ubuntu-22.04-arm` and `windows-11-arm` ARM runners are free for
public repositories; private repos may need a paid plan.

## Adding a hook

1. Add a function to `crates/native-payloads/src/lib.rs` (or a module
   you create alongside it).
2. Annotate it with `#[inject(...)]` and pick a unique Mixin class
   simple name for each annotated function (or share one when several
   methods live on the same Mixin).
3. `cargo run -p builder` picks it up automatically and adds another
   Mixin class to the jar.

```rust
#[inject_macro::inject(
    target = "Lnet/minecraft/server/MinecraftServer;",
    method = "loadWorld",
    at = "TAIL",
    class = "load_world_Mixin"
)]
fn run() -> &'static str {
    "another hook!"
}
```

## `#[inject]` contract

| field    | required                | meaning                                                                                                |
|----------|-------------------------|--------------------------------------------------------------------------------------------------------|
| `target` | yes                     | JVM internal-form descriptor of the class to mix into                                                  |
| `method` | yes                     | name of the method on the target class                                                                 |
| `at`     | no — defaults to `HEAD` | Mixin `@At` injection point: `HEAD`, `TAIL`, `RETURN`, etc.                                            |
| `class`  | yes                     | Simple class name of the Mixin (under `com.example.mixin`). Used to derive the JNI symbol name.        |

The annotated function must currently return `&'static str` (or any
value that coerces to `&str`). The proc macro wraps that body in a JNI
shim that hands the value back to the Mixin via `JNIEnv::new_string`.

## Workspace layout

```
crates/
├── inject-macro/      proc-macro for #[inject(target, method, at, class)]
├── native-payloads/   cdylib with the inject hooks (host-native build)
└── builder/           host bin that emits the mod jar via crustf
```

`native-payloads` builds as a regular cdylib for the host target, so
`cargo build` at the workspace root just works. The builder invokes
`cargo build -p native-payloads --release` itself when needed.

## Limitations

- One signature shape: `fn() -> &'static str`. Other signatures are not
  supported in v1.
- `compatibilityLevel: JAVA_8` on the Mixin (class file version 52);
  the generated `onRun` body is branchless so no `StackMapTable` is
  needed. The `NativeLoader` is emitted at class file version 49 (Java 5)
  so its OS-detection branches don't require a `StackMapTable` either.
- CI covers the full 6-way `{linux, windows, macos} × {x86_64, aarch64}`
  matrix using native GitHub-hosted runners (no cross-compilation).
  Additional architectures (e.g., `riscv64`) can be added by appending a
  matrix entry plus an entry in the `package` job's
  `NATIVE_LIB_DIRS` env var.

## License

Choose one. Suggested: dual MIT / Apache-2.0.

[`crustf`]: https://github.com/topi-banana/crustf
[`cargo_metadata`]: https://crates.io/crates/cargo_metadata
[`object`]: https://crates.io/crates/object
