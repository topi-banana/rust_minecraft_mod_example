use std::cell::Cell;
use std::fmt::Display;

use jni::JNIEnv;
use jni::objects::JValue;

pub use jni::errors::Error as JniError;
pub type Result<T = ()> = std::result::Result<T, JniError>;

thread_local! {
    static CURRENT_ENV: Cell<*mut jni::sys::JNIEnv> = const { Cell::new(std::ptr::null_mut()) };
}

/// `#[inject_macro::inject]` が生成する JNI wrapper の中でだけ作られる RAII guard。
/// Drop されるまで thread-local に現在の JNIEnv ポインタを保存し、
/// `api::println` 等のラッパー関数が引数なしで env を取得できるようにする。
pub struct EnvGuard {
    prev: *mut jni::sys::JNIEnv,
}

impl EnvGuard {
    /// # Safety
    /// `env` は呼び出し元 JNI 関数の lifetime 内でのみ有効。
    /// この guard を Drop するまで env への他の参照を作らないこと。
    pub unsafe fn enter(env: &mut JNIEnv) -> Self {
        let raw = env.get_raw();
        let prev = CURRENT_ENV.with(|c| c.replace(raw));
        EnvGuard { prev }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        CURRENT_ENV.with(|c| c.set(self.prev));
    }
}

fn with_env<R>(f: impl FnOnce(&mut JNIEnv) -> R) -> R {
    let raw = CURRENT_ENV.with(|c| c.get());
    assert!(
        !raw.is_null(),
        "api::* called outside of a #[inject] function"
    );
    // SAFETY: EnvGuard により、現スレッド上で active な JNI 関数の env が登録されている。
    let mut env = unsafe { JNIEnv::from_raw(raw).expect("invalid JNIEnv pointer") };
    f(&mut env)
}

/// `System.out.println(value.to_string())` を JNI 経由で呼ぶ。
pub fn println<T: Display>(value: T) -> Result<()> {
    let s = value.to_string();
    with_env(|env| -> Result<()> {
        let jstr = env.new_string(&s)?;
        let system_cls = env.find_class("java/lang/System")?;
        let out = env.get_static_field(&system_cls, "out", "Ljava/io/PrintStream;")?;
        let out_obj = out.l()?;
        env.call_method(
            &out_obj,
            "println",
            "(Ljava/lang/String;)V",
            &[JValue::from(&jstr)],
        )?;
        Ok(())
    })
}
