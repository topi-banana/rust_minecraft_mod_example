use std::sync::atomic::{AtomicU64, Ordering};

use api::{CallbackInfo, println};
use jni::objects::JObject;

#[inject_macro::inject]
fn hello(_ci: CallbackInfo) {
    println("Hello from native!").ok();
}

#[inject_macro::inject]
fn goodbye(_ci: CallbackInfo) {
    println("Goodbye from native!").ok();
}

#[inject_macro::inject]
fn cancel_demo(ci: CallbackInfo) {
    println("cancelling runServer at HEAD").ok();
    ci.cancel().ok();
}

static TICK_COUNT: AtomicU64 = AtomicU64::new(0);

#[inject_macro::inject]
fn on_tick(_supplier: JObject, _ci: CallbackInfo) {
    let n = TICK_COUNT.fetch_add(1, Ordering::Relaxed);
    if n.is_multiple_of(100) {
        println(format!("tickServer called {n} times")).ok();
    }
}
