#[inject_macro::inject]
fn hello() -> &'static str {
    "Hello from native!"
}

#[inject_macro::inject]
fn goodbye() -> &'static str {
    "Goodbye from native!"
}
