use crustf::CodeBuilder;

pub mod minecraft_server;

pub use minecraft_server::MinecraftServerMixin;

/// `com.example.runtime.NativePayloads` に置く 1 個の native static method。
pub struct NativeMethod {
    pub name: &'static str,
    pub descriptor: &'static str,
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
    pub descriptor: &'static str,
    pub target_method: &'static str,
    pub at: MixinAt,
    pub cancellable: bool,
    pub exceptions: &'static [&'static str],
    pub code: fn(&dyn MixinClass, &mut CodeBuilder),
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

    fn native_methods(&self) -> &'static [NativeMethod];

    fn methods(&self) -> &'static [MixinMethod];
}
