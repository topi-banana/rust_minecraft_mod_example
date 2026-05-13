use crustf::CodeBuilder;

use super::{MixinClass, MixinMethod, NativeMethod};
use crate::{NATIVE_LOADER_INTERNAL, NATIVE_PAYLOADS_OWNER};

pub struct MinecraftServerMixin;

const NATIVES: &[NativeMethod] = &[NativeMethod {
    name: "hello",
    descriptor: "()Ljava/lang/String;",
}];

const METHODS: &[MixinMethod] = &[MixinMethod {
    name: "onRun",
    descriptor: "(Lorg/spongepowered/asm/mixin/injection/callback/CallbackInfo;)V",
    target_method: "runServer",
    at: "HEAD",
    exceptions: &["java/io/IOException"],
    code: emit_on_run,
}];

fn emit_on_run(owner: &dyn MixinClass, c: &mut CodeBuilder) {
    c.max_stack(2);
    c.invokestatic(
        NATIVE_LOADER_INTERNAL,
        &format!("ensure_{}", owner.native_lib_name()),
        "()V",
    )
    .invokestatic(NATIVE_PAYLOADS_OWNER, "hello", "()Ljava/lang/String;")
    .astore(2)
    .getstatic("java/lang/System", "out", "Ljava/io/PrintStream;")
    .aload(2)
    .invokevirtual("java/io/PrintStream", "println", "(Ljava/lang/String;)V")
    .return_void();
}

impl MixinClass for MinecraftServerMixin {
    fn target_class(&self) -> &'static str {
        "net/minecraft/server/MinecraftServer"
    }
    fn mixin_class_simple_name(&self) -> &'static str {
        "MinecraftServerMixin"
    }
    fn native_lib_name(&self) -> &'static str {
        "minecraft_server"
    }
    fn native_methods(&self) -> &'static [NativeMethod] {
        NATIVES
    }
    fn methods(&self) -> &'static [MixinMethod] {
        METHODS
    }
}
