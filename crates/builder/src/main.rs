use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use crustf::{
    AccessFlags, Annotation, ClassFileBuilder, CodeBuilder, ElementValue, FieldBuilder, JarBuilder,
    MethodBuilder, Version,
};
use serde::Serialize;

mod mixins;
use mixins::{MinecraftServerMixin, MixinClass};

pub const MIXIN_PACKAGE: &str = "com.example.mixin";
pub const MIXIN_PACKAGE_INTERNAL: &str = "com/example/mixin";
pub const NATIVE_LOADER_INTERNAL: &str = "com/example/runtime/NativeLoader";
// JNI 静的バインディングの対象 holder クラス。Mixin に native メソッドを
// 置くと Mixin プロセッサがターゲットクラスにマージしてしまうため (結果
// として `Java_net_minecraft_server_MinecraftServer_<fn>` を JVM が探して
// UnsatisfiedLinkError)、 別の通常 Java クラスに集約する。inject-macro 側
// `JNI_NATIVE_OWNER` ("com_example_runtime_NativePayloads") と必ず同期。
pub const NATIVE_PAYLOADS_OWNER: &str = "com/example/runtime/NativePayloads";
const MOD_ID: &str = "hello-native-mod";
const NATIVES_PACKAGE: &str = "native-payloads";
const NATIVE_LIB_DIRS_ENV: &str = "NATIVE_LIB_DIRS";

pub const MIXIN_ANNOTATION: &str = "Lorg/spongepowered/asm/mixin/Mixin;";
pub const INJECT_ANNOTATION: &str = "Lorg/spongepowered/asm/mixin/injection/Inject;";
pub const AT_ANNOTATION: &str = "Lorg/spongepowered/asm/mixin/injection/At;";

const MIXINS: &[&dyn MixinClass] = &[&MinecraftServerMixin];

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

struct NativeLib {
    platform: String,
    lib_filename: String,
    bytes: Vec<u8>,
}

