plugins { id("com.android.application") }
android {
 namespace = "app.wispnet.discoveryexample"
 compileSdk = 36
 defaultConfig { applicationId = "app.wispnet.discoveryexample"; minSdk = 34; targetSdk = 36; versionCode = 1; versionName = "1.0" }
 compileOptions { sourceCompatibility = JavaVersion.VERSION_17; targetCompatibility = JavaVersion.VERSION_17 }
}
