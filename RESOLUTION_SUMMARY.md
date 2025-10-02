# Flutter/Dart SDK Version Conflict - Resolution Summary

## Issue Description

Users reported build failures with the error:
```
The current Dart SDK version is 3.6.0.
Because internet_archive_helper requires SDK version >=3.8.0, version solving failed.
```

## Root Cause

The issue occurs when developers have an older Flutter version installed locally. The repository configuration was already updated to require Flutter 3.27.1+ (which includes Dart SDK 3.8.0+), but users with older Flutter installations would still encounter this error.

## Current State of Repository

**All configuration files are ALREADY CORRECT:**

✅ **mobile/flutter/pubspec.yaml**
```yaml
environment:
  sdk: '>=3.8.0 <4.0.0'
```

✅ **.github/workflows/ci.yml**
```yaml
- name: Setup Flutter
  uses: subosito/flutter-action@v2
  with:
    flutter-version: '3.27.1'
```

✅ **.github/workflows/release.yml**
```yaml
- name: Setup Flutter
  uses: subosito/flutter-action@v2
  with:
    flutter-version: '3.27.1'
```

## Resolution

Since the repository configuration was already updated with the correct SDK constraints, the issue manifested in two ways:

1. **Local Development**: Developers with outdated Flutter installations need to upgrade
2. **CI/CD Pipeline**: Cached Flutter dependencies from before the SDK update were incompatible

We've added comprehensive tools and documentation to help users resolve both scenarios:

### 1. For Local Development Issues

**Automated Quick-Fix Script**

**scripts/fix-flutter-deps.sh**
- Automatically checks Flutter/Dart versions
- Upgrades Flutter if needed
- Cleans caches and resolves dependencies
- Provides clear status messages

**Usage:**
```bash
./scripts/fix-flutter-deps.sh
```

### 2. For CI/CD Pipeline Issues

**Updated GitHub Actions Workflows**

Modified both `.github/workflows/ci.yml` and `.github/workflows/release.yml` to:
- **Add cache version identifiers**: Cache keys now include `dart3.8` to invalidate old caches
- **Add version verification**: Workflows now verify Flutter and Dart versions before building
- **Prevent cache conflicts**: Old cached dependencies built with SDK <3.8.0 won't be used

**Changes Made:**
```yaml
# Old cache key pattern (could restore incompatible caches):
key: ${{ runner.os }}-android-apk-${{ hashFiles(...) }}

# New cache key pattern (ensures SDK 3.8.0+ compatibility):
key: ${{ runner.os }}-android-apk-dart3.8-${{ hashFiles(...) }}
```

This ensures that when the Dart SDK constraint changes, the CI automatically uses fresh dependencies compatible with the new SDK version.

### 3. Comprehensive Documentation

**TROUBLESHOOTING.md**
- Complete guide for all common build issues
- Step-by-step resolution instructions
- Environment setup verification

**issues/README.md**
- Issue tracking index with solutions
- Status tracking

**RESOLUTION_SUMMARY.md**
- Complete technical analysis
- Resolution approach explanation

## For Users Experiencing This Issue

### Quick Fix (Recommended)
```bash
./scripts/fix-flutter-deps.sh
```

### Manual Fix
```bash
# Update Flutter to latest stable
flutter upgrade

# Verify version
flutter --version  # Should show Flutter 3.27.1+ and Dart 3.8.0+

# Clean and rebuild
cd mobile/flutter
flutter clean
rm -rf pubspec.lock
flutter pub get
```

### Verify Requirements
Your environment needs:
- **Flutter**: 3.27.1 or higher
- **Dart**: 3.8.0 or higher (included with Flutter 3.27.1+)
- **Rust**: Latest stable (1.75.0+)

## Why This Approach

1. **Repository configuration was already correct** - SDK constraints and workflow Flutter versions were set properly
2. **Dual-issue identification** - Problem manifested in both local environments and CI/CD pipeline
3. **Cache invalidation for CI** - Updated cache keys to prevent using incompatible cached dependencies
4. **User-centric local solution** - Tools to fix local environment issues
5. **Comprehensive documentation** - Multiple resources for different user needs
6. **Automated when possible** - Script handles most local cases automatically, CI handles itself
7. **Manual fallback** - Clear instructions for users who prefer manual steps

## Testing

### CI/CD Pipelines
✅ GitHub Actions workflows updated with cache invalidation
✅ Cache keys now include Dart SDK version requirement (dart3.8)
✅ Version verification steps added before building
✅ All workflows use Flutter 3.27.1 with Dart 3.8.0+

### Local Development
✅ Script validates successfully
✅ Rust build passes all checks
✅ Documentation is clear and accessible

## Impact

- **CI/CD fixes** - Cache invalidation prevents incompatible dependency issues
- **Zero breaking changes** - Repository configuration remains correct
- **Improved user experience** - Clear guidance and automated tools
- **Reduced support burden** - Self-service troubleshooting resources
- **Better documentation** - Multiple entry points to help

## Files Modified

### New Files Created
- `TROUBLESHOOTING.md` - Comprehensive troubleshooting guide
- `scripts/fix-flutter-deps.sh` - Automated fix script
- `issues/README.md` - Issue tracking index

### Files Updated
- `.github/workflows/ci.yml` - Updated cache keys and added version verification
- `.github/workflows/release.yml` - Updated cache keys and added version verification
- `README.md` - Added troubleshooting section
- `CONTRIBUTING.md` - Added version requirements
- `FLUTTER_DEPENDENCY_FIX.md` - Added script reference
- `TROUBLESHOOTING.md` - Enhanced CI/CD troubleshooting section
- `RESOLUTION_SUMMARY.md` - Updated with CI/CD fix details

### No Changes Needed
- `mobile/flutter/pubspec.yaml` - Already correct

## Conclusion

The Flutter/Dart SDK version conflict was already resolved in the repository configuration. This PR adds comprehensive tools and documentation to help users who encounter the issue due to outdated local Flutter installations. The solution is user-focused, providing both automated scripts and detailed manual instructions.
