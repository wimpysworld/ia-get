# Flutter Project Separation Guide

## Overview

This guide explains how to separate the Flutter mobile app from the main `ia-get-cli` repository into its own standalone repository. The Flutter app is already fully self-contained and can be moved without modifications.

---

## Current Architecture ✅

The Flutter app is completely independent:
- ✅ **Zero dependencies** on Rust CLI code
- ✅ **Pure Dart** implementation (no FFI)
- ✅ **Self-contained** assets and resources
- ✅ **Independent** build system (Flutter SDK)
- ✅ **Separate** dependency management (pubspec.yaml)

---

## Prerequisites

### Required Tools
```bash
# Flutter SDK (3.27.0 or higher)
flutter --version

# Dart SDK (3.8.0 or higher) - included with Flutter
dart --version

# Git
git --version
```

### Optional Tools
- Android Studio or VS Code with Flutter extensions
- Android SDK for Android builds
- Xcode for iOS builds (macOS only)

---

## Separation Steps

### 1. Create New Repository

**Option A: GitHub Web Interface**
1. Go to https://github.com/new
2. Repository name: `internet-archive-helper-mobile` (or your choice)
3. Description: "Flutter mobile app for downloading files from Internet Archive"
4. Choose Public or Private
5. Initialize with README: **No** (we'll bring our own)
6. Click "Create repository"

**Option B: GitHub CLI**
```bash
gh repo create internet-archive-helper-mobile --public --description "Flutter mobile app for downloading files from Internet Archive"
```

### 2. Copy Flutter Project Files

From the `ia-get-cli` repository, copy the entire `mobile/flutter/` directory:

```powershell
# PowerShell
$source = "C:\path\to\ia-get-cli\mobile\flutter"
$dest = "C:\path\to\new-repo"

# Copy all files
Copy-Item -Recurse -Force "$source\*" -Destination $dest

# Verify structure
tree $dest /F
```

**What Gets Copied:**
```
new-repo/
├── android/                # Android-specific configuration
├── assets/                 # App assets (images, icons)
├── ios/                    # iOS-specific configuration (if exists)
├── lib/                    # Dart source code
│   ├── core/              # Core utilities
│   ├── models/            # Data models
│   ├── providers/         # State management
│   ├── screens/           # UI screens
│   ├── services/          # Business logic
│   ├── widgets/           # Reusable widgets
│   └── main.dart          # App entry point
├── test/                   # Unit tests
├── analysis_options.yaml   # Dart analyzer configuration
├── build.yaml             # Build configuration
├── pubspec.yaml           # Dependencies
└── README.md              # Documentation
```

### 3. Verify File Integrity

Check that all critical files are present:

```bash
cd new-repo

# Verify main files exist
ls -l pubspec.yaml
ls -l lib/main.dart
ls -l android/app/build.gradle

# Check dependencies can resolve
flutter pub get

# Verify no broken imports
flutter analyze
```

### 4. Update README.md

Create a new README specifically for the Flutter app:

```markdown
# Internet Archive Helper - Mobile App

A Flutter mobile application for browsing and downloading files from the Internet Archive.

## Features
- 📱 Cross-platform (Android & iOS)
- 🔍 Search Internet Archive collections
- 📥 Download files with progress tracking
- ✨ Clean, modern Material Design UI
- 🚀 Pure Dart implementation (fast & reliable)

## Getting Started

### Prerequisites
- Flutter SDK 3.27+ ([Install Flutter](https://flutter.dev/docs/get-started/install))
- Android Studio or VS Code

### Installation
1. Clone this repository
2. Run `flutter pub get` to install dependencies
3. Run `flutter run` to start the app

### Building for Production
```bash
# Android APK
flutter build apk --release

# Android App Bundle (for Play Store)
flutter build appbundle --release

# iOS (requires macOS)
flutter build ios --release
```

## Architecture
- **State Management**: Provider
- **Networking**: Pure Dart (http/dio packages)
- **API**: Internet Archive REST API
- **Storage**: Hive for local data

## Contributing
Contributions welcome! Please read [CONTRIBUTING.md](CONTRIBUTING.md) first.

## License
MIT License - see [LICENSE](LICENSE) file
```

### 5. Set Up Git Repository

```bash
cd new-repo

# Initialize git
git init

# Add all files
git add .

# First commit
git commit -m "Initial commit: Flutter app from ia-get-cli separation"

# Link to remote
git remote add origin https://github.com/yourusername/internet-archive-helper-mobile.git

# Push to GitHub
git branch -M main
git push -u origin main
```

### 6. Configure GitHub Repository

#### A. Repository Settings
- **About Section**: Add description and topics
  - Topics: `flutter`, `dart`, `internet-archive`, `mobile-app`, `android`, `ios`
  - Description: "Flutter mobile app for Internet Archive"

#### B. Branch Protection
- Settings → Branches → Add rule
  - Branch name pattern: `main`
  - Require pull request reviews
  - Require status checks to pass

#### C. GitHub Actions (CI/CD)
Create `.github/workflows/flutter.yml`:

```yaml
name: Flutter CI

on:
  push:
    branches: [ main, develop ]
  pull_request:
    branches: [ main, develop ]

jobs:
  build:
    runs-on: ubuntu-latest
    
    steps:
    - uses: actions/checkout@v4
    
    - name: Set up Flutter
      uses: subosito/flutter-action@v2
      with:
        flutter-version: '3.27.0'
        channel: 'stable'
    
    - name: Install dependencies
      run: flutter pub get
    
    - name: Analyze code
      run: flutter analyze
    
    - name: Run tests
      run: flutter test
    
    - name: Build APK
      run: flutter build apk --debug

  test:
    runs-on: ubuntu-latest
    
    steps:
    - uses: actions/checkout@v4
    
    - name: Set up Flutter
      uses: subosito/flutter-action@v2
      with:
        flutter-version: '3.27.0'
    
    - name: Run tests with coverage
      run: |
        flutter pub get
        flutter test --coverage
    
    - name: Upload coverage
      uses: codecov/codecov-action@v3
      with:
        files: coverage/lcov.info
```

### 7. Update Original Repository

In the `ia-get-cli` repository:

#### A. Update Main README
Add a section about the mobile app:

```markdown
## 📱 Mobile App

The Flutter mobile app has been moved to its own repository for independent development:

**🔗 [Internet Archive Helper Mobile App](https://github.com/yourusername/internet-archive-helper-mobile)**

Features:
- Cross-platform (Android & iOS)
- Browse and download from Internet Archive
- Pure Dart implementation (no native dependencies)
- Modern Material Design interface

See the mobile repo for installation and development instructions.
```

#### B. Archive Mobile Directory (Optional)
```bash
# Create archived directory
mkdir -p docs/archived/mobile_snapshot

# Move mobile directory
mv mobile docs/archived/mobile_snapshot/

# Or delete if no longer needed
rm -rf mobile

# Commit changes
git add .
git commit -m "Mobile app moved to separate repository"
```

---

## Post-Separation Checklist

### New Repository (Flutter App) ✅
- [ ] All files copied correctly
- [ ] `flutter pub get` runs successfully
- [ ] `flutter analyze` shows no errors
- [ ] `flutter test` passes
- [ ] Android build works: `flutter build apk`
- [ ] README.md updated for standalone project
- [ ] LICENSE file present
- [ ] .gitignore configured properly
- [ ] GitHub Actions CI/CD configured
- [ ] Repository settings configured (topics, description)

### Original Repository (Rust CLI) ✅
- [ ] README updated with link to mobile repo
- [ ] Mobile directory removed or archived
- [ ] Documentation updated (remove mobile references)
- [ ] CONTRIBUTING.md updated if needed
- [ ] Links to mobile docs redirected

### Communication ✅
- [ ] Announce separation in CHANGELOG
- [ ] Update documentation links
- [ ] Notify contributors (if any)
- [ ] Update project website/wiki (if exists)

---

## Directory Structure (After Separation)

### New Flutter Repository
```
internet-archive-helper-mobile/
├── .github/
│   └── workflows/
│       └── flutter.yml
├── android/
├── assets/
├── ios/
├── lib/
├── test/
├── .gitignore
├── analysis_options.yaml
├── build.yaml
├── CHANGELOG.md
├── CONTRIBUTING.md
├── LICENSE
├── pubspec.yaml
└── README.md
```

### Original Rust CLI Repository
```
ia-get-cli/
├── src/              # Rust source code
├── tests/            # Rust tests
├── docs/             # CLI documentation
│   └── archived/
│       └── mobile_snapshot/  # Optional: archived mobile code
├── Cargo.toml
├── README.md         # Updated with link to mobile repo
└── ...
```

---

## Dependency Management

### Flutter App Dependencies (pubspec.yaml)
Already up-to-date with latest versions:

```yaml
dependencies:
  flutter:
    sdk: flutter
  provider: ^6.1.5
  http: ^1.5.0
  dio: ^5.9.0
  hive: ^2.2.3
  path_provider: ^2.1.5
  shared_preferences: ^2.3.5

dev_dependencies:
  flutter_test:
    sdk: flutter
  freezed: ^3.2.3
  json_serializable: ^6.11.1
  build_runner: ^2.9.0
  freezed_annotation: ^3.1.0
```

**Update Strategy:**
- Run `flutter pub outdated` quarterly
- Update dependencies before major releases
- Test thoroughly after updates

---

## Maintenance Strategy

### Version Synchronization
The mobile app and CLI are **independent** but may share version numbers for clarity:

**Option 1: Synchronized Versions**
- CLI: `v1.6.0`
- Mobile: `v1.6.0`
- Benefits: Clear matching versions
- Drawbacks: May not make sense if features diverge

**Option 2: Independent Versions**
- CLI: `v1.6.0`
- Mobile: `v2.1.0`
- Benefits: Each project evolves independently
- Drawbacks: Less obvious they're related

**Recommendation**: Use independent versioning since they're separate codebases with different release cycles.

### Release Coordination
- CLI and mobile releases can happen independently
- Document compatibility if APIs change
- Consider shared API version numbers

---

## Benefits of Separation

### Development Benefits ✅
- **Faster Builds**: No Rust compilation for mobile developers
- **Simpler Setup**: Only Flutter SDK required
- **Clearer Focus**: Each repo has single responsibility
- **Independent Releases**: Deploy mobile updates without CLI changes
- **Better CI/CD**: Optimized workflows for each platform

### Community Benefits ✅
- **Easier Contributing**: Mobile contributors don't need Rust knowledge
- **Clearer Issues**: Separate issue trackers for each project
- **Focused Documentation**: Each repo has relevant docs only
- **Better Discovery**: Flutter devs find mobile repo more easily

### Technical Benefits ✅
- **Smaller Repos**: Faster cloning and operations
- **No Build Conflicts**: Flutter and Rust builds don't interfere
- **Independent Dependencies**: Update Flutter/Rust dependencies separately
- **Clearer Architecture**: No confusion about project structure

---

## Troubleshooting

### Issue: Assets Not Found
**Problem**: Images/icons missing after separation
**Solution**: Verify `assets/` directory copied and `pubspec.yaml` references correct paths

```yaml
flutter:
  assets:
    - assets/images/
    - assets/icons/
```

### Issue: Build Fails
**Problem**: Android/iOS build errors
**Solution**: 
1. Clean build: `flutter clean`
2. Get dependencies: `flutter pub get`
3. Rebuild: `flutter build apk`

### Issue: Old Import Paths
**Problem**: Code references old directory structure
**Solution**: Search and replace old paths
```bash
# Find old imports
grep -r "package:ia_get_cli" lib/

# Should use:
import 'package:internet_archive_helper/...'
```

### Issue: Git History Lost
**Problem**: Want to preserve commit history
**Solution**: Use `git subtree` or `git filter-branch` (advanced)

---

## Advanced: Preserving Git History

If you want to keep the commit history for the mobile directory:

```bash
# Clone original repo
git clone https://github.com/yourusername/ia-get-cli.git mobile-extracted

cd mobile-extracted

# Extract mobile/flutter subdirectory with history
git filter-branch --subdirectory-filter mobile/flutter -- --all

# Clean up
git reset --hard
git gc --aggressive
git prune

# Add new remote
git remote set-url origin https://github.com/yourusername/internet-archive-helper-mobile.git

# Push to new repo
git push -u origin main
```

**Warning**: This is an advanced operation. Test in a separate clone first!

---

## Questions & Support

### Where to Ask Questions?
- **Mobile App Issues**: New mobile repo issue tracker
- **CLI Issues**: Original ia-get-cli issue tracker
- **API/Integration**: Either repo (cross-link if needed)

### Documentation Links
- **Flutter Documentation**: https://flutter.dev/docs
- **Internet Archive API**: https://archive.org/services/docs/api
- **Provider Package**: https://pub.dev/packages/provider

---

## Next Steps After Separation

1. **Set Up Development Environment**
   - Install Flutter SDK
   - Configure Android Studio / VS Code
   - Clone new repository

2. **Configure CI/CD**
   - Set up GitHub Actions
   - Configure automated testing
   - Set up deployment pipelines

3. **Plan First Release**
   - Update version to 1.0.0 (fresh start)
   - Create release notes
   - Tag release in Git

4. **Community Building**
   - Create CONTRIBUTING.md
   - Set up issue templates
   - Add code of conduct

5. **Future Enhancements**
   - iOS support
   - Tablet optimization
   - Offline mode
   - Background downloads

---

*Guide Version: 1.0*  
*Last Updated: October 5, 2025*  
*Compatible with: Flutter 3.27+, Dart 3.8+*
