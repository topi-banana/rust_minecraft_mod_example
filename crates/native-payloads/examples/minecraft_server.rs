#[inject_macro::inject]
fn hello() -> &'static str {
    "Hello from native!"
}