fn main() -> Result<()> {
    let metadata = cargo_metadata::MetadataCommand::new()
        .exec()
        .context("failed to invoke cargo metadata")?;
    let ws_root: PathBuf = metadata.workspace_root.clone().into();
    let out_dir = ws_root.join("out");

    let lib_names: Vec<&'static str> = {
        let mut v: Vec<&'static str> = MIXINS.iter().map(|m| m.native_lib_name()).collect();
        v.sort();
        v.dedup();
        v
    };

    let native_libs: Vec<NativeLib> = match env::var(NATIVE_LIB_DIRS_ENV) {
        Ok(env_value) => {
            println!("→ aggregate mode: {NATIVE_LIB_DIRS_ENV}={env_value}");
            aggregate_libs_from_env(&env_value)?
        }
        Err(_) => {
            let host = host_platform_key();
            println!("→ local mode (host: {host})");
            run_cargo_build(&ws_root)?;
            discover_native_libs(&ws_root, &host, &lib_names)?
        }
    };
    println!("→ {} native lib(s):", native_libs.len());
    for nl in &native_libs {
        println!(
            "    {} ({}, {} bytes)",
            nl.lib_filename,
            nl.platform,
            nl.bytes.len()
        );
    }

    let mut class_files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut all_class_simples: Vec<String> = Vec::new();
    for m in MIXINS {
        let (internal, bytes) = build_mixin_class_for(*m)?;
        all_class_simples.push(m.mixin_class_simple_name().to_string());
        class_files.push((format!("{internal}.class"), bytes));
    }
    println!("→ generated {} Mixin class(es)", class_files.len());

    let loader_bytes = build_native_loader_class(&lib_names)?;
    println!(
        "→ generated NativeLoader ({} bytes, {} lib(s))",
        loader_bytes.len(),
        lib_names.len()
    );

    let payloads_bytes = build_native_payloads_class(MIXINS)?;
    println!(
        "→ generated NativePayloads holder ({} bytes)",
        payloads_bytes.len()
    );

    let fabric_json = serde_json::to_string_pretty(&fabric_mod_descriptor())?;
    let mixin_json = serde_json::to_string_pretty(&mixin_config(&all_class_simples))?;

    fs::create_dir_all(&out_dir)?;
    let mut builder = JarBuilder::new()
        .file("fabric.mod.json", fabric_json)
        .file(format!("{MOD_ID}.mixins.json"), mixin_json)
        .file(format!("{NATIVE_LOADER_INTERNAL}.class"), loader_bytes)
        .file(format!("{NATIVE_PAYLOADS_OWNER}.class"), payloads_bytes);
    for (entry, bytes) in class_files {
        builder = builder.file(entry, bytes);
    }
    for nl in native_libs {
        builder = builder.file(
            format!("native/{}/{}", nl.platform, nl.lib_filename),
            nl.bytes,
        );
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

fn host_platform_key() -> String {
    format!("{}-{}", env::consts::OS, env::consts::ARCH)
}

fn run_cargo_build(ws_root: &Path) -> Result<()> {
    println!("→ cargo build -p {NATIVES_PACKAGE} --release --examples");
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let status = Command::new(&cargo)
        .args(["build", "-p", NATIVES_PACKAGE, "--release", "--examples"])
        .current_dir(ws_root)
        .status()
        .context("failed to spawn cargo")?;
    if !status.success() {
        bail!("{NATIVES_PACKAGE} build failed (exit {status})");
    }
    Ok(())
}

fn discover_native_libs(
    ws_root: &Path,
    platform: &str,
    lib_names: &[&str],
) -> Result<Vec<NativeLib>> {
    let dir = ws_root.join("target/release/examples");
    let prefix = env::consts::DLL_PREFIX;
    let suffix = env::consts::DLL_SUFFIX;
    let mut out = Vec::new();
    for name in lib_names {
        let lib_filename = format!("{prefix}{name}{suffix}");
        let path = dir.join(&lib_filename);
        if !path.exists() {
            bail!(
                "expected {} after cargo build, but it does not exist",
                path.display()
            );
        }
        let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        out.push(NativeLib {
            platform: platform.to_string(),
            lib_filename,
            bytes,
        });
    }
    Ok(out)
}

/// Parse `NATIVE_LIB_DIRS=linux-x86_64=path1,windows-x86_64=path2,...`
/// and load every native lib file from each platform's directory.
fn aggregate_libs_from_env(env_value: &str) -> Result<Vec<NativeLib>> {
    let mut out = Vec::new();
    for entry in env_value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let (platform, dir) = entry.split_once('=').ok_or_else(|| {
            anyhow!("malformed {NATIVE_LIB_DIRS_ENV} entry `{entry}`, expected `platform=path`")
        })?;
        let platform = platform.trim();
        let dir = PathBuf::from(dir.trim());
        let mut found = false;
        for f in fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
            let f = f?;
            let path = f.path();
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if !looks_like_native_lib(name) {
                continue;
            }
            let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
            out.push(NativeLib {
                platform: platform.to_string(),
                lib_filename: name.to_string(),
                bytes,
            });
            found = true;
        }
        if !found {
            bail!(
                "no native lib (*.so / *.dll / *.dylib) found in {} for platform {}",
                dir.display(),
                platform
            );
        }
    }
    if out.is_empty() {
        bail!("{NATIVE_LIB_DIRS_ENV} produced no libs");
    }
    Ok(out)
}

fn looks_like_native_lib(name: &str) -> bool {
    name.ends_with(".so") || name.ends_with(".dll") || name.ends_with(".dylib")
}

fn fabric_mod_descriptor() -> FabricMod {
    FabricMod {
        schema_version: 1,
        id: MOD_ID.into(),
        version: "0.1.0".into(),
        name: "Hello Native Mod".into(),
        description: "Calls native payloads via JNI to print strings at server start.".into(),
        environment: "*".into(),
        license: "MIT".into(),
        mixins: vec![format!("{MOD_ID}.mixins.json")],
        depends: BTreeMap::from([
            ("fabricloader".into(), ">=0.15.0".into()),
            ("minecraft".into(), ">=1.20".into()),
        ]),
    }
}

