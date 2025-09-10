# Add project specific ProGuard rules here.
# You can control the set of applied configuration files using the
# proguardFiles setting in build.gradle.

# Flutter ProGuard rules - Updated for latest Flutter
-keep class io.flutter.app.** { *; }
-keep class io.flutter.plugin.**  { *; }
-keep class io.flutter.util.**  { *; }
-keep class io.flutter.view.**  { *; }
-keep class io.flutter.**  { *; }
-keep class io.flutter.plugins.**  { *; }
-keep class io.flutter.embedding.**  { *; }

# FFI and native library rules
-keep class com.gameaday.ia_get_mobile.** { *; }
-keepclassmembers class * {
    native <methods>;
}

# Rust FFI symbols - Enhanced for better performance
-keep class * {
    native <methods>;
}
-keepclasseswithmembers class * {
    native <methods>;
}

# Generic ProGuard rules for Android - Play Store optimized
-keepattributes *Annotation*
-keepattributes Signature
-keepattributes InnerClasses
-keepattributes EnclosingMethod
-keepattributes RuntimeVisibleAnnotations
-keepattributes RuntimeInvisibleAnnotations
-keepattributes RuntimeVisibleParameterAnnotations
-keepattributes RuntimeInvisibleParameterAnnotations

# Network and HTTP libraries - Updated for modern libraries
-dontwarn okhttp3.**
-dontwarn okio.**
-dontwarn javax.annotation.**
-dontwarn org.conscrypt.**
-dontwarn retrofit2.**
-dontwarn com.squareup.okhttp3.**

# Platform calls to Java APIs
-dontwarn android.support.**
-dontwarn androidx.**

# Keep Activity and Service classes
-keep public class * extends android.app.Activity
-keep public class * extends android.app.Service
-keep public class * extends android.content.BroadcastReceiver
-keep public class * extends android.content.ContentProvider

# Preserve line numbers for debugging crashes (Play Console crash reports)
-keepattributes SourceFile,LineNumberTable
-renamesourcefileattribute SourceFile

# Keep BuildConfig for different flavors
-keep class **.BuildConfig { *; }

# AndroidX and Google Play Services
-keep class androidx.** { *; }
-keep interface androidx.** { *; }
-dontwarn androidx.**

# Material Design Components
-keep class com.google.android.material.** { *; }
-dontwarn com.google.android.material.**

# Kotlin Coroutines (if used)
-keepnames class kotlinx.coroutines.internal.MainDispatcherFactory {}
-keepnames class kotlinx.coroutines.CoroutineExceptionHandler {}
-keepclassmembernames class kotlinx.** {
    volatile <fields>;
}

# Keep custom exceptions for better crash reporting
-keep public class * extends java.lang.Exception

# R8 compatibility for Flutter
-keep class dart.** { *; }
-keep class org.dartlang.** { *; }

# Play Core for app updates and feature delivery
-keep class com.google.android.play.core.** { *; }
-dontwarn com.google.android.play.core.**

# Optimize for size while preserving functionality
-optimizations !code/simplification/arithmetic,!code/simplification/cast,!field/*,!class/merging/*
-optimizationpasses 5
-allowaccessmodification
-dontpreverify
-repackageclasses ''

# Keep enums
-keepclassmembers enum * {
    public static **[] values();
    public static ** valueOf(java.lang.String);
}

# Keep Parcelable classes
-keep class * implements android.os.Parcelable {
    public static final android.os.Parcelable$Creator *;
}

# Keep Serializable classes
-keepnames class * implements java.io.Serializable
-keepclassmembers class * implements java.io.Serializable {
    static final long serialVersionUID;
    private static final java.io.ObjectStreamField[] serialPersistentFields;
    !static !transient <fields>;
    private void writeObject(java.io.ObjectOutputStream);
    private void readObject(java.io.ObjectInputStream);
    java.lang.Object writeReplace();
    java.lang.Object readResolve();
}

# Google Tink Crypto Library Rules - Handle optional HTTP dependencies
-dontwarn com.google.api.client.http.**
-dontwarn com.google.api.client.util.**
-dontwarn com.google.api.client.googleapis.**
-dontwarn com.google.api.client.json.**
-dontwarn com.google.api.client.extensions.**

# Tink KeysDownloader is optional and not used in offline crypto operations  
-dontwarn com.google.crypto.tink.util.KeysDownloader
-dontwarn com.google.crypto.tink.integration.gcpkms.**
-dontwarn com.google.crypto.tink.integration.awskms.**

# Additional Google API Client classes referenced by Tink
-dontwarn com.google.api.client.http.GenericUrl
-dontwarn com.google.api.client.http.HttpHeaders
-dontwarn com.google.api.client.http.HttpRequest
-dontwarn com.google.api.client.http.HttpRequestFactory
-dontwarn com.google.api.client.http.HttpResponse
-dontwarn com.google.api.client.http.HttpTransport
-dontwarn com.google.api.client.http.javanet.NetHttpTransport$Builder

# Keep Tink core crypto functionality that we do use
-keep class com.google.crypto.tink.** { *; }
-keep class com.google.crypto.tink.proto.** { *; }

# Security-crypto specific rules
-keep class androidx.security.crypto.** { *; }
-dontwarn androidx.security.crypto.**