plugins {
    id("com.android.application")
}

android {
    namespace = "dev.utter.app"
    compileSdk = 36
    defaultConfig {
        applicationId = "dev.utter.app"
        minSdk = 31
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.7"
        // The phone (arm64) and the emulator (x86_64): native/build.sh builds both.
        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
    }
    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }
    buildFeatures { buildConfig = true }
    // The core loads onnxruntime and LiteRT by path, and starts llama-server as a process: all
    // three must be files in the native library folder.
    packaging { jniLibs { useLegacyPackaging = true } }
}

dependencies {
    // Their native libraries only (libonnxruntime.so; libLiteRt.so and the GPU accelerator):
    // the Rust core calls them, the Java APIs are unused.
    implementation("com.microsoft.onnxruntime:onnxruntime-android:1.31.0")
    implementation("com.google.ai.edge.litert:litert:2.3.0")
    implementation("com.google.ai.edge.litert:litert-gpu:2.3.0")
}