fn mixin_config(class_simples: &[String]) -> MixinConfig {
    MixinConfig {
        required: true,
        package: MIXIN_PACKAGE.into(),
        compatibility_level: "JAVA_8".into(),
        mixins: class_simples.to_vec(),
        injectors: MixinInjectors { default_require: 1 },
    }
}

fn build_mixin_class_for(m: &dyn MixinClass) -> crustf::Result<(String, Vec<u8>)> {
    let class_internal = format!("{MIXIN_PACKAGE_INTERNAL}/{}", m.mixin_class_simple_name());
    let mut cb = ClassFileBuilder::new(&class_internal)
        .version(Version::new(52, 0))
        .annotation(Annotation::invisible(MIXIN_ANNOTATION).element(
            "value",
            ElementValue::Array(vec![ElementValue::Class(m.target_class_descriptor())]),
        ))
        .method(
            MethodBuilder::new("<init>", "()V")
                .access_flags(AccessFlags::PUBLIC)
                .code(|c| {
                    c.max_stack(1)
                        .aload(0)
                        .invokespecial("java/lang/Object", "<init>", "()V")
                        .return_void();
                }),
        );
    for mm in m.methods() {
        let at_anno = Annotation::visible(AT_ANNOTATION)
            .element("value", ElementValue::String(mm.at.to_string()));
        let mut inject_anno = Annotation::visible(INJECT_ANNOTATION)
            .element("method", ElementValue::String(mm.target_method.to_string()))
            .element(
                "at",
                ElementValue::Array(vec![ElementValue::from(at_anno)]),
            );
        if mm.cancellable {
            inject_anno = inject_anno.element("cancellable", ElementValue::Boolean(true));
        }
        let mut mb = MethodBuilder::new(mm.name, mm.descriptor)
            .access_flags(AccessFlags::PRIVATE)
            .annotation(inject_anno);
        for ex in mm.exceptions {
            mb = mb.exception(*ex);
        }
        let code_fn = mm.code;
        mb = mb.code(|c| code_fn(m, c));
        cb = cb.method(mb);
    }
    Ok((class_internal, cb.build()?))
}

/// Holder class for all native methods. Mixin classes themselves cannot host
/// `native` declarations because the Mixin processor merges them into the
/// target class (then JVM looks up `Java_<target>_<fn>` and crashes with
/// `UnsatisfiedLinkError`). A plain class outside the `com.example.mixin`
/// package is invisible to the Mixin processor, so symbols stay bound to
/// `Java_com_example_runtime_NativePayloads_<fn>` as the JNI spec expects.
fn build_native_payloads_class(mixins: &[&dyn MixinClass]) -> crustf::Result<Vec<u8>> {
    let mut builder = ClassFileBuilder::new(NATIVE_PAYLOADS_OWNER).method(
        MethodBuilder::new("<init>", "()V")
            .access_flags(AccessFlags::PRIVATE)
            .code(|c| {
                c.max_stack(1)
                    .aload(0)
                    .invokespecial("java/lang/Object", "<init>", "()V")
                    .return_void();
            }),
    );
    let mut seen: std::collections::BTreeSet<(&str, &str)> = std::collections::BTreeSet::new();
    for m in mixins {
        for n in m.native_methods() {
            if seen.insert((n.name, n.descriptor)) {
                builder = builder.method(
                    MethodBuilder::new(n.name, n.descriptor).access_flags(
                        AccessFlags::PUBLIC | AccessFlags::STATIC | AccessFlags::NATIVE,
                    ),
                );
            }
        }
    }
    builder.build()
}

