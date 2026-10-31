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

/// Java transfers ownership with ParcelFileDescriptor.detachFd(). Construct the
/// owner before decoding the request, so invalid input and panics also close it.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_biplabs_wispnet_Native_putFileDescriptor(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    input: jni::objects::JString,
    descriptor: jni::sys::jint,
) -> jni::sys::jstring {
    use std::os::fd::FromRawFd;
    let result = if descriptor < 0 {
        "{\"ok\":false,\"error\":\"Invalid document descriptor\",\"code\":\"operation_failed\"}"
            .into()
    } else {
        // SAFETY: this entry point's contract accepts only detached descriptors
        // whose ownership Java relinquishes to this call exactly once.
        let file = unsafe { std::fs::File::from_raw_fd(descriptor) };
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let input: String = match env.get_string(&input) {
                Ok(value) => value.into(),
                Err(_) => return "{\"ok\":false,\"error\":\"Invalid document request\",\"code\":\"operation_failed\"}".into(),
            };
            wispnet_client::put_file_descriptor(&input, file)
        }))
        .unwrap_or_else(|_| "{\"ok\":false,\"error\":\"Native document transfer failed\",\"code\":\"operation_failed\"}".into())
    };
    env.new_string(result)
        .map(|value| value.into_raw())
        .unwrap_or(std::ptr::null_mut())
}
