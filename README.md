# rust_minecraft_mod_example

[日本語版 README](./README_jp.md)

[![CI](https://github.com/topi-banana/rust_minecraft_mod_example/actions/workflows/ci.yml/badge.svg)](https://github.com/topi-banana/rust_minecraft_mod_example/actions/workflows/ci.yml)

A Fabric Minecraft mod toolchain that builds `.class` files and the mod
jar entirely from Rust — no Java toolchain, no Gradle, no Mixin Gradle
plugin. Server-side hooks are written as plain Rust functions annotated
with `#[inject]`; the corresponding Mixin metadata (target class, target
method, `@At`, bytecode body) lives next to them as a `MixinClass` trait
implementation in the builder. The host side loads the bundled `.so` /
`.dll` / `.dylib` at runtime and calls into it over JNI — no third-party
runtime such as wasmer-java required.

## How it works

Runtime code and build code sit in 1:1 correspondence: one payload is
one `.rs` file on each side.

```
native-payloads/examples/minecraft_server.rs           builder/src/mixins/minecraft_server.rs
 ┌──────────────────────────────────────────┐          ┌──────────────────────────────────────────┐
 │ #[inject_macro::inject]                  │          │ pub struct MinecraftServerMixin;         │
 │ fn hello() -> &'static str {             │   1:1    │ impl MixinClass for MinecraftServerMixin │
 │     "Hello from native!"                 │ <──────> │ {  fn target_class(&self) { ... }        │
 │ }                                        │          │    fn native_methods(&self) { ... }      │
 └──────────────────────────────────────────┘          │    fn methods(&self) { /* @Inject ... */}│
                       │                               │ }                                        │
   cargo build -p native-payloads                      └──────────────────────────────────────────┘
        --release --examples                                          │
                       ▼                                              ▼
   target/release/examples/libminecraft_server.{so,dll,dylib}   cargo run -p builder
     └── exported JNI symbol                                          │
         Java_com_example_runtime_NativePayloads_hello                ▼
                                                       out/hello-native-mod.jar
                                                         ├─ fabric.mod.json
                                                         ├─ hello-native-mod.mixins.json
                                                         ├─ com/example/mixin/MinecraftServerMixin.class
                                                         ├─ com/example/runtime/NativePayloads.class
                                                         ├─ com/example/runtime/NativeLoader.class
                                                         └─ native/<platform>/<libname>
```

1. The `#[inject]` proc macro is intentionally tiny: it wraps each
   annotated Rust function in a JNI shim exported as
   `Java_com_example_runtime_NativePayloads_<jni_escaped_fn>`. Nothing
   else is embedded in the cdylib.
2. `builder/src/main.rs` keeps a compile-time list:
   ```rust
   const MIXINS: &[&dyn MixinClass] = &[&MinecraftServerMixin, /* ... */];
   ```
   Each `MixinClass` impl declares its target class, the cdylib it
   maps to (`native_lib_name`), the Mixin class' simple name, the
   native methods to expose on the shared holder, and the `@Inject`
   handlers (with their bytecode). The builder iterates the list and
   emits:
   - `com/example/mixin/<MixinName>.class` per mixin.
   - `com/example/runtime/NativePayloads.class` — a plain (non-mixin)
     class hosting every `native static String <fn>()` declaration.
     Mixins themselves can't host native methods because the Mixin
     processor would merge them into the target class and break JNI
     static binding.
   - `com/example/runtime/NativeLoader.class` — one
     `ensure_<lib>()V` synchronized method and `loaded_<lib>: Z` flag
     per cdylib, sharing a `resourcePath(String libBasename)` helper
     that resolves the in-jar path based on `os.name` / `os.arch`.
3. At runtime, each Mixin handler calls
   `NativeLoader.ensure_<lib>()` once, which extracts the appropriate
   `.so` / `.dll` / `.dylib` from the jar into a temp file (registered
   for cleanup via `deleteOnExit`) and `System.load`s it. From there
   the JVM binds `NativePayloads.<fn>()` to the Rust-side JNI shims.

## Prerequisites

- Rust 1.95+
- A Minecraft + [Fabric Loader] 0.15+ install on Minecraft 1.20+ for the
  end-to-end test.

[Fabric Loader]: https://fabricmc.net/

## Build

### Host platform only

```
cargo run -p builder
```

The builder invokes `cargo build -p native-payloads --release --examples`
itself, then packages everything into `out/hello-native-mod.jar`. Drop
the jar into `<minecraft>/mods/` next to a Fabric Loader install; on
server start the Mixin runs at `MinecraftServer#runServer @At("HEAD")`
and you see `Hello from native!` in the log.

### Linux + Windows from a single Linux (or WSL2) host

The Windows MSVC toolchain isn't usable from WSL2, but `gnu` via
mingw-w64 cross-compiles fine:

```
rustup target add x86_64-pc-windows-gnu
sudo apt install -y mingw-w64

cargo build -p native-payloads --release --examples
cargo build -p native-payloads --release --examples --target x86_64-pc-windows-gnu

mkdir -p staging/linux-x86_64 staging/windows-x86_64
cp target/release/examples/libminecraft_server.so                       staging/linux-x86_64/
cp target/x86_64-pc-windows-gnu/release/examples/minecraft_server.dll   staging/windows-x86_64/

NATIVE_LIB_DIRS=linux-x86_64=staging/linux-x86_64,windows-x86_64=staging/windows-x86_64 \
    cargo run -p builder
```

`NATIVE_LIB_DIRS` switches the builder into *aggregate mode*: it skips
the local cargo build and reads every `.so` / `.dll` / `.dylib` it can
find under each `<platform>=<dir>` mapping. Multiple payloads in the
same directory are all picked up.

### CI (all 6 platforms)

`build-natives` in [`.github/workflows/ci.yml`](./.github/workflows/ci.yml)
builds the cdylibs natively on `{linux, windows, macos} × {x86_64, aarch64}`
GitHub-hosted runners (`ubuntu-latest`, `ubuntu-22.04-arm`,
`windows-latest`, `windows-11-arm`, `macos-15-intel`, `macos-latest`).
The `package` job downloads all 6 artifacts and reruns
`cargo run -p builder` with `NATIVE_LIB_DIRS` covering every platform.

The `ubuntu-22.04-arm` and `windows-11-arm` ARM runners are free for
public repositories; private repos may need a paid plan.

## Adding a hook

A new payload takes one `.rs` file on each side plus three small
wiring edits.

1. **Runtime code** — `crates/native-payloads/examples/<name>.rs`:
   ```rust
   #[inject_macro::inject]
   fn run() -> &'static str { "another hook!" }
   ```
2. **Cargo.toml entry** — `crates/native-payloads/Cargo.toml`:
   ```toml
   [[example]]
   name = "<name>"
   path = "examples/<name>.rs"
   crate-type = ["cdylib"]
   ```
3. **Build code** — `crates/builder/src/mixins/<name>.rs`:
   ```rust
   use crustf::CodeBuilder;
   use super::{MixinClass, MixinMethod, NativeMethod};
   use crate::{NATIVE_LOADER_INTERNAL, NATIVE_PAYLOADS_OWNER};

   pub struct AnotherMixin;

   const NATIVES: &[NativeMethod] = &[NativeMethod {
       name: "run",
       descriptor: "()Ljava/lang/String;",
   }];

   const METHODS: &[MixinMethod] = &[MixinMethod {
       name: "onLoadWorld",
       descriptor: "(Lorg/spongepowered/asm/mixin/injection/callback/CallbackInfo;)V",
       target_method: "loadWorld",
       at: "TAIL",
       exceptions: &["java/io/IOException"],
       code: emit_on_load_world,
   }];

   fn emit_on_load_world(owner: &dyn MixinClass, c: &mut CodeBuilder) {
       c.max_stack(2);
       c.invokestatic(
           NATIVE_LOADER_INTERNAL,
           &format!("ensure_{}", owner.native_lib_name()),
           "()V",
       )
       .invokestatic(NATIVE_PAYLOADS_OWNER, "run", "()Ljava/lang/String;")
       .astore(2)
       .getstatic("java/lang/System", "out", "Ljava/io/PrintStream;")
       .aload(2)
       .invokevirtual("java/io/PrintStream", "println", "(Ljava/lang/String;)V")
       .return_void();
   }

   impl MixinClass for AnotherMixin {
       fn target_class(&self) -> &'static str { "net/minecraft/server/MinecraftServer" }
       fn mixin_class_simple_name(&self) -> &'static str { "AnotherMixin" }
       fn native_lib_name(&self) -> &'static str { "<name>" }
       fn native_methods(&self) -> &'static [NativeMethod] { NATIVES }
       fn methods(&self) -> &'static [MixinMethod] { METHODS }
   }
   ```
4. **Re-export** — `crates/builder/src/mixins/mod.rs`:
   ```rust
   pub mod <name>;
   pub use <name>::AnotherMixin;
   ```
5. **List in main** — `crates/builder/src/main.rs`:
   ```rust
   const MIXINS: &[&dyn MixinClass] = &[&MinecraftServerMixin, &AnotherMixin];
   ```

Multiple `@Inject` handlers on the same Mixin class are just additional
entries in `METHODS` — `MinecraftServerMixin` already ships two (HEAD
and RETURN of `runServer`) as a demo.

## `#[inject]` contract

`#[inject_macro::inject]` takes no arguments. It emits a JNI wrapper
exported under
`Java_com_example_runtime_NativePayloads_<jni_escaped_fn_name>` so the
JVM binds the corresponding `native static String <fn>()` declaration
on `NativePayloads` automatically once `System.load` has run.

The annotated function must currently return `&'static str` (or any
value that coerces to `&str`); the macro hands it back to Java via
`JNIEnv::new_string`.

Target class, target method, `@At` injection point, and Mixin class
name all live in the `MixinClass` impl on the builder side — single
source of truth, no duplication between cdylib metadata and the
generated classes.

## Workspace layout

```
crates/
├── inject-macro/      proc-macro: emits the JNI wrapper for #[inject]
├── native-payloads/   one cdylib per `examples/<name>.rs` ([[example]])
└── builder/           one MixinClass impl per `src/mixins/<name>.rs`;
                       main.rs lists them in `const MIXINS` and emits
                       the mod jar via crustf
```

## Limitations

- One signature shape: `fn() -> &'static str`. Other signatures are not
  supported in v1.
- `compatibilityLevel: JAVA_8` on the Mixin (class file version 52);
  the generated `onRun` body is branchless so no `StackMapTable` is
  needed. `NativeLoader` is emitted at class file version 49 (Java 5)
  so its OS-detection branches don't require a `StackMapTable` either.
- CI covers the full 6-way `{linux, windows, macos} × {x86_64, aarch64}`
  matrix using native GitHub-hosted runners (no cross-compilation).
  Additional architectures (e.g. `riscv64`) can be added by appending a
  matrix entry plus an entry in the `package` job's `NATIVE_LIB_DIRS`
  env var.

## License

Choose one. Suggested: dual MIT / Apache-2.0.

[`crustf`]: https://github.com/topi-banana/crustf
