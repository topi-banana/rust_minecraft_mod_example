use crustf::CodeBuilder;

pub mod minecraft_server;

pub use minecraft_server::MinecraftServerMixin;

/// 対象メソッド引数の Java 側型。 descriptor 1 文字 (または `L...;` / `[...`)、
/// stack/local の slot size、対応する load opcode の 3 つをここから派生させる。
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)] // public な variant 集合: 個別 Mixin で必要なときに使う。
pub enum JavaType {
    Int,
    Long,
    Float,
    Double,
    Boolean,
    Byte,
    Short,
    Char,
    /// internal name, e.g. `"java/util/function/BooleanSupplier"`.
    Object(&'static str),
    /// 先頭 `'['` を除いた残りの descriptor。 例えば int 配列なら `"I"`、
    /// `String[]` なら `"Ljava/lang/String;"`、 二次元 int 配列なら `"[I"`。
    Array(&'static str),
}

impl JavaType {
    pub fn slot_size(&self) -> u16 {
        match self {
            JavaType::Long | JavaType::Double => 2,
            _ => 1,
        }
    }

    pub fn descriptor(&self) -> String {
        match self {
            JavaType::Int => "I".into(),
            JavaType::Long => "J".into(),
            JavaType::Float => "F".into(),
            JavaType::Double => "D".into(),
            JavaType::Boolean => "Z".into(),
            JavaType::Byte => "B".into(),
            JavaType::Short => "S".into(),
            JavaType::Char => "C".into(),
            JavaType::Object(name) => format!("L{name};"),
            JavaType::Array(inner) => format!("[{inner}"),
        }
    }

    /// `slot` 位置の値を operand stack に積む。
    pub fn emit_load(&self, c: &mut CodeBuilder, slot: u16) {
        match self {
            JavaType::Long => {
                c.lload(slot);
            }
            JavaType::Double => {
                c.dload(slot);
            }
            JavaType::Float => {
                c.fload(slot);
            }
            JavaType::Object(_) | JavaType::Array(_) => {
                c.aload(slot);
            }
            JavaType::Int
            | JavaType::Boolean
            | JavaType::Byte
            | JavaType::Short
            | JavaType::Char => {
                c.iload(slot);
            }
        }
    }
}

/// `com.example.runtime.NativePayloads` に置く 1 個の native static method。
pub struct NativeMethod {
    pub name: String,
    pub descriptor: String,
}

#[derive(Debug, Clone, Copy)]
pub enum MixinAt {
    Head,
    Return,
}

impl std::fmt::Display for MixinAt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MixinAt::Head => write!(f, "HEAD"),
            MixinAt::Return => write!(f, "RETURN"),
        }
    }
}

/// 生成 Mixin クラス側の 1 個の @Inject ハンドラ method。
pub struct MixinMethod {
    pub name: &'static str,
    pub target_method: &'static str,
    /// 対象メソッドの引数列。 これと `CallbackInfo` 末尾結合から handler / native
    /// 双方の descriptor が一意に決まる。
    pub target_args: &'static [JavaType],
    pub at: MixinAt,
    pub cancellable: bool,
    pub exceptions: &'static [&'static str],
    /// NativePayloads holder クラスに置く native static method の名前
    /// (= Rust 側 `#[inject]` 関数名)。
    pub native_name: &'static str,
    pub code: fn(&MixinMethod, &dyn MixinClass, &mut CodeBuilder),
}

impl MixinMethod {
    /// Mixin handler / native static method の両方で使う descriptor。
    /// 並び: target_args 群 + CallbackInfo。 return 型は void。
    pub fn descriptor(&self) -> String {
        let mut s = String::from("(");
        for t in self.target_args {
            s.push_str(&t.descriptor());
        }
        s.push_str("Lorg/spongepowered/asm/mixin/injection/callback/CallbackInfo;)V");
        s
    }
}

pub trait MixinClass: Sync {
    fn target_class(&self) -> &'static str;

    fn target_class_descriptor(&self) -> String {
        format!("L{};", self.target_class())
    }

    fn mixin_class_simple_name(&self) -> &'static str;

    /// 対応する cdylib の name (= `[[example]] name`)。
    /// builder は `target/release/examples/{prefix}<name>{suffix}` を期待する。
    fn native_lib_name(&self) -> &'static str;

    fn methods(&self) -> &'static [MixinMethod];

    /// `methods()` から native_name + descriptor を重複排除して一覧化。
    /// 同じ payload (例: `cancel_demo`) に対して複数の @Inject が向く構成でも
    /// `(name, descriptor)` ペアで dedupe するので NativePayloads には 1 つだけ
    /// native 宣言が出る。
    fn native_methods(&self) -> Vec<NativeMethod> {
        let mut seen: std::collections::BTreeSet<(String, String)> =
            std::collections::BTreeSet::new();
        let mut out = Vec::new();
        for m in self.methods() {
            let desc = m.descriptor();
            let key = (m.native_name.to_string(), desc.clone());
            if seen.insert(key) {
                out.push(NativeMethod {
                    name: m.native_name.to_string(),
                    descriptor: desc,
                });
            }
        }
        out
    }
}
