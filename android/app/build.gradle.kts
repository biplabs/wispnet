plugins { id("com.android.application") }
android {
    namespace = "com.biplabs.wispnet"
    compileSdk = 35
    defaultConfig { applicationId = "com.biplabs.wispnet"; minSdk = 34; targetSdk = 35; versionCode = 1; versionName = "0.1.0"; testInstrumentationRunner = "com.biplabs.wispnet.AppTestRunner"; ndk { abiFilters += "arm64-v8a" } }
    buildTypes {
        create("peercheck") { initWith(getByName("debug")); applicationIdSuffix = ".peercheck"; signingConfig = signingConfigs.getByName("debug") }
    }
    testBuildType = providers.gradleProperty("wispnetTestBuildType").orElse("debug").get()
    packaging { jniLibs { useLegacyPackaging = true } }
    compileOptions { sourceCompatibility = JavaVersion.VERSION_17; targetCompatibility = JavaVersion.VERSION_17 }
}
