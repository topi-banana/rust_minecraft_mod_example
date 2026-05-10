use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Expr, ExprLit, ItemFn, Lit, MetaNameValue, Token, parse_macro_input};

const FIELD_SEP: u8 = 0x1f; // ASCII US
const RECORD_SEP: u8 = 0x1e; // ASCII RS

struct InjectArgs {
    target: String,
    method: String,
    at: String,
}

impl Parse for InjectArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let kvs = Punctuated::<MetaNameValue, Token![,]>::parse_terminated(input)?;
        let mut target = None;
        let mut method = None;
        let mut at = None;
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

#[proc_macro_attribute]
pub fn inject(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as InjectArgs);
    let func = parse_macro_input!(input as ItemFn);

    if !matches!(func.vis, syn::Visibility::Public(_)) {
        return syn::Error::new_spanned(&func.sig.ident, "#[inject] functions must be `pub`")
            .to_compile_error()
            .into();
    }
    let abi_ok = func
        .sig
        .abi
        .as_ref()
        .and_then(|a| a.name.as_ref())
        .map(|n| n.value() == "C")
        .unwrap_or(false);
    if !abi_ok {
        return syn::Error::new_spanned(&func.sig, "#[inject] functions must be `extern \"C\"`")
            .to_compile_error()
            .into();
    }

    let fn_name = func.sig.ident.to_string();

    let mut bytes: Vec<u8> = Vec::with_capacity(
        fn_name.len() + args.target.len() + args.method.len() + args.at.len() + 4,
    );
    bytes.extend_from_slice(fn_name.as_bytes());
    bytes.push(FIELD_SEP);
    bytes.extend_from_slice(args.target.as_bytes());
    bytes.push(FIELD_SEP);
    bytes.extend_from_slice(args.method.as_bytes());
    bytes.push(FIELD_SEP);
    bytes.extend_from_slice(args.at.as_bytes());
    bytes.push(RECORD_SEP);

    let n = bytes.len();
    let byte_lits = bytes.iter().map(|b| quote!(#b));
    let static_ident = format_ident!("__INJECT_META_{}", fn_name.to_uppercase());

    quote! {
        #[unsafe(no_mangle)]
        #func

        #[used]
        #[unsafe(link_section = "inject_meta")]
        static #static_ident: [u8; #n] = [ #(#byte_lits),* ];
    }
    .into()
}
