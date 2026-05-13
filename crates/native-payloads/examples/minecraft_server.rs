#[inject_macro::inject]
fn hello(_ci: api::CallbackInfo) {
    api::println("Hello from native!").ok();
}

#[inject_macro::inject]
fn goodbye(_ci: api::CallbackInfo) {
    api::println("Goodbye from native!").ok();
}

#[inject_macro::inject]
fn cancel_demo(ci: api::CallbackInfo) {
    api::println("cancelling runServer at HEAD").ok();
    ci.cancel().ok();
}
