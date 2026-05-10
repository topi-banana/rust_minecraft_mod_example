#![cfg_attr(target_arch = "wasm32", no_main, no_std)]

#[cfg(not(target_arch = "wasm32"))]
fn main() {}

#[cfg(target_arch = "wasm32")]
const GREETING: &str = "Hello from WASM!";

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn greet() -> u64 {
    let p = GREETING.as_ptr() as u32 as u64;
    let l = GREETING.len() as u32 as u64;
    (p << 32) | l
}

#[cfg(target_arch = "wasm32")]
#[panic_handler]
fn ph(_: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}
