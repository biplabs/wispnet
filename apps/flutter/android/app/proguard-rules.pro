# Rust AndroidPublisher invokes this method through JNI GetStaticMethodID.
# Java reachability alone cannot preserve it; its name and signature are ABI.
-keepclassmembers class com.biplabs.wispnet.Native {
    static java.lang.String publishReceivedFile(java.lang.String, int);
}
