# Android Deployment Guide - Complete Walkthrough

This guide provides step-by-step instructions for generating APK files for testing and preparing the app for Google Play Store deployment.

## ✅ Current Status

The project is **fully configured** for Android deployment:
- ✅ Native Rust libraries built for all Android architectures (arm64-v8a, armeabi-v7a, x86_64, x86)
- ✅ Flutter mobile app structure complete
- ✅ Build scripts functional (`scripts/build-mobile.sh`)
- ✅ Android signing configuration with fallbacks
- ✅ Play Store documentation and assets prepared

## 🚀 Quick Start - Generate APK for Testing

### 1. Prerequisites

Install the required tools:

```bash
# Install Flutter (follow official guide for your OS)
# https://flutter.dev/docs/get-started/install

# Verify installation
flutter doctor

# Install Android SDK and accept licenses
flutter doctor --android-licenses
```

### 2. Generate Development APK

```bash
# Clone the repository (if not already done)
git clone https://github.com/Gameaday/ia-get-cli.git
cd ia-get-cli

# Generate development APK
./scripts/build-mobile.sh --development

# Output: target/mobile/ia-get-mobile-development.apk
```

### 3. Install and Test APK

```bash
# Install on connected Android device or emulator
adb install target/mobile/ia-get-mobile-development.apk

# Or manually: Copy APK to device and install via file manager
```

## 📱 APK Build Variants

The build system supports multiple variants:

```bash
# Development APK (debug, larger size, easier debugging)
./scripts/build-mobile.sh --development

# Staging APK (profile mode, testing environment)
./scripts/build-mobile.sh --staging

# Production APK (release mode, optimized, smaller size)
./scripts/build-mobile.sh --production

# Production App Bundle for Play Store (RECOMMENDED)
./scripts/build-mobile.sh --production --appbundle --store-ready
```

### Build Output Locations

- **APK Files**: `target/mobile/ia-get-mobile-{environment}.apk`
- **App Bundles**: `target/mobile/ia-get-mobile-{environment}.aab`
- **Native Libraries**: `target/mobile/android/*/libia_get.so`

## 🏪 Google Play Store Deployment

### Step 1: Set Up Signing Keys

#### Generate Upload Keystore (One-time setup)

```bash
# Generate upload keystore for Play Store submission
keytool -genkey -v -keystore upload-keystore.jks -keyalg RSA -keysize 2048 -validity 10000 -alias upload

# Follow prompts to enter:
# - Password (remember this!)
# - Your organization details
# - Key password (can be same as store password)
```

#### Configure Signing

Create `mobile/flutter/android/key.properties`:

```properties
uploadKeyStore=upload-keystore.jks
uploadKeyAlias=upload
uploadStorePassword=your-secure-password
uploadKeyPassword=your-secure-password
```

**⚠️ Important**: Keep your keystore file and passwords secure. Never commit them to version control.

### Step 2: Build App Bundle for Play Store

```bash
# Build production App Bundle
./scripts/build-mobile.sh --production --appbundle --store-ready

# Output: target/mobile/ia-get-mobile-production.aab
```

### Step 3: Google Play Console Setup

1. **Create Google Play Console Account**
   - Visit [Google Play Console](https://play.google.com/console)
   - Pay $25 one-time registration fee
   - Complete developer profile

2. **Create New App**
   - Click "Create app"
   - App name: "Internet Archive Helper"
   - Package name: `com.gameaday.internet_archive_helper`
   - Category: Tools

3. **Upload App Bundle**
   - Go to "Release" → "Production"
   - Click "Create new release"
   - Upload `target/mobile/ia-get-mobile-production.aab`
   - Add release notes (see template below)

### Step 4: Complete Store Listing

#### App Information
- **Short Description**: "Internet Archive Helper - Download books, movies, music & more"
- **Full Description**: Use template from `PLAY_STORE_GUIDE.md`
- **App Category**: Tools
- **Content Rating**: Complete IARC questionnaire
- **Target Audience**: All ages

#### Required Assets
- **App Icon**: 512×512 PNG (configured in Flutter app)
- **Feature Graphic**: 1024×500 PNG (create using app screenshots)
- **Screenshots**: Minimum 2, maximum 8 (phone screenshots)
- **Privacy Policy**: Link to `PRIVACY_POLICY.md` (must be publicly hosted)

### Step 5: Submit for Review

1. Complete all required sections in Play Console
2. Submit for review (typically takes 1-3 days)
3. Monitor review status and respond to any feedback

## 🛠️ Troubleshooting

### Build Issues

#### Flutter Not Found
```bash
# Install Flutter following official guide
# Add to PATH: export PATH="$PATH:[PATH_TO_FLUTTER]/flutter/bin"
flutter doctor
```

#### Android Licenses Not Accepted
```bash
flutter doctor --android-licenses
# Accept all licenses by typing 'y'
```

#### Signing Errors
```bash
# Check if keystore file exists and paths are correct
ls -la mobile/flutter/android/key.properties
ls -la mobile/flutter/android/*.jks

# The build will fall back to debug signing if production keys aren't available
```

#### Missing Android SDK
```bash
# Install Android Studio or standalone SDK tools
# Set ANDROID_SDK_ROOT environment variable
export ANDROID_SDK_ROOT=/path/to/android/sdk
```

### APK Installation Issues

#### Unknown Sources
- Enable "Install from unknown sources" in Android settings
- Or use ADB: `adb install -r your-app.apk`

#### Architecture Mismatch
- The APK includes libraries for all major Android architectures
- If issues persist, try rebuilding: `./scripts/build-mobile.sh --production --clean`

### Play Store Review Issues

#### Common Rejections
- **Privacy Policy**: Must be publicly accessible and complete
- **Permissions**: Justify all requested permissions in store description
- **Content Policy**: Ensure app complies with Play Store policies
- **Target API**: Make sure you're targeting the latest required Android API level

## 📋 Release Checklist

### Pre-Release
- [ ] Native libraries build successfully for all architectures
- [ ] APK installs and runs on test devices
- [ ] App Bundle generates without errors
- [ ] Signing keys are secure and backed up
- [ ] Privacy policy is publicly accessible

### Play Store Submission
- [ ] App Bundle uploaded to Play Console
- [ ] Store listing complete with descriptions and assets
- [ ] Content rating questionnaire completed
- [ ] Privacy policy linked and accessible
- [ ] All required screenshots uploaded
- [ ] Release notes written
- [ ] Target countries selected
- [ ] Pricing set (free)

### Post-Launch
- [ ] Monitor crash reports in Play Console
- [ ] Respond to user reviews
- [ ] Track download metrics
- [ ] Plan feature updates based on feedback

## 🔗 Resources

- [PLAY_STORE_GUIDE.md](./PLAY_STORE_GUIDE.md) - Detailed Play Store submission guide
- [PRIVACY_POLICY.md](./PRIVACY_POLICY.md) - App privacy policy
- [mobile/flutter/assets/APP_STORE_ASSETS.md](./mobile/flutter/assets/APP_STORE_ASSETS.md) - Store assets documentation
- [Google Play Console](https://play.google.com/console)
- [Android Developer Guide](https://developer.android.com/guide)
- [Flutter Documentation](https://flutter.dev/docs)

## 🆘 Need Help?

If you encounter issues:

1. Check the troubleshooting section above
2. Review the build logs for specific error messages
3. Consult the Flutter and Android documentation
4. Open an issue in the repository with detailed error information

The build system is designed to be robust with fallbacks, so most issues are related to environment setup rather than the code itself.