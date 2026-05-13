# rust_minecraft_mod_example

[English README](./README.md)

[![CI](https://github.com/topi-banana/rust_minecraft_mod_example/actions/workflows/ci.yml/badge.svg)](https://github.com/topi-banana/rust_minecraft_mod_example/actions/workflows/ci.yml)

Java ツールチェーン・Gradle・Mixin Gradle plugin を一切使わず、`.class`
ファイルと mod jar を全て Rust から組み立てる Fabric Minecraft mod 用
ツールチェーンです。サーバー側のフックは通常の `cdylib` Rust 関数として
書き、`#[inject(...)]` 1 つで宣言します。ホスト側は同梱した `.so` /
`.dll` / `.dylib` を実行時にロードし、JNI 経由で関数を呼び出します
(wasmer-java など外部ランタイムは不要)。

## 仕組み

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
     ├── exported JNI シンボル  Java_com_example_mixin_hello_1Mixin_hello
     └── section `.inject_meta` (or `__injmeta` / `.injmta`)
                              │
                       cargo run -p builder
                              ▼
   out/hello-native-mod.jar
     ├─ fabric.mod.json
     ├─ hello-native-mod.mixins.json
     ├─ com/example/mixin/hello_Mixin.class          ← crustf 生成 Mixin
     ├─ com/example/runtime/NativeLoader.class       ← crustf 生成 loader
     └─ native/<platform>/<libname>                  ← OS 毎のネイティブライブラリ
```

1. `#[inject]` proc macro は対象関数につき 2 つの成果物を生成する:
   - `Java_<pkg>_<class>_<method>` という名前の JNI ラッパー。ライブラリ
     ロード後に JVM が自動でリンクする。
   - `(fn_name, target, method, at, class)` のレコードを ELF なら
     `.inject_meta`、Mach-O なら `__DATA,__injmeta`、PE なら
     `.injmta` という小さな section に埋め込み、`builder` が後で読む。
2. host 側の `builder` が [`cargo_metadata`] で `native-payloads` の
   cdylib を見つけ、[`object`] で `inject_meta` を読み、[`crustf`] で
   レコードごとに parameterize された Mixin class と共有の
   `NativeLoader` utility class を生成する。
3. ランタイムでは Mixin class の onRun が `NativeLoader.ensureLoaded()`
   を呼び、現在の `os.name`/`os.arch` に対応する native lib を一時
   ファイルへ展開して `deleteOnExit` で後始末を予約し、`System.load(...)`
   する。以降は Mixin の `native String <fn_name>()` 宣言が JVM によって
   Rust 側の JNI ラッパーに静的にバインドされる。

## 必要な物

- Rust 1.95+
- 実機検証には Minecraft + [Fabric Loader] 0.15+ on Minecraft 1.20+。

[Fabric Loader]: https://fabricmc.net/

## ビルド

```
cargo run -p builder
```

これは `native-payloads` を **ホストの target だけ** でビルドするため
出力 jar はホストプラットフォーム 1 つ分のみを含みます。`out/hello-native-mod.jar`
が生成されるので `<minecraft>/mods/` に投入してください。サーバー起動時
に `MinecraftServer#runServer` の `@At("HEAD")` に注入された Mixin が
走り、ログに `Hello from native!` が出ます。

複数プラットフォーム対応の jar は CI ジョブで生成されます
( [`.github/workflows/ci.yml`](./.github/workflows/ci.yml) 参照)。
`build-natives` matrix は 6 種類の GitHub-hosted ランナー
(`ubuntu-latest` / `ubuntu-22.04-arm` / `windows-latest` /
`windows-11-arm` / `macos-13` / `macos-14`) でネイティブに cdylib を
ビルドし、`{linux, windows, macos} × {x86_64, aarch64}` の組み合わせを
カバーします。`package` ジョブが 6 つの artifact を集約し、
`NATIVE_LIB_DIRS=linux-x86_64=...,linux-aarch64=...,windows-x86_64=...,windows-aarch64=...,macos-x86_64=...,macos-aarch64=...`
を渡して `cargo run -p builder` を再実行することで、全プラットフォーム
対応の jar を出力します。

`ubuntu-22.04-arm` と `windows-11-arm` の ARM ランナーは **public
リポジトリでは無料**で利用できます (private リポジトリでは有料プランが
必要な場合があります)。

## フックの追加

1. `crates/native-payloads/src/lib.rs` (または隣に作ったモジュール) に
   関数を追加。
2. `#[inject(...)]` を付け、Mixin class の simple name を `class` 引数
   で指定 (関数ごとに別 class を割り当てるか、同じ Mixin に複数 method
   を載せる場合は共有する)。
3. `cargo run -p builder` が自動で検出し、新しい Mixin class が jar に
   追加される。

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

## `#[inject]` の仕様

| フィールド | 必須                       | 意味                                                                              |
|------------|----------------------------|-----------------------------------------------------------------------------------|
| `target`   | はい                       | 注入先クラスの JVM internal-form descriptor                                       |
| `method`   | はい                       | 注入先メソッド名                                                                  |
| `at`       | いいえ — default は `HEAD` | Mixin `@At` の注入位置: `HEAD`, `TAIL`, `RETURN` 等                               |
| `class`    | はい                       | Mixin class の simple name (`com.example.mixin` 配下)。JNI シンボル名にも使われる |

対象関数は現状必ず `&'static str` (もしくは `&str` に coerce する型)
を返す必要があります。proc macro はこの本体を JNI シムでラップし、Mixin
に `JNIEnv::new_string` 経由で Java `String` を返します。

## ワークスペース構成

```
crates/
├── inject-macro/      proc-macro: #[inject(target, method, at, class)]
├── native-payloads/   inject フックを書く cdylib (ホスト target でビルド)
└── builder/           crustf で mod jar を生成する host bin
```

`native-payloads` はホスト target で普通の cdylib としてビルドされる
ので、ワークスペースルートで `cargo build` がそのまま通ります。builder
は必要なときに自身で `cargo build -p native-payloads --release` を
呼びます。

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
[`cargo_metadata`]: https://crates.io/crates/cargo_metadata
[`object`]: https://crates.io/crates/object
