#[inject_macro::inject]
fn hello() {
    api::println("Hello from native!").ok();
}

#[inject_macro::inject]
fn goodbye() {
    api::println("Goodbye from native!").ok();
}
