#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_biplabs_wispnet_Native_call(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    input: jni::objects::JString,
) -> jni::sys::jstring {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let input: zeroize::Zeroizing<String> =
            zeroize::Zeroizing::new(env.get_string(&input).map(|s| s.into()).unwrap_or_default());
        wispnet_client::call(&input)
    }))
    .unwrap_or_else(|_| "{\"ok\":false,\"error\":\"Native operation failed\"}".into());
    env.new_string(result)
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}
