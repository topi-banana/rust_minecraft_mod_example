use crustf::CodeBuilder;

use super::{MixinClass, MixinMethod, NativeMethod};
use crate::{NATIVE_LOADER_INTERNAL, NATIVE_PAYLOADS_OWNER};

pub struct MinecraftServerMixin;

const NATIVES: &[NativeMethod] = &[
    NativeMethod {
        name: "hello",
        descriptor: "()Ljava/lang/String;",
    },
    NativeMethod {
        name: "goodbye",
        descriptor: "()Ljava/lang/String;",
    },
];

const METHODS: &[MixinMethod] = &[
    MixinMethod {
        name: "onRun",
        descriptor: "(Lorg/spongepowered/asm/mixin/injection/callback/CallbackInfo;)V",
        target_method: "runServer",
        at: "HEAD",
        exceptions: &["java/io/IOException"],
        code: emit_on_run,
    },
    MixinMethod {
        name: "onRunReturn",
        descriptor: "(Lorg/spongepowered/asm/mixin/injection/callback/CallbackInfo;)V",
        target_method: "runServer",
        at: "RETURN",
        exceptions: &["java/io/IOException"],
        code: emit_on_run_return,
    },
];

fn emit_print_via_native(owner: &dyn MixinClass, c: &mut CodeBuilder, native_fn: &str) {
    c.max_stack(2);
    c.invokestatic(
        NATIVE_LOADER_INTERNAL,
        &format!("ensure_{}", owner.native_lib_name()),
        "()V",
    )
    .invokestatic(NATIVE_PAYLOADS_OWNER, native_fn, "()Ljava/lang/String;")
    .astore(2)
    .getstatic("java/lang/System", "out", "Ljava/io/PrintStream;")
    .aload(2)
    .invokevirtual("java/io/PrintStream", "println", "(Ljava/lang/String;)V")
    .return_void();
}

fn emit_on_run(owner: &dyn MixinClass, c: &mut CodeBuilder) {
    emit_print_via_native(owner, c, "hello");
}

fn emit_on_run_return(owner: &dyn MixinClass, c: &mut CodeBuilder) {
    emit_print_via_native(owner, c, "goodbye");
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
