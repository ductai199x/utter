plugins {
    // AGP 9 compiles Kotlin itself.
    id("com.android.application") version "9.4.1" apply false
    // LiteRT 2.3's classes carry Kotlin 2.4 metadata; this puts that compiler on the build
    // classpath in place of AGP's default.
    id("org.jetbrains.kotlin.android") version "2.4.0" apply false
}
