#[inject_macro::inject(
    target = "Lnet/minecraft/server/MinecraftServer;",
    method = "runServer",
    at = "HEAD",
    class = "hello_Mixin"
)]
fn hello() -> &'static str {
    "Hello from native!"
}
