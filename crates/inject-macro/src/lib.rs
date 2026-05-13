use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Expr, ExprLit, ItemFn, Lit, MetaNameValue, Token, parse_macro_input};

const FIELD_SEP: u8 = 0x1f; // ASCII US
const RECORD_SEP: u8 = 0x1e; // ASCII RS

// JNI 関数名の owner prefix (`package_class` を JNI 規約で `_` 連結したもの)。
// Mixin クラスに直接 native メソッドを置くと Mixin プロセッサがターゲット
// クラスへマージしてしまい JNI 静的バインディングが破綻するので、 別途
// `com.example.runtime.NativePayloads` という holder クラスに集約する。
// builder 側 `NATIVE_PAYLOADS_OWNER` ("com/example/runtime/NativePayloads")
// と必ず同期させること。
const JNI_NATIVE_OWNER: &str = "com_example_runtime_NativePayloads";

struct InjectArgs {
    target: String,
    method: String,
    at: String,
    class: String,
}

impl Parse for InjectArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let kvs = Punctuated::<MetaNameValue, Token![,]>::parse_terminated(input)?;
        let mut target = None;
        let mut method = None;
        let mut at = None;
        let mut class = None;
        for nv in kvs {
            let key = nv
                .path
                .get_ident()
                .ok_or_else(|| syn::Error::new_spanned(&nv.path, "expected identifier"))?
                .to_string();
            let val = lit_str(&nv.value)?;
            match key.as_str() {
                "target" => target = Some(val),
                "method" => method = Some(val),
                "at" => at = Some(val),
                "class" => class = Some(val),
                other => {
                    return Err(syn::Error::new_spanned(
                        &nv.path,
                        format!("unknown #[inject] argument `{other}`"),
                    ));
                }
            }
        }
        Ok(InjectArgs {
            target: target.ok_or_else(|| {
                syn::Error::new(input.span(), "missing required `target = \"...\"`")
            })?,
            method: method.ok_or_else(|| {
                syn::Error::new(input.span(), "missing required `method = \"...\"`")
            })?,
            at: at.unwrap_or_else(|| "HEAD".into()),
            class: class.ok_or_else(|| {
                syn::Error::new(
                    input.span(),
                    "missing required `class = \"...\"` (Mixin class simple name, e.g. \"hello_Mixin\")",
                )
            })?,
        })
    }
}

fn lit_str(e: &Expr) -> syn::Result<String> {
    if let Expr::Lit(ExprLit {
        lit: Lit::Str(s), ..
    }) = e
    {
        Ok(s.value())
    } else {
        Err(syn::Error::new_spanned(e, "expected string literal"))
    }
}

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
    let args = parse_macro_input!(args as InjectArgs);
    let func = parse_macro_input!(input as ItemFn);

    let fn_name = func.sig.ident.to_string();

    // メタデータレコード: fn_name | target | method | at | class
    let mut bytes: Vec<u8> = Vec::with_capacity(
        fn_name.len()
            + args.target.len()
            + args.method.len()
            + args.at.len()
            + args.class.len()
            + 5,
    );
    bytes.extend_from_slice(fn_name.as_bytes());
    bytes.push(FIELD_SEP);
    bytes.extend_from_slice(args.target.as_bytes());
    bytes.push(FIELD_SEP);
    bytes.extend_from_slice(args.method.as_bytes());
    bytes.push(FIELD_SEP);
    bytes.extend_from_slice(args.at.as_bytes());
    bytes.push(FIELD_SEP);
    bytes.extend_from_slice(args.class.as_bytes());
    bytes.push(RECORD_SEP);

    let n = bytes.len();
    let byte_lits = bytes.iter().map(|b| quote!(#b));
    let static_ident = format_ident!("__INJECT_META_{}", fn_name.to_uppercase());

    let inner_ident = format_ident!("__inject_impl_{}", func.sig.ident);
    let _class_escaped = jni_escape(&args.class); // class はメタデータのみで使用
    let method_escaped = jni_escape(&fn_name);
    let jni_ident = format_ident!("Java_{}_{}", JNI_NATIVE_OWNER, method_escaped);

    let block = &func.block;
    let inputs = &func.sig.inputs;
    let output = &func.sig.output;

    quote! {
        #[inline]
        fn #inner_ident(#inputs) #output #block

        #[unsafe(no_mangle)]
        pub extern "system" fn #jni_ident(
            env: ::jni::JNIEnv,
            _cls: ::jni::objects::JClass,
        ) -> ::jni::sys::jstring {
            let s: &str = #inner_ident();
            let mut env = env;
            env.new_string(s)
                .map(|j| j.into_raw())
                .unwrap_or(::core::ptr::null_mut())
        }

        #[used]
        #[cfg_attr(target_os = "linux",   unsafe(link_section = ".inject_meta"))]
        #[cfg_attr(target_os = "macos",   unsafe(link_section = "__DATA,__injmeta"))]
        #[cfg_attr(target_os = "windows", unsafe(link_section = ".injmta"))]
        static #static_ident: [u8; #n] = [ #(#byte_lits),* ];
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
