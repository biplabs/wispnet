#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_biplabs_wispnet_Native_call(
    mut env: jni::JNIEnv,
    class: jni::objects::JClass,
    input: jni::objects::JString,
) -> jni::sys::jstring {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let input: zeroize::Zeroizing<String> =
            zeroize::Zeroizing::new(env.get_string(&input).map(|s| s.into()).unwrap_or_default());
        let publisher = env.get_java_vm().and_then(|vm| {
            env.new_global_ref(&class)
                .map(|class| AndroidPublisher { vm, class })
        });
        match publisher {
            Ok(publisher) => {
                wispnet_client::call_with_publisher(&input, Some(std::sync::Arc::new(publisher)))
            }
            Err(_) => "{\"ok\":false,\"error\":\"Android storage unavailable\"}".into(),
        }
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

#[cfg(target_os = "android")]
struct AndroidPublisher {
    vm: jni::JavaVM,
    class: jni::objects::GlobalRef,
}
#[cfg(target_os = "android")]
static PUBLICATIONS: std::sync::LazyLock<
    std::sync::Mutex<
        std::collections::BTreeMap<String, std::sync::Arc<std::sync::atomic::AtomicBool>>,
    >,
> = std::sync::LazyLock::new(Default::default);
#[cfg(target_os = "android")]
impl wispnet_files::Publisher for AndroidPublisher {
    fn publish(
        &self,
        publication: wispnet_files::Publication<'_>,
        source: std::fs::File,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> std::io::Result<()> {
        use jni::objects::{JClass, JObject, JString, JValue};
        use std::{io, os::fd::AsRawFd};
        let unavailable = |_| io::Error::other("Android destination unavailable");
        let id = publication.transfer_id;
        PUBLICATIONS
            .lock()
            .map_err(|_| io::Error::other("Publication unavailable"))?
            .insert(id.to_owned(), cancel);
        struct Active<'a>(&'a str);
        impl Drop for Active<'_> {
            fn drop(&mut self) {
                if let Ok(mut active) = PUBLICATIONS.lock() {
                    active.remove(self.0);
                }
            }
        }
        let _active = Active(id);
        let mut env = self.vm.attach_current_thread().map_err(unavailable)?;
        let request =
            serde_json::json!({"transfer":id,"root":publication.root,"path":publication.path,
            "size_bytes":publication.size_bytes,"sha256":publication.sha256})
            .to_string();
        let text = env.new_string(request).map_err(unavailable)?;
        let class: &JClass = self.class.as_obj().into();
        let outcome = env.call_static_method(
            class,
            "publishReceivedFile",
            "(Ljava/lang/String;I)Ljava/lang/String;",
            &[
                JValue::Object(&JObject::from(text)),
                JValue::Int(source.as_raw_fd()),
            ],
        );
        let result = match outcome {
            Ok(value) => value.l().map_err(unavailable)?,
            Err(error) => {
                let _ = env.exception_clear();
                return Err(unavailable(error));
            }
        };
        let result = JString::from(result);
        let text: String = env.get_string(&result).map_err(unavailable)?.into();
        match text.as_str() {
            "ok" => Ok(()),
            "conflict" => Err(io::Error::from(io::ErrorKind::AlreadyExists)),
            "denied" => Err(io::Error::from(io::ErrorKind::PermissionDenied)),
            _ => Err(io::Error::other("Cannot save received file")),
        }
    }
}
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_biplabs_wispnet_Native_publicationCancelled(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    id: jni::objects::JString,
) -> jni::sys::jboolean {
    let cancelled = (|| {
        let id: String = env.get_string(&id).ok()?.into();
        Some(
            PUBLICATIONS
                .lock()
                .ok()?
                .get(&id)?
                .load(std::sync::atomic::Ordering::Acquire),
        )
    })()
    .unwrap_or(true);
    u8::from(cancelled)
}
