use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use crustf::{
    AccessFlags, Annotation, ArrayType, ClassFileBuilder, CodeBuilder, ElementValue, JarBuilder,
    MethodBuilder, Version,
};
use serde::Serialize;

const CLASS_INTERNAL: &str = "com/example/mixin/HelloWasmMixin";
const MIXIN_PACKAGE: &str = "com.example.mixin";
const MOD_ID: &str = "hello-wasm-mod";
const WASM_JAR_ENTRY: &str = "assets/hello-wasm-mod/payload.wasm";
const WASM_RESOURCE_PATH: &str = "/assets/hello-wasm-mod/payload.wasm";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FabricMod {
    schema_version: u32,
    id: String,
    version: String,
    name: String,
    description: String,
    environment: String,
    license: String,
    mixins: Vec<String>,
    depends: BTreeMap<String, String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MixinConfig {
    required: bool,
    package: String,
    compatibility_level: String,
    mixins: Vec<String>,
    injectors: MixinInjectors,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MixinInjectors {
    default_require: u32,
}

fn main() -> Result<()> {
    let ws_root = workspace_root()?;
    let out_dir = ws_root.join("out");
    let vendor = ws_root.join("vendor");

    println!("→ cargo build -p wasm-payload --release --target wasm32-unknown-unknown");
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let status = Command::new(&cargo)
        .args([
            "build",
            "-p",
            "wasm-payload",
            "--release",
            "--target",
            "wasm32-unknown-unknown",
        ])
        .current_dir(&ws_root)
        .status()
        .context("failed to spawn cargo")?;
    if !status.success() {
        bail!("wasm-payload build failed (exit {status})");
    }

    let wasm_path = ws_root.join("target/wasm32-unknown-unknown/release/wasm-payload.wasm");
    let wasm_bytes =
        fs::read(&wasm_path).with_context(|| format!("reading {}", wasm_path.display()))?;
    println!("→ wasm payload: {} ({} bytes)", wasm_path.display(), wasm_bytes.len());

    let jars = find_all_wasmer_jnis(&vendor)?;
    println!(
        "→ vendored wasmer-jni jars ({} platform(s)):",
        jars.len()
    );
    for (platform, path) in &jars {
        println!("    {}: {}", platform, path.display());
    }
    let wasmer_entries = extract_wasmer_jni_entries(&jars)?;
    let native_paths: Vec<&str> = wasmer_entries
        .keys()
        .filter(|k| k.starts_with("org/wasmer/native/"))
        .map(String::as_str)
        .collect();
    println!(
        "    extracted {} entries ({} native libs)",
        wasmer_entries.len(),
        native_paths.len()
    );
    for p in &native_paths {
        println!("      {p}");
    }
    if native_paths.is_empty() {
        bail!(
            "no `org/wasmer/native/*` entries found in any vendored jar — \
             the runtime loader will fail with UnsatisfiedLinkError. Verify the \
             jars are real wasmer-jni distributions and not source-only artifacts."
        );
    }

    let class_bytes = build_mixin_class()?;
    let fabric_json = serde_json::to_string_pretty(&fabric_mod_descriptor())?;
    let mixin_json = serde_json::to_string_pretty(&mixin_config())?;

    fs::create_dir_all(&out_dir)?;
    let mut builder = JarBuilder::new()
        .file("fabric.mod.json", fabric_json)
        .file(format!("{MOD_ID}.mixins.json"), mixin_json)
        .file(format!("{CLASS_INTERNAL}.class"), class_bytes)
        .file(WASM_JAR_ENTRY, wasm_bytes);
    // Flatten wasmer-jni into the mod jar root (Fabric's JiJ would require an
    // inner fabric.mod.json — wasmer-jni is a plain library, so JiJ won't
    // expose its classes). KnotClassLoader sees `org/wasmer/*` directly.
    for (name, bytes) in wasmer_entries {
        builder = builder.file(name, bytes);
    }
    let jar_bytes = builder.build()?;
    let jar_path = out_dir.join(format!("{MOD_ID}.jar"));
    fs::write(&jar_path, &jar_bytes)?;
    println!(
        "→ wrote {} ({} bytes)\n  drop it in <minecraft>/mods/ next to a Fabric Loader install",
        jar_path.display(),
        jar_bytes.len()
    );

    Ok(())
}

fn workspace_root() -> Result<PathBuf> {
    let manifest = env!("CARGO_MANIFEST_DIR");
    Path::new(manifest)
        .ancestors()
        .nth(2)
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("cannot locate workspace root from {manifest}"))
}

fn find_all_wasmer_jnis(vendor: &Path) -> Result<Vec<(String, PathBuf)>> {
    let entries = fs::read_dir(vendor).with_context(|| {
        format!(
            "cannot open {}.\n\
             place one or more `wasmer-jni-{{platform}}-*.jar` (downloaded from\n\
             https://github.com/wasmerio/wasmer-java/releases) inside the vendor/ \
             directory",
            vendor.display()
        )
    })?;
    let mut found = Vec::new();
    for entry in entries.filter_map(std::result::Result::ok) {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        // wasmer-jni-{platform}-{version}.jar
        let Some(middle) = name
            .strip_prefix("wasmer-jni-")
            .and_then(|m| m.strip_suffix(".jar"))
        else {
            continue;
        };
        let Some(dash) = middle.rfind('-') else {
            continue;
        };
        let platform = &middle[..dash];
        if platform.is_empty() {
            continue;
        }
        found.push((platform.to_string(), path));
    }
    if found.is_empty() {
        bail!(
            "no `wasmer-jni-*.jar` found in {}.\n\
             download one or more platform-specific jars from\n\
             https://github.com/wasmerio/wasmer-java/releases",
            vendor.display()
        );
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(found)
}

/// Take the union of every entry across input jars (skipping
/// `META-INF/MANIFEST.MF`), first-write-wins on duplicates.
///
/// We deliberately do NOT filter by platform. wasmer-java's runtime loader
/// builds resource paths from `{os.name normalized}-{os.arch}` (e.g.
/// `windows-amd64`, `darwin-x86_64`), while jar filenames sometimes invert the
/// order (`amd64-windows`). Filtering by a filename-derived platform silently
/// drops the actual native lib.
fn extract_wasmer_jni_entries(jars: &[(String, PathBuf)]) -> Result<BTreeMap<String, Vec<u8>>> {
    use std::collections::btree_map::Entry;

    let mut all_files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for (_platform, jar_path) in jars {
        let file = fs::File::open(jar_path)
            .with_context(|| format!("opening {}", jar_path.display()))?;
        let mut archive = zip::ZipArchive::new(file)
            .with_context(|| format!("reading {} as zip", jar_path.display()))?;
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            if entry.is_dir() {
                continue;
            }
            let name = entry.name().to_string();
            if name == "META-INF/MANIFEST.MF" {
                continue;
            }
            if let Entry::Vacant(slot) = all_files.entry(name) {
                let mut bytes = Vec::with_capacity(entry.size() as usize);
                entry.read_to_end(&mut bytes)?;
                slot.insert(bytes);
            }
        }
    }
    Ok(all_files)
}

fn fabric_mod_descriptor() -> FabricMod {
    FabricMod {
        schema_version: 1,
        id: MOD_ID.into(),
        version: "0.1.0".into(),
        name: "Hello WASM Mod".into(),
        description: "Calls a wasm payload via wasmer-java to print a string at server start."
            .into(),
        environment: "*".into(),
        license: "MIT".into(),
        mixins: vec![format!("{MOD_ID}.mixins.json")],
        depends: BTreeMap::from([
            ("fabricloader".into(), ">=0.15.0".into()),
            ("minecraft".into(), ">=1.20".into()),
        ]),
    }
}

fn mixin_config() -> MixinConfig {
    MixinConfig {
        required: true,
        package: MIXIN_PACKAGE.into(),
        compatibility_level: "JAVA_8".into(),
        mixins: vec!["HelloWasmMixin".into()],
        injectors: MixinInjectors { default_require: 1 },
    }
}

fn build_mixin_class() -> crustf::Result<Vec<u8>> {
    ClassFileBuilder::new(CLASS_INTERNAL)
        .version(Version::new(52, 0))
        .annotation(
            Annotation::invisible("Lorg/spongepowered/asm/mixin/Mixin;").element(
                "value",
                ElementValue::Array(vec![ElementValue::Class(
                    "Lnet/minecraft/server/MinecraftServer;".into(),
                )]),
            ),
        )
        .method(
            MethodBuilder::new("<init>", "()V")
                .access_flags(AccessFlags::PUBLIC)
                .code(|c| {
                    c.max_stack(1)
                        .aload(0)
                        .invokespecial("java/lang/Object", "<init>", "()V")
                        .return_void();
                }),
        )
        .method(
            MethodBuilder::new(
                "onRun",
                "(Lorg/spongepowered/asm/mixin/injection/callback/CallbackInfo;)V",
            )
            .access_flags(AccessFlags::PRIVATE)
            .exception("java/io/IOException")
            .annotation(
                Annotation::visible("Lorg/spongepowered/asm/mixin/injection/Inject;")
                    .element("method", ElementValue::String("runServer".into()))
                    .element(
                        "at",
                        ElementValue::Array(vec![ElementValue::from(
                            Annotation::visible("Lorg/spongepowered/asm/mixin/injection/At;")
                                .element("value", ElementValue::String("HEAD".into())),
                        )]),
                    ),
            )
            .code(emit_on_run),
        )
        .build()
}

fn emit_on_run(c: &mut CodeBuilder) {
    // Peak stack depth in this method is 5 slots, reached at `dup2; bipush 32`
    // during the long-to-(int,int) split in Step 4. Set 6 for headroom.
    c.max_stack(6);

    // Step 1: byte[] wasmBytes = HelloWasmMixin.class
    //                                .getResourceAsStream("/assets/.../payload.wasm")
    //                                .readAllBytes();
    c.ldc_class(CLASS_INTERNAL)
        .ldc_string(WASM_RESOURCE_PATH)
        .invokevirtual(
            "java/lang/Class",
            "getResourceAsStream",
            "(Ljava/lang/String;)Ljava/io/InputStream;",
        )
        .invokevirtual("java/io/InputStream", "readAllBytes", "()[B")
        .astore(2);

    // Step 2: Instance instance = new Instance(wasmBytes);
    c.new_class("org/wasmer/Instance")
        .dup()
        .aload(2)
        .invokespecial("org/wasmer/Instance", "<init>", "([B)V")
        .astore(3);

    // Step 3: Exports exports = instance.exports;
    c.aload(3)
        .getfield("org/wasmer/Instance", "exports", "Lorg/wasmer/Exports;")
        .astore(4);

    // Step 4:
    //   Function greet = exports.getFunction("greet");
    //   long packed = ((Long) greet.apply(new Object[0])[0]).longValue();
    //   int ptr = (int)(packed >>> 32);
    //   int len = (int) packed;
    c.aload(4)
        .ldc_string("greet")
        .invokevirtual(
            "org/wasmer/Exports",
            "getFunction",
            "(Ljava/lang/String;)Lorg/wasmer/exports/Function;",
        )
        .iconst_0()
        .anewarray("java/lang/Object")
        .invokeinterface(
            "org/wasmer/exports/Function",
            "apply",
            "([Ljava/lang/Object;)[Ljava/lang/Object;",
        )
        .iconst_0()
        .aaload()
        .checkcast("java/lang/Long")
        .invokevirtual("java/lang/Long", "longValue", "()J")
        .dup2()
        .bipush(32)
        .lushr()
        .l2i()
        .istore(7)
        .l2i()
        .istore(8);

    // Step 5:
    //   ByteBuffer buf = exports.getMemory("memory").buffer();
    //   buf.position(ptr);
    //   byte[] outBytes = new byte[len];
    //   buf.get(outBytes);
    c.aload(4)
        .ldc_string("memory")
        .invokevirtual(
            "org/wasmer/Exports",
            "getMemory",
            "(Ljava/lang/String;)Lorg/wasmer/Memory;",
        )
        .invokevirtual("org/wasmer/Memory", "buffer", "()Ljava/nio/ByteBuffer;")
        .iload(7)
        .invokevirtual("java/nio/ByteBuffer", "position", "(I)Ljava/nio/Buffer;")
        .checkcast("java/nio/ByteBuffer")
        .iload(8)
        .newarray(ArrayType::Byte)
        .dup_x1()
        .invokevirtual(
            "java/nio/ByteBuffer",
            "get",
            "([B)Ljava/nio/ByteBuffer;",
        )
        .pop()
        .astore(9);

    // Step 6:
    //   System.out.println(new String(outBytes, StandardCharsets.UTF_8));
    c.new_class("java/lang/String")
        .dup()
        .aload(9)
        .getstatic(
            "java/nio/charset/StandardCharsets",
            "UTF_8",
            "Ljava/nio/charset/Charset;",
        )
        .invokespecial(
            "java/lang/String",
            "<init>",
            "([BLjava/nio/charset/Charset;)V",
        )
        .astore(9)
        .getstatic("java/lang/System", "out", "Ljava/io/PrintStream;")
        .aload(9)
        .invokevirtual("java/io/PrintStream", "println", "(Ljava/lang/String;)V")
        .return_void();
}