fn build_native_loader_class(lib_names: &[&str]) -> crustf::Result<Vec<u8>> {
    // 明示的に `.version` を呼ばない → crustf default の JAVA_5 (49) になり、
    // ifeq/ifne/goto を含む分岐コードでも StackMapTable を出力する必要がない。
    let mut b = ClassFileBuilder::new(NATIVE_LOADER_INTERNAL)
        .method(
            MethodBuilder::new("<init>", "()V")
                .access_flags(AccessFlags::PRIVATE)
                .code(|c| {
                    c.max_stack(1)
                        .aload(0)
                        .invokespecial("java/lang/Object", "<init>", "()V")
                        .return_void();
                }),
        )
        .method(
            MethodBuilder::new("resourcePath", "(Ljava/lang/String;)Ljava/lang/String;")
                .access_flags(AccessFlags::PUBLIC | AccessFlags::STATIC)
                .code(emit_resource_path),
        );
    for name in lib_names {
        let flag = format!("loaded_{name}");
        let ensure = format!("ensure_{name}");
        let lib = name.to_string();
        let flag_for_method = flag.clone();
        b = b
            .field(
                FieldBuilder::new(&flag, "Z")
                    .access_flags(AccessFlags::PRIVATE | AccessFlags::STATIC),
            )
            .method(
                MethodBuilder::new(&ensure, "()V")
                    .access_flags(
                        AccessFlags::PUBLIC | AccessFlags::STATIC | AccessFlags::SYNCHRONIZED,
                    )
                    .exception("java/io/IOException")
                    .code(move |c| emit_ensure_for_lib(c, &lib, &flag_for_method)),
            );
    }
    b.build()
}

