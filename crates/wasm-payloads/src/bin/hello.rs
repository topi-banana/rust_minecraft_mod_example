#![cfg_attr(target_arch = "wasm32", no_main, no_std)]

#[cfg(not(target_arch = "wasm32"))]
fn main() {}

const HELLO: &str = "Hello from WASM!";

#[inject_macro::inject(
    target = "Lnet/minecraft/server/MinecraftServer;",
    method = "runServer",
    at = "HEAD"
)]
pub extern "C" fn hello() -> u64 {
    let p = HELLO.as_ptr() as u32 as u64;
    let l = HELLO.len() as u32 as u64;
    (p << 32) | l
}

#[cfg(target_arch = "wasm32")]
#[panic_handler]
fn ph(_: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable();
}
