plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
}

android {
    namespace = "app.inari.client"
    compileSdk = 35
    // Pinned to what the build image carries (ghcr.io/cirruslabs/android-sdk:35):
    // AGP 8.7 would otherwise ask for 34.0.0 and try to download it.
    buildToolsVersion = "35.0.0"

    defaultConfig {
        applicationId = System.getenv("INARI_APPLICATION_ID") ?: "app.inari.client"
        minSdk = 29
        targetSdk = 35
        versionCode = 79
        versionName = "0.40.1"
        // Where Inari is and where newer APKs are published: set by the machine that builds
        // (android/build.sh reads ~/.config/inari-android/build.env), empty in the repository.
        buildConfigField("String", "INARI_DEFAULT_URL", "\"${System.getenv("INARI_DEFAULT_URL") ?: ""}\"")
        // Names an installation already uses (its saved settings, its keystore entry): set them so an
        // update keeps them. The defaults are neutral.
        buildConfigField("String", "PREFS_NAME", "\"${System.getenv("INARI_PREFS_NAME") ?: "inari"}\"")
        buildConfigField("String", "GATE_KEY_ALIAS", "\"${System.getenv("INARI_GATE_KEY_ALIAS") ?: "inari-gate"}\"")
        buildConfigField("String", "INARI_UPDATES_URL", "\"${System.getenv("INARI_UPDATES_URL") ?: ""}\"")
    }

    // Release signing: the keystore and its password come from the machine
    // that builds (android/build.sh release mounts them), never from the
    // repository. Without them, a release build is simply left unsigned.
    val keystore = System.getenv("INARI_KEYSTORE")
    signingConfigs {
        if (keystore != null) {
            create("release") {
                val pass = file(System.getenv("INARI_KEYSTORE_PASS_FILE")!!).readText().trim()
                storeFile = file(keystore)
                storeType = "PKCS12"
                storePassword = pass
                keyAlias = System.getenv("INARI_KEY_ALIAS") ?: "inari"
                keyPassword = pass
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            signingConfig = signingConfigs.findByName("release")
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
    buildFeatures {
        compose = true
        buildConfig = true
    }
    androidResources {
        // The fox's mesh is already meshopt-compressed: stored as-is, it streams straight from the APK.
        noCompress += "glb"
    }
    testOptions {
        // org.json is an Android API: in JVM unit tests it must answer, not throw.
        unitTests.isReturnDefaultValues = true
    }
}

dependencies {
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.runtime.ktx)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.ui)
    implementation(libs.androidx.material3)
    implementation(libs.androidx.material.icons)
    // The fingerprint that unlocks the gate's password.
    implementation(libs.androidx.biometric)
    implementation(libs.androidx.fragment)
    // Kitsune's place reminders ring through the system's geofencing.
    implementation(libs.play.services.location)
    testImplementation(libs.junit)
    testImplementation("org.json:json:20240303")
}