/// Bytecode for:
/// ```java
/// static String resourcePath(String libBasename) {
///     String osName = System.getProperty("os.name").toLowerCase();
///     String raw    = System.getProperty("os.arch");
///     String arch;
///     if      ("amd64".equals(raw)) arch = "x86_64";
///     else if ("arm64".equals(raw)) arch = "aarch64";
///     else                          arch = raw;
///     String os;
///     if      (osName.startsWith("windows")) os = "windows";
///     else if (osName.startsWith("mac"))     os = "macos";
///     else                                   os = "linux";
///     String libName = System.mapLibraryName(libBasename);
///     return "/native/" + os + "-" + arch + "/" + libName;
/// }
/// ```
/// local 0 = libBasename (引数), 1 = osName, 2 = arch, 3 = os, 4 = libName。
fn emit_resource_path(c: &mut CodeBuilder) {
    let l_not_amd64 = c.label();
    let l_not_arm64 = c.label();
    let l_arch_done = c.label();
    let l_not_win = c.label();
    let l_not_mac = c.label();
    let l_compose = c.label();

    c.max_stack(3);

    // osName = System.getProperty("os.name").toLowerCase();
    c.ldc_string("os.name")
        .invokestatic(
            "java/lang/System",
            "getProperty",
            "(Ljava/lang/String;)Ljava/lang/String;",
        )
        .invokevirtual("java/lang/String", "toLowerCase", "()Ljava/lang/String;")
        .astore(1);

    // raw = System.getProperty("os.arch");
    c.ldc_string("os.arch").invokestatic(
        "java/lang/System",
        "getProperty",
        "(Ljava/lang/String;)Ljava/lang/String;",
    );
    // stack: raw

    // if ("amd64".equals(raw)) { arch = "x86_64"; goto done; }
    c.dup()
        .ldc_string("amd64")
        .invokevirtual("java/lang/String", "equals", "(Ljava/lang/Object;)Z")
        .ifeq(l_not_amd64)
        .pop()
        .ldc_string("x86_64")
        .astore(2)
        .goto(l_arch_done);

    c.place(l_not_amd64);
    // stack: raw — else if ("arm64".equals(raw)) { arch = "aarch64"; goto done; }
    c.dup()
        .ldc_string("arm64")
        .invokevirtual("java/lang/String", "equals", "(Ljava/lang/Object;)Z")
        .ifeq(l_not_arm64)
        .pop()
        .ldc_string("aarch64")
        .astore(2)
        .goto(l_arch_done);

    c.place(l_not_arm64);
    // stack: raw — else { arch = raw; }
    c.astore(2);

    c.place(l_arch_done);

    // if (osName.startsWith("windows")) { os = "windows"; goto compose; }
    c.aload(1)
        .ldc_string("windows")
        .invokevirtual("java/lang/String", "startsWith", "(Ljava/lang/String;)Z")
        .ifeq(l_not_win)
        .ldc_string("windows")
        .astore(3)
        .goto(l_compose);

    c.place(l_not_win);
    // else if (osName.startsWith("mac")) { os = "macos"; goto compose; }
    c.aload(1)
        .ldc_string("mac")
        .invokevirtual("java/lang/String", "startsWith", "(Ljava/lang/String;)Z")
        .ifeq(l_not_mac)
        .ldc_string("macos")
        .astore(3)
        .goto(l_compose);

    c.place(l_not_mac);
    c.ldc_string("linux").astore(3);

    c.place(l_compose);

    // libName = System.mapLibraryName(libBasename);
    c.aload(0)
        .invokestatic(
            "java/lang/System",
            "mapLibraryName",
            "(Ljava/lang/String;)Ljava/lang/String;",
        )
        .astore(4);

    // return "/native/" + os + "-" + arch + "/" + libName;
    c.new_class("java/lang/StringBuilder")
        .dup()
        .invokespecial("java/lang/StringBuilder", "<init>", "()V")
        .ldc_string("/native/")
        .invokevirtual(
            "java/lang/StringBuilder",
            "append",
            "(Ljava/lang/String;)Ljava/lang/StringBuilder;",
        )
        .aload(3)
        .invokevirtual(
            "java/lang/StringBuilder",
            "append",
            "(Ljava/lang/String;)Ljava/lang/StringBuilder;",
        )
        .ldc_string("-")
        .invokevirtual(
            "java/lang/StringBuilder",
            "append",
            "(Ljava/lang/String;)Ljava/lang/StringBuilder;",
        )
        .aload(2)
        .invokevirtual(
            "java/lang/StringBuilder",
            "append",
            "(Ljava/lang/String;)Ljava/lang/StringBuilder;",
        )
        .ldc_string("/")
        .invokevirtual(
            "java/lang/StringBuilder",
            "append",
            "(Ljava/lang/String;)Ljava/lang/StringBuilder;",
        )
        .aload(4)
        .invokevirtual(
            "java/lang/StringBuilder",
            "append",
            "(Ljava/lang/String;)Ljava/lang/StringBuilder;",
        )
        .invokevirtual(
            "java/lang/StringBuilder",
            "toString",
            "()Ljava/lang/String;",
        )
        .areturn();
}

