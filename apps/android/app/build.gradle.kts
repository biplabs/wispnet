plugins { id("com.android.application") }
val releaseVersion = Regex("""VERSION = "([0-9]+\.[0-9]+\.[0-9]+)"""").find(rootProject.file("../node/desktop/wispnet_desktop/version.py").readText())!!.groupValues[1]
val releaseVersionCode = releaseVersion.split(".").map(String::toInt).let { it[0] * 1_000_000 + it[1] * 1_000 + it[2] }
val sourceIdentity = providers.exec { workingDir(rootProject.file("../..")); commandLine("python3", "scripts/build_identity.py") }.standardOutput.asText.get().trim()
android {
    namespace = "com.biplabs.wispnet"
    compileSdk = 36
    defaultConfig { applicationId = "app.wispnet.android"; minSdk = 34; targetSdk = 36; versionCode = providers.gradleProperty("wispnetVersionCode").orElse(releaseVersionCode.toString()).get().toInt(); versionName = providers.gradleProperty("wispnetVersionName").orElse(releaseVersion).get(); testInstrumentationRunner = "com.biplabs.wispnet.AppTestRunner"; ndk { abiFilters += providers.gradleProperty("wispnetAndroidAbi").orElse("arm64-v8a").get() } }
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
