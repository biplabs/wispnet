plugins { id("com.android.application"); id("dev.flutter.flutter-gradle-plugin") }
val releaseVersion = Regex("""VERSION = "([0-9]+\.[0-9]+\.[0-9]+)"""").find(rootProject.file("../../node/desktop/wispnet_desktop/version.py").readText())!!.groupValues[1]
val sourceIdentity = providers.exec { workingDir(rootProject.file("../../..")); commandLine("python3", "scripts/build_identity.py") }.standardOutput.asText.get().trim()
val nativeAbis = providers.gradleProperty("target-platform").orElse("android-arm64").get().split(",").map {
    when (it) { "android-arm64" -> "arm64-v8a"; "android-x64" -> "x86_64"; else -> error("Supported Android targets: android-arm64, android-x64") }
}
val stageWispnetNative = tasks.register<Sync>("stageWispnetNative") {
    val nativeRoot = rootProject.file("../../android/app/src/main/jniLibs")
    from(nativeRoot) { nativeAbis.forEach { include("$it/libwispnet_android.so") } }
    into(layout.buildDirectory.dir("wispnet-jni"))
    doFirst {
        nativeAbis.forEach { require(nativeRoot.resolve("$it/libwispnet_android.so").isFile) { "Build the Rust library with scripts/build_android.sh first ($it)" } }
    }
}
android {
    namespace = "com.biplabs.wispnet"
    compileSdk = 36
    ndkVersion = flutter.ndkVersion
    sourceSets.getByName("main") {
        java.srcDir("../../../android/app/src/main/java")
        res.srcDir("../../../android/app/src/main/res")
        jniLibs.srcDir(layout.buildDirectory.dir("wispnet-jni").get().asFile)
    }
    defaultConfig { applicationId = "app.wispnet.android"; minSdk = 34; targetSdk = 36; versionCode = flutter.versionCode; versionName = flutter.versionName; testInstrumentationRunner = "com.biplabs.wispnet.AppTestRunner"; ndk { abiFilters += nativeAbis } }
    val uploadEnvironment = listOf("WISPNET_UPLOAD_KEYSTORE", "WISPNET_UPLOAD_STORE_PASSWORD", "WISPNET_UPLOAD_KEY_ALIAS", "WISPNET_UPLOAD_KEY_PASSWORD")
    val supplied = uploadEnvironment.map { providers.environmentVariable(it).orNull }
    require(supplied.all { it == null } || supplied.all { !it.isNullOrBlank() }) { "Supply all upload signing environment variables, or none for an unsigned build" }
    if (supplied.all { it != null }) {
        signingConfigs.create("upload") {
            storeFile = file(supplied[0]!!)
            storePassword = supplied[1]
            keyAlias = supplied[2]
            keyPassword = supplied[3]
        }
    }
    buildTypes {
        getByName("debug") { versionNameSuffix = sourceIdentity.removePrefix(releaseVersion) }
        getByName("release") {
            isDebuggable = false
            signingConfig = signingConfigs.findByName("upload")
        }
        create("peercheck") { initWith(getByName("debug")); applicationIdSuffix = ".peercheck"; signingConfig = signingConfigs.getByName("debug") }
    }
    testBuildType = providers.gradleProperty("wispnetTestBuildType").orElse("debug").get()
    packaging { jniLibs { useLegacyPackaging = false } }
    compileOptions { sourceCompatibility = JavaVersion.VERSION_17; targetCompatibility = JavaVersion.VERSION_17 }
}

// ZXing uses AndroidX at runtime but omits it from the published POM.
dependencies { implementation("androidx.core:core:1.17.0"); implementation("com.journeyapps:zxing-android-embedded:4.3.0"); implementation("com.google.zxing:core:3.4.1") }

flutter { source = "../.." }

tasks.named("preBuild") { dependsOn(stageWispnetNative) }