/// Bytecode for:
/// ```java
/// public static synchronized void ensure_<lib>() throws IOException {
///     if (loaded_<lib>) return;
///     String path = resourcePath("<lib>");
///     File tmp = File.createTempFile("<lib>_", "_" + System.mapLibraryName("<lib>"));
///     tmp.deleteOnExit();
///     InputStream is = NativeLoader.class.getResourceAsStream(path);
///     if (is == null) throw new IOException("native lib not found: " + path);
///     Files.copy(is, tmp.toPath(), StandardCopyOption.REPLACE_EXISTING);
///     is.close();
///     System.load(tmp.getAbsolutePath());
///     loaded_<lib> = true;
/// }
/// ```
fn emit_ensure_for_lib(c: &mut CodeBuilder, lib_name: &str, flag_name: &str) {
    let l_not_loaded = c.label();
    let l_is_ok = c.label();

    c.max_stack(8);

    // if (loaded_<lib>) return;
    c.getstatic(NATIVE_LOADER_INTERNAL, flag_name, "Z")
        .ifeq(l_not_loaded)
        .return_void();
    c.place(l_not_loaded);

    // String path = resourcePath("<lib>");
    c.ldc_string(lib_name)
        .invokestatic(
            NATIVE_LOADER_INTERNAL,
            "resourcePath",
            "(Ljava/lang/String;)Ljava/lang/String;",
        )
        .astore(0);

    // File tmp = File.createTempFile("<lib>_", "_" + System.mapLibraryName("<lib>"));
    let temp_prefix = format!("{lib_name}_");
    c.ldc_string(&temp_prefix)
        .new_class("java/lang/StringBuilder")
        .dup()
        .invokespecial("java/lang/StringBuilder", "<init>", "()V")
        .ldc_string("_")
        .invokevirtual(
            "java/lang/StringBuilder",
            "append",
            "(Ljava/lang/String;)Ljava/lang/StringBuilder;",
        )
        .ldc_string(lib_name)
        .invokestatic(
            "java/lang/System",
            "mapLibraryName",
            "(Ljava/lang/String;)Ljava/lang/String;",
        )
        .invokevirtual(
            "java/lang/StringBuilder",
            "append",
            "(Ljava/lang/String;)Ljava/lang/StringBuilder;",
        )
        .invokevirtual(
            "java/lang/StringBuilder",
            "toString",
            "()Ljava/lang/String;",
        )
        .invokestatic(
            "java/io/File",
            "createTempFile",
            "(Ljava/lang/String;Ljava/lang/String;)Ljava/io/File;",
        )
        .astore(1);

    // tmp.deleteOnExit();
    c.aload(1)
        .invokevirtual("java/io/File", "deleteOnExit", "()V");

    // InputStream is = NativeLoader.class.getResourceAsStream(path);
    c.ldc_class(NATIVE_LOADER_INTERNAL)
        .aload(0)
        .invokevirtual(
            "java/lang/Class",
            "getResourceAsStream",
            "(Ljava/lang/String;)Ljava/io/InputStream;",
        )
        .astore(2);

    // if (is == null) throw new IOException("native lib not found: " + path);
    c.aload(2).ifnonnull(l_is_ok);
    c.new_class("java/io/IOException")
        .dup()
        .new_class("java/lang/StringBuilder")
        .dup()
        .invokespecial("java/lang/StringBuilder", "<init>", "()V")
        .ldc_string("native lib not found: ")
        .invokevirtual(
            "java/lang/StringBuilder",
            "append",
            "(Ljava/lang/String;)Ljava/lang/StringBuilder;",
        )
        .aload(0)
        .invokevirtual(
            "java/lang/StringBuilder",
            "append",
            "(Ljava/lang/String;)Ljava/lang/StringBuilder;",
        )
        .invokevirtual(
            "java/lang/StringBuilder",
            "toString",
            "()Ljava/lang/String;",
        )
        .invokespecial("java/io/IOException", "<init>", "(Ljava/lang/String;)V")
        .athrow();
    c.place(l_is_ok);

    // Files.copy(is, tmp.toPath(), new CopyOption[]{ StandardCopyOption.REPLACE_EXISTING });
    c.aload(2)
        .aload(1)
        .invokevirtual("java/io/File", "toPath", "()Ljava/nio/file/Path;")
        .iconst_1()
        .anewarray("java/nio/file/CopyOption")
        .dup()
        .iconst_0()
        .getstatic(
            "java/nio/file/StandardCopyOption",
            "REPLACE_EXISTING",
            "Ljava/nio/file/StandardCopyOption;",
        )
        .aastore()
        .invokestatic(
            "java/nio/file/Files",
            "copy",
            "(Ljava/io/InputStream;Ljava/nio/file/Path;[Ljava/nio/file/CopyOption;)J",
        )
        .pop2();

    // is.close();
    c.aload(2)
        .invokevirtual("java/io/InputStream", "close", "()V");

    // System.load(tmp.getAbsolutePath());
    c.aload(1)
        .invokevirtual("java/io/File", "getAbsolutePath", "()Ljava/lang/String;")
        .invokestatic("java/lang/System", "load", "(Ljava/lang/String;)V");

    // loaded_<lib> = true;
    c.iconst_1()
        .putstatic(NATIVE_LOADER_INTERNAL, flag_name, "Z")
        .return_void();
}
