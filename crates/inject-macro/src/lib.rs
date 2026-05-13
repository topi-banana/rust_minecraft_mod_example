use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::parse::Nothing;
use syn::{ItemFn, parse_macro_input};

// JNI 関数名の owner prefix (`package_class` を JNI 規約で `_` 連結したもの)。
// Mixin クラスに直接 native メソッドを置くと Mixin プロセッサがターゲット
// クラスへマージしてしまい JNI 静的バインディングが破綻するので、 別途
// `com.example.runtime.NativePayloads` という holder クラスに集約する。
// builder 側 `NATIVE_PAYLOADS_OWNER` ("com/example/runtime/NativePayloads")
// と必ず同期させること。
const JNI_NATIVE_OWNER: &str = "com_example_runtime_NativePayloads";

/// JNI shortened name escape per JNI Specification §13.2:
/// `_` → `_1`, `;` → `_2`, `[` → `_3`, `/` and `.` → `_`,
/// ASCII letter/digit pass through, other Unicode → `_0xxxx` (UTF-16 unit hex).
fn jni_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        match c {
            '_' => out.push_str("_1"),
            ';' => out.push_str("_2"),
            '[' => out.push_str("_3"),
            '/' | '.' => out.push('_'),
            c if c.is_ascii_alphanumeric() => out.push(c),
            c => {
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf).iter() {
                    out.push_str(&format!("_0{:04x}", *u));
                }
            }
        }
    }
    out
}

#[proc_macro_attribute]
pub fn inject(args: TokenStream, input: TokenStream) -> TokenStream {
    let _ = parse_macro_input!(args as Nothing);
    let func = parse_macro_input!(input as ItemFn);

    let fn_name = func.sig.ident.to_string();
    let inner_ident = format_ident!("__inject_impl_{}", func.sig.ident);
    let method_escaped = jni_escape(&fn_name);
    let jni_ident = format_ident!("Java_{}_{}", JNI_NATIVE_OWNER, method_escaped);

    let block = &func.block;

    quote! {
        #[inline]
        fn #inner_ident() #block

        #[unsafe(no_mangle)]
        pub extern "system" fn #jni_ident(
            env: ::jni::JNIEnv,
            _cls: ::jni::objects::JClass,
        ) {
            let mut env = env;
            // SAFETY: `env` はこの JNI 関数呼び出しの間だけ有効で、guard と
            // 同じスコープにあるので、guard より長生きしない。
            let _guard = unsafe { ::api::EnvGuard::enter(&mut env) };
            #inner_ident();
        }
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::jni_escape;

    #[test]
    fn jni_escape_passthrough() {
        assert_eq!(jni_escape("hello"), "hello");
        assert_eq!(jni_escape("Class123"), "Class123");
    }

    #[test]
    fn jni_escape_underscore_to_underscore_one() {
        assert_eq!(jni_escape("hello_world"), "hello_1world");
        assert_eq!(jni_escape("hello_Mixin"), "hello_1Mixin");
        assert_eq!(jni_escape("__"), "_1_1");
    }

    #[test]
    fn jni_escape_slash_and_dot_to_underscore() {
        assert_eq!(jni_escape("com/example"), "com_example");
        assert_eq!(jni_escape("com.example"), "com_example");
    }

    #[test]
    fn jni_escape_semicolon_and_bracket() {
        assert_eq!(jni_escape("Class;"), "Class_2");
        assert_eq!(jni_escape("[I"), "_3I");
    }

    #[test]
    fn jni_escape_unicode_bmp() {
        // U+3042 HIRAGANA LETTER A
        assert_eq!(jni_escape("あ"), "_03042");
    }
}
