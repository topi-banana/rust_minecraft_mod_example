use crustf::CodeBuilder;

use super::{MixinAt, MixinClass, MixinMethod, NativeMethod};
use crate::{NATIVE_LOADER_INTERNAL, NATIVE_PAYLOADS_OWNER};

pub struct MinecraftServerMixin;

const NATIVE_DESC: &str = "(Lorg/spongepowered/asm/mixin/injection/callback/CallbackInfo;)V";

fn emit_call_native(owner: &dyn MixinClass, c: &mut CodeBuilder, native_fn: &str) {
    c.max_stack(2);
    c.invokestatic(
        NATIVE_LOADER_INTERNAL,
        &format!("ensure_{}", owner.native_lib_name()),
        "()V",
    )
    .aload(1)
    .invokestatic(NATIVE_PAYLOADS_OWNER, native_fn, NATIVE_DESC)
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
                descriptor: NATIVE_DESC,
            },
            NativeMethod {
                name: "goodbye",
                descriptor: NATIVE_DESC,
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
                cancellable: false,
                exceptions: &["java/io/IOException"],
                code: |owner, c| emit_call_native(owner, c, "hello"),
            },
            MixinMethod {
                name: "onRunReturn",
                descriptor: "(Lorg/spongepowered/asm/mixin/injection/callback/CallbackInfo;)V",
                target_method: "runServer",
                at: MixinAt::Return,
                cancellable: false,
                exceptions: &["java/io/IOException"],
                code: |owner, c| emit_call_native(owner, c, "goodbye"),
            },
        ]
    }
}
