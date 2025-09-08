# Add project specific ProGuard rules here.
# You can control the set of applied configuration files using the
# proguardFiles setting in build.gradle.

# Flutter ProGuard rules
-keep class io.flutter.app.** { *; }
-keep class io.flutter.plugin.**  { *; }
-keep class io.flutter.util.**  { *; }
-keep class io.flutter.view.**  { *; }
-keep class io.flutter.**  { *; }
-keep class io.flutter.plugins.**  { *; }

# FFI and native library rules
-keep class com.gameaday.ia_get_mobile.** { *; }
-keepclassmembers class * {
    native <methods>;
}

# Rust FFI symbols
-keep class * {
    native <methods>;
}

# Generic ProGuard rules for Android
-keepattributes *Annotation*
-keepattributes Signature
-keepattributes InnerClasses
-keepattributes EnclosingMethod

# Network and HTTP libraries
-dontwarn okhttp3.**
-dontwarn okio.**
-dontwarn javax.annotation.**
-dontwarn org.conscrypt.**

# Platform calls to Java APIs
-dontwarn android.support.**
-dontwarn androidx.**

# Keep native method names for FFI
-keepclasseswithmembers class * {
    native <methods>;
}

# Preserve line numbers for debugging crashes
-keepattributes SourceFile,LineNumberTable
-renamesourcefileattribute SourceFile