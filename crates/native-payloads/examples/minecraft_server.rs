#[inject_macro::inject]
fn hello() {
    api::println("Hello from native!");
}

#[inject_macro::inject]
fn goodbye() {
    api::println("Goodbye from native!");
}
