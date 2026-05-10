# rust_minecraft_mod_example

[English README](./README.md)

[![CI](https://github.com/topi-banana/rust_minecraft_mod_example/actions/workflows/ci.yml/badge.svg)](https://github.com/topi-banana/rust_minecraft_mod_example/actions/workflows/ci.yml)

Java ツールチェーン・Gradle・Mixin Gradle plugin を一切使わず、`.class`
ファイルと mod jar を全て Rust から組み立てる Fabric Minecraft mod 用
ツールチェーンです。サーバー側のフックは `wasm32-unknown-unknown` の小さな
バイナリとして書き、`#[inject(...)]` 1 つで宣言します。ランタイムでは
[wasmer-java][wj] が wasm をロード・呼び出します。

[wj]: https://github.com/wasmerio/wasmer-java

## 仕組み

```
crates/wasm-payloads/src/bin/hello.rs
  ┌────────────────────────────────────────────────────────────┐
  │ #[inject(target = "Lnet/.../MinecraftServer;",             │
  │         method = "runServer", at = "HEAD")]                │
  │ pub extern "C" fn hello() -> u64 { ... }                   │
  └────────────────────────────────────────────────────────────┘
                              │
              cargo build --target wasm32-unknown-unknown
                              ▼
   target/.../release/hello.wasm  +  custom section "inject_meta"
                              │
                       cargo run -p builder
                              ▼
   out/hello-wasm-mod.jar
     ├─ fabric.mod.json
     ├─ hello-wasm-mod.mixins.json
     ├─ com/example/mixin/hello_hello_Mixin.class   ← crustf 生成 bytecode
     ├─ assets/hello-wasm-mod/hello.wasm
     └─ org/wasmer/**                                ← wasmer-jni 平坦化
```

1. `#[inject]` proc macro が `(fn_name, target, method, at)` を wasm の
   custom section `inject_meta` に書き込む。
2. host 側の `builder` が [`cargo_metadata`] で `wasm-payloads` の bin
   を列挙し、[`wasmparser`] で各 wasm の `inject_meta` を読み、
   [`crustf`] でレコードごとに parameterize された Mixin class を生成。
3. ランタイムでは Mixin class が同梱の `.wasm` リソースをロードし、
   wasmer-java で instantiate し、export 関数を呼んで linear memory から
   文字列を取り出して `System.out.println` する。

## 必要な物

- Rust 1.95+
- `wasm32-unknown-unknown` ターゲット
  (`rustup target add wasm32-unknown-unknown`)
- [wasmer-java releases][wj-releases] から `wasmer-jni-{platform}-0.3.0.jar`
  を入手して `vendor/` に配置。amd64 Linux / macOS / Windows 対応。
  入手できた数だけ置いておけば、builder が merge して 1 つの mod jar で
  全 platform 対応の jar を出力する。
- 実機検証には Minecraft + [Fabric Loader] 0.15+ on Minecraft 1.20+。

[wj-releases]: https://github.com/wasmerio/wasmer-java/releases
[Fabric Loader]: https://fabricmc.net/

## ビルド

```
cargo run -p builder
```

`out/hello-wasm-mod.jar` が生成されるので `<minecraft>/mods/` に投入。
サーバー起動時に `MinecraftServer#runServer` の `@At("HEAD")` に注入さ
れた Mixin が走り、ログに `Hello from WASM!` が出る。

## フックの追加

1. `crates/wasm-payloads/src/bin/<name>.rs` を作る。
2. `#[inject(...)]` を export 関数に付ける (1 ファイル内に複数可)。
3. `cargo run -p builder` が自動で検出し、新しい Mixin class が jar に
   追加される。

```rust
#![cfg_attr(target_arch = "wasm32", no_main, no_std)]

#[cfg(not(target_arch = "wasm32"))]
compile_error!("wasm-payloads must be built with `--target wasm32-unknown-unknown`");

#[cfg(not(target_arch = "wasm32"))]
fn main() {}

const MSG: &str = "another hook!";

#[inject_macro::inject(
    target = "Lnet/minecraft/server/MinecraftServer;",
    method = "loadWorld",
    at = "TAIL"
)]
pub extern "C" fn run() -> u64 {
    let p = MSG.as_ptr() as u32 as u64;
    let l = MSG.len() as u32 as u64;
    (p << 32) | l
}

#[cfg(target_arch = "wasm32")]
#[panic_handler]
fn ph(_: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}
```

## `#[inject]` の仕様

| フィールド | 必須                        | 意味                                                         |
|------------|-----------------------------|--------------------------------------------------------------|
| `target`   | はい                        | 注入先クラスの JVM internal-form descriptor                  |
| `method`   | はい                        | 注入先メソッド名                                             |
| `at`       | いいえ — default は `HEAD`  | Mixin `@At` の注入位置: `HEAD`, `TAIL`, `RETURN` 等          |

対象関数は **必ず** `pub extern "C"` で、現在は **必ず** シグネチャ
`() -> u64` で `(ptr << 32) | len` (linear memory 内の UTF-8 文字列)
を返さなければならない。ホスト側は `ByteBuffer` 経由で `ptr` から
`len` バイトを読み、`System.out.println` で出力する。

## ワークスペース構成

```
crates/
├── inject-macro/      proc-macro: #[inject(target, method, at)]
├── wasm-payloads/     wasm32-unknown-unknown 専用の bin 群
└── builder/           crustf で mod jar を生成する host bin
```

`wasm-payloads` は workspace の `default-members` から除外されており、
非 wasm32 ターゲット時に `compile_error!` で拒否する仕掛けが入っている
ので、ワークスペースルートで `cargo build` してもホスト crate のみ
ビルドされる。builder 側が必要なときに自身で
`cargo build -p wasm-payloads --target wasm32-unknown-unknown` を呼ぶ。

## 制限事項

- シグネチャは `pub extern "C" fn() -> u64` で `(ptr, len)` パック
  された UTF-8 文字列を返す形のみ対応 (v1)。
- Mixin の `compatibilityLevel: JAVA_8` (class file version 52)。生成
  される `onRun` は分岐なしなので `StackMapTable` 不要。
- wasmer-java 0.3.0 は upstream で開発停止中、amd64 のみサポート。
  ARM (Apple Silicon、ARM Linux) は対象外。
- mod jar には同梱した全 platform の native lib (各 5–10MB 程度) が
  入るため、対応 platform 数に比例してサイズが増える。

## ライセンス

未定。MIT / Apache-2.0 dual を推奨。

[`crustf`]: https://github.com/topi-banana/crustf
[`cargo_metadata`]: https://crates.io/crates/cargo_metadata
[`wasmparser`]: https://crates.io/crates/wasmparser
