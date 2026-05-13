use crustf::CodeBuilder;

use super::{JavaType, MixinAt, MixinClass, MixinMethod};
use crate::{NATIVE_LOADER_INTERNAL, NATIVE_PAYLOADS_OWNER};

pub struct MinecraftServerMixin;

fn emit_call_native(
    owner: &dyn MixinClass,
    c: &mut CodeBuilder,
    native_fn: &str,
    target_args: &[JavaType],
) {
    c.invokestatic(
        NATIVE_LOADER_INTERNAL,
        &format!("ensure_{}", owner.native_lib_name()),
        "()V",
    );

    // slot 0 = this (instance handler なので除外)。
    // slot 1 から対象メソッド引数を順に load し、その直後の slot に CallbackInfo。
    let mut slot: u16 = 1;
    for t in target_args {
        t.emit_load(c, slot);
        slot += t.slot_size();
    }
    c.aload(slot);

    let mut desc = String::from("(");
    for t in target_args {
        desc.push_str(&t.descriptor());
    }
    desc.push_str("Lorg/spongepowered/asm/mixin/injection/callback/CallbackInfo;)V");
    c.invokestatic(NATIVE_PAYLOADS_OWNER, native_fn, &desc)
        .return_void();

    let stack_sum: u16 = target_args.iter().map(JavaType::slot_size).sum::<u16>() + 1;
    c.max_stack(stack_sum.max(2));
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
    fn methods(&self) -> &'static [MixinMethod] {
        &[
            MixinMethod {
                name: "onRun",
                target_method: "runServer",
                target_args: &[],
                at: MixinAt::Head,
                cancellable: false,
                exceptions: &["java/io/IOException"],
                native_name: "hello",
                code: |mm, owner, c| emit_call_native(owner, c, mm.native_name, mm.target_args),
            },
            MixinMethod {
                name: "onRunReturn",
                target_method: "runServer",
                target_args: &[],
                at: MixinAt::Return,
                cancellable: false,
                exceptions: &["java/io/IOException"],
                native_name: "goodbye",
                code: |mm, owner, c| emit_call_native(owner, c, mm.native_name, mm.target_args),
            },
            MixinMethod {
                name: "onRunCancel",
                target_method: "runServer",
                target_args: &[],
                at: MixinAt::Head,
                cancellable: true,
                exceptions: &["java/io/IOException"],
                native_name: "cancel_demo",
                code: |mm, owner, c| emit_call_native(owner, c, mm.native_name, mm.target_args),
            },
            MixinMethod {
                name: "onTickServer",
                target_method: "tickServer",
                target_args: &[JavaType::Object("java/util/function/BooleanSupplier")],
                at: MixinAt::Head,
                cancellable: false,
                exceptions: &[],
                native_name: "on_tick",
                code: |mm, owner, c| emit_call_native(owner, c, mm.native_name, mm.target_args),
            },
        ]
    }
}
