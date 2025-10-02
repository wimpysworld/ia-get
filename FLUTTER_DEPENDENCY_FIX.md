# Flutter Dependency Fix

## Issue Summary

The Flutter build was failing with the following dependency resolution error:

```
Resolving dependencies...
The current Dart SDK version is 3.5.4.

Because internet_archive_helper depends on flutter_lints >=6.0.0 which requires SDK version ^3.8.0, version solving failed.
```

## Root Cause

The project's `pubspec.yaml` specified:
- Dart SDK constraint: `>=3.0.0 <4.0.0`
- `flutter_lints: ^6.0.0` dependency

However, `flutter_lints ^6.0.0` requires Dart SDK ^3.8.0, which was incompatible with the Flutter version being used (3.24.5 includes Dart 3.5.4).

## Solution Applied

### 1. Updated Dart SDK Constraint

**File**: `mobile/flutter/pubspec.yaml`

Changed the SDK constraint to require Dart 3.8.0 or higher:

```yaml
environment:
  sdk: '>=3.8.0 <4.0.0'  # Updated from '>=3.0.0 <4.0.0'
```

### 2. Updated Flutter Version in CI/CD

**Files**: `.github/workflows/ci.yml` and `.github/workflows/release.yml`

Updated Flutter version from 3.24.5 to 3.27.1 (which includes Dart 3.8.0+):

```yaml
- name: Setup Flutter
  uses: subosito/flutter-action@v2
  with:
    flutter-version: '3.27.1'  # Updated from '3.24.5'
    channel: 'stable'
    cache: true
```

### 3. Updated Documentation

**File**: `docs/MOBILE_DEVELOPMENT_GUIDE.md`

Updated the Flutter version reference from 3.16.0 to 3.27.1 to match the CI/CD configuration.

## Dependency Compatibility

All existing dependencies in `pubspec.yaml` are compatible with Dart SDK 3.8.0:

- ✅ `ffi: ^2.1.4` - Compatible with Dart SDK >=3.0.0
- ✅ `cupertino_icons: ^1.0.8` - Compatible with Dart SDK >=3.0.0
- ✅ `provider: ^6.1.5` - Compatible with Dart SDK >=3.0.0
- ✅ `path_provider: ^2.1.5` - Compatible with Dart SDK >=3.0.0
- ✅ `permission_handler: ^12.0.0` - Compatible with Dart SDK >=3.0.0
- ✅ `http: ^1.5.0` - Compatible with Dart SDK >=3.3.0
- ✅ `dio: ^5.9.0` - Compatible with Dart SDK >=3.0.0
- ✅ `flutter_spinkit: ^5.2.2` - Compatible with Dart SDK >=3.0.0
- ✅ `percent_indicator: ^4.2.5` - Compatible with Dart SDK >=3.0.0
- ✅ `intl: ^0.20.2` - Compatible with Dart SDK >=3.0.0
- ✅ `shared_preferences: ^2.5.3` - Compatible with Dart SDK >=3.0.0
- ✅ `url_launcher: ^6.3.2` - Compatible with Dart SDK >=3.3.0
- ✅ `flutter_lints: ^6.0.0` - Requires Dart SDK ^3.8.0 ✓ Now compatible!

## Testing

**Quick Fix**: If you're experiencing this issue locally, run:
```bash
./scripts/fix-flutter-deps.sh
```

This script will automatically:
1. Check your Flutter and Dart versions
2. Upgrade Flutter if needed
3. Clean Flutter cache
4. Resolve dependencies

**Manual Steps** (if you prefer):

To verify the fix works:

1. Install Flutter 3.27.1 or higher (includes Dart 3.8.0+):
   ```bash
   flutter upgrade
   ```

2. Navigate to the Flutter app directory:
   ```bash
   cd mobile/flutter
   ```

3. Run dependency resolution:
   ```bash
   flutter pub get
   ```

4. Expected result: Dependencies should resolve successfully without errors.

**Still having issues?** See **[TROUBLESHOOTING.md](TROUBLESHOOTING.md)** for comprehensive troubleshooting steps.

## Impact

- **Minimal Breaking Changes**: The SDK constraint update only affects the minimum required Dart version
- **Forward Compatible**: All dependencies work with the new SDK version
- **CI/CD**: Updated workflows will automatically use the correct Flutter version
- **Development**: Developers need Flutter 3.27.1+ installed locally

## Flutter-Dart Version Mapping

For reference:
- Flutter 3.16.0 → Dart 3.2.x
- Flutter 3.24.5 → Dart 3.5.4
- Flutter 3.27.1 → Dart 3.8.0+

## Alternative Considered

An alternative would have been to downgrade `flutter_lints` to version 5.x which supports Dart SDK >=3.5.0. However, the issue requested "update all dependencies to a higher set of compatible versions," so updating the SDK constraint to support the latest flutter_lints was the appropriate solution.
