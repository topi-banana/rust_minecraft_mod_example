use api::{CallbackInfo, println};

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
