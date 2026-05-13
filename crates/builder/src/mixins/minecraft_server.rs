use crustf::CodeBuilder;

use super::{MixinAt, MixinClass, MixinMethod, NativeMethod};
use crate::{NATIVE_LOADER_INTERNAL, NATIVE_PAYLOADS_OWNER};

pub struct MinecraftServerMixin;

fn emit_call_native(owner: &dyn MixinClass, c: &mut CodeBuilder, native_fn: &str) {
    c.max_stack(1);
    c.invokestatic(
        NATIVE_LOADER_INTERNAL,
        &format!("ensure_{}", owner.native_lib_name()),
        "()V",
    )
    .invokestatic(NATIVE_PAYLOADS_OWNER, native_fn, "()V")
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
        &[
            NativeMethod {
                name: "hello",
                descriptor: "()V",
            },
            NativeMethod {
                name: "goodbye",
                descriptor: "()V",
            },
        ]
    }
    fn methods(&self) -> &'static [MixinMethod] {
        &[
            MixinMethod {
                name: "onRun",
                descriptor: "(Lorg/spongepowered/asm/mixin/injection/callback/CallbackInfo;)V",
                target_method: "runServer",
                at: MixinAt::Head,
                exceptions: &["java/io/IOException"],
                code: |owner, c| emit_call_native(owner, c, "hello"),
            },
            MixinMethod {
                name: "onRunReturn",
                descriptor: "(Lorg/spongepowered/asm/mixin/injection/callback/CallbackInfo;)V",
                target_method: "runServer",
                at: MixinAt::Return,
                exceptions: &["java/io/IOException"],
                code: |owner, c| emit_call_native(owner, c, "goodbye"),
            },
        ]
    }
}
