# rust_minecraft_mod_example

[English README](./README.md)

[![CI](https://github.com/topi-banana/rust_minecraft_mod_example/actions/workflows/ci.yml/badge.svg)](https://github.com/topi-banana/rust_minecraft_mod_example/actions/workflows/ci.yml)

Java ツールチェーン・Gradle・Mixin Gradle plugin を一切使わず、`.class`
ファイルと mod jar を全て Rust から組み立てる Fabric Minecraft mod 用
ツールチェーンです。サーバー側のフックは普通の Rust 関数として書き、
引数なしの `#[inject]` を付けるだけ。対応する Mixin のメタデータ
(対象クラス、対象メソッド、`@At`、bytecode 本体) は builder 側の
`MixinClass` trait 実装として隣に置きます。ホスト側は同梱した `.so` /
`.dll` / `.dylib` を実行時にロードし、JNI 経由で関数を呼び出します
(wasmer-java など外部ランタイムは不要)。

## 仕組み

実行時コードとビルド時コードは 1:1 で対応します。1 ペイロード = 両側で
1 ファイルずつ。

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
     └── exported JNI シンボル                                        │
         Java_com_example_runtime_NativePayloads_hello                ▼
                                                       out/hello-native-mod.jar
                                                         ├─ fabric.mod.json
                                                         ├─ hello-native-mod.mixins.json
                                                         ├─ com/example/mixin/MinecraftServerMixin.class
                                                         ├─ com/example/runtime/NativePayloads.class
                                                         ├─ com/example/runtime/NativeLoader.class
                                                         └─ native/<platform>/<libname>
```

1. `#[inject]` proc macro は意図的に小さく作ってあり、対象関数を
   `Java_com_example_runtime_NativePayloads_<jni_escaped_fn>` という
   JNI シムでラップして cdylib にエクスポートするだけ。cdylib にメタ
   情報を埋め込んだりはしません。
2. `builder/src/main.rs` がコンパイル時のリストを持ちます:
   ```rust
   const MIXINS: &[&dyn MixinClass] = &[&MinecraftServerMixin, /* ... */];
   ```
   各 `MixinClass` impl が「注入先クラス」「対応する cdylib 名
   (`native_lib_name`)」「Mixin クラスの simple name」「holder に
   公開する native メソッド一覧」「`@Inject` ハンドラ群 (bytecode 込)」
   を宣言します。builder はこのリストを舐めて以下を生成します:
   - `com/example/mixin/<MixinName>.class` (Mixin ごとに 1 つ)
   - `com/example/runtime/NativePayloads.class` — 全 `native static
     String <fn>()` 宣言を集める普通の (非 mixin) クラス。Mixin に
     native メソッドを直接置くと Mixin プロセッサがターゲットクラスに
     マージしてしまい JNI 静的バインディングが壊れるので、別 holder
     に逃がしています。
   - `com/example/runtime/NativeLoader.class` — cdylib ごとに
     `ensure_<lib>()V` synchronized メソッドと `loaded_<lib>: Z`
     フラグを生成し、`resourcePath(String libBasename)` ヘルパーで
     `os.name` / `os.arch` を見て jar 内パスを解決します。
3. ランタイムでは各 Mixin ハンドラがまず `NativeLoader.ensure_<lib>()`
   を呼びます。これが現在の `os.name`/`os.arch` に対応する `.so`/
   `.dll`/`.dylib` を jar から temp file に展開し、`deleteOnExit` で
   後始末を予約してから `System.load(...)`。以降 JVM が
   `NativePayloads.<fn>()` を Rust 側 JNI シムに静的バインドします。

## 必要な物

- Rust 1.95+
- 実機検証には Minecraft + [Fabric Loader] 0.15+ on Minecraft 1.20+。

[Fabric Loader]: https://fabricmc.net/

## ビルド

### ホスト platform 1 つだけ

```
cargo run -p builder
```

builder が内部で `cargo build -p native-payloads --release --examples`
を呼び、`out/hello-native-mod.jar` を生成します。Fabric Loader を
入れた Minecraft の `<minecraft>/mods/` に投入してサーバー起動すると、
`MinecraftServer#runServer` の `@At("HEAD")` に注入された Mixin が走り、
ログに `Hello from native!` が出ます。

### Linux + Windows を 1 つの Linux (もしくは WSL2) ホストで

WSL2 から MSVC ターゲットは使えませんが、mingw-w64 経由の `gnu`
ターゲットはクロスコンパイルできます:

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

`NATIVE_LIB_DIRS` を設定すると builder は **集約モード** に切り替わり、
ローカルの cargo build をスキップして各 `<platform>=<dir>` のディレクトリ
にある `.so` / `.dll` / `.dylib` を全部取り込みます。同じディレクトリに
複数のペイロードが入っていても全部拾われます。

### CI (全 6 platform)

[`.github/workflows/ci.yml`](./.github/workflows/ci.yml) の
`build-natives` matrix が 6 種類の GitHub-hosted ランナー
(`ubuntu-latest` / `ubuntu-22.04-arm` / `windows-latest` /
`windows-11-arm` / `macos-15-intel` / `macos-latest`) でネイティブに
cdylib をビルドし、`{linux, windows, macos} × {x86_64, aarch64}` の
組み合わせを全部カバーします。`package` ジョブが 6 つの artifact を
集約し、`NATIVE_LIB_DIRS=...` 付きで `cargo run -p builder` を再実行
して全 platform 対応の jar を出力します。

`ubuntu-22.04-arm` と `windows-11-arm` の ARM ランナーは **public
リポジトリでは無料**で利用できます (private リポジトリでは有料プランが
必要な場合があります)。

## フックの追加

ペイロードを 1 つ増やすには、両側で `.rs` ファイルを 1 つずつ + 3 箇所の
配線を行います。

1. **実行時コード** — `crates/native-payloads/examples/<name>.rs`:
   ```rust
   #[inject_macro::inject]
   fn run() -> &'static str { "another hook!" }
   ```
2. **Cargo.toml エントリ** — `crates/native-payloads/Cargo.toml`:
   ```toml
   [[example]]
   name = "<name>"
   path = "examples/<name>.rs"
   crate-type = ["cdylib"]
   ```
3. **ビルド時コード** — `crates/builder/src/mixins/<name>.rs`:
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
4. **re-export** — `crates/builder/src/mixins/mod.rs`:
   ```rust
   pub mod <name>;
   pub use <name>::AnotherMixin;
   ```
5. **main の MIXINS に追加** — `crates/builder/src/main.rs`:
   ```rust
   const MIXINS: &[&dyn MixinClass] = &[&MinecraftServerMixin, &AnotherMixin];
   ```

同じ Mixin クラスに `@Inject` ハンドラを複数置きたい場合は `METHODS`
配列にエントリを足すだけです。 `MinecraftServerMixin` は既にデモとして
2 つ (`runServer` の HEAD と RETURN) 載せています。

## `#[inject]` の仕様

`#[inject_macro::inject]` は引数なしの attribute マクロ。Rust 関数を
`Java_com_example_runtime_NativePayloads_<jni_escaped_fn_name>` という
JNI シンボルでエクスポートします。`System.load` 後、JVM が
`NativePayloads.<fn>()` の `native` 宣言と自動的に静的バインドします。

対象関数は現状必ず `&'static str` (もしくは `&str` に coerce する型)
を返す必要があります。proc macro はこの本体を JNI シムでラップし、
`JNIEnv::new_string` 経由で Java `String` を返します。

注入先クラス・注入先メソッド・`@At` 位置・Mixin クラス名はすべて
builder 側の `MixinClass` impl に集約されており、cdylib のメタデータと
生成 classfile の間で情報が二重化することはありません。

## ワークスペース構成

```
crates/
├── inject-macro/      proc-macro: #[inject] に JNI ラッパーを生成
├── native-payloads/   examples/<name>.rs が 1 ファイル = 1 cdylib ([[example]])
└── builder/           src/mixins/<name>.rs ごとに MixinClass impl 1 つ。
                       main.rs の `const MIXINS` に列挙し、crustf で mod
                       jar を出力する host bin
```

## 制限事項

- シグネチャは `fn() -> &'static str` のみ対応 (v1)。
- Mixin の `compatibilityLevel: JAVA_8` (class file version 52)。生成
  される `onRun` は分岐なしなので `StackMapTable` 不要。`NativeLoader`
  は class file version 49 (Java 5) で出力され、OS 判定の分岐があっても
  `StackMapTable` 不要になっている。
- CI は `{linux, windows, macos} × {x86_64, aarch64}` の全 6 組み合わせ
  をネイティブの GitHub-hosted ランナーでカバー (クロスコンパイル不要)。
  他のアーキテクチャ (例: `riscv64`) を追加したい場合は matrix エントリ
  と `package` ジョブの `NATIVE_LIB_DIRS` env を 1 行ずつ足すだけ。

## ライセンス

未定。MIT / Apache-2.0 dual を推奨。

[`crustf`]: https://github.com/topi-banana/crustf
