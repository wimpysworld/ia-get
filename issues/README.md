# Issue Tracking and Resolution

This directory contains documentation for specific issues that have been encountered and resolved.

## Current Issues and Solutions

### Flutter/Dart SDK Version Conflict

**Problem**: Build fails with error about Dart SDK version being too old:
```
The current Dart SDK version is 3.X.X.
Because internet_archive_helper requires SDK version >=3.8.0
```

**Status**: ✅ RESOLVED

**Solution**: The repository configuration has been updated to use Flutter 3.35.0+ (which includes Dart 3.8.0+). If you're still experiencing this issue locally:

1. **Quick Fix**: Run `./scripts/fix-flutter-deps.sh`
2. **Manual Fix**: Run `flutter upgrade` to update to Flutter 3.35.0+
3. **See**: [TROUBLESHOOTING.md](../TROUBLESHOOTING.md) for detailed instructions

The fix has been applied in these files:
- ✅ `mobile/flutter/pubspec.yaml` - SDK constraint: `>=3.8.0 <4.0.0`
- ✅ `.github/workflows/ci.yml` - Flutter version: `3.35.0`
- ✅ `.github/workflows/release.yml` - Flutter version: `3.35.0`

### CI Job Failures

**Problem**: CI job fails after successful build

**Status**: 🔍 DOCUMENTED

See: [CI_Job_Fails_with_Exit_Code_1_After_Successful_Build.md](CI_Job_Fails_with_Exit_Code_1_After_Successful_Build.md)

## Getting Help

If you encounter a new issue:

1. Check [TROUBLESHOOTING.md](../TROUBLESHOOTING.md)
2. Search [existing GitHub issues](https://github.com/Gameaday/ia-get-cli/issues)
3. If it's a new issue, create a GitHub issue with:
   - Clear description
   - Steps to reproduce
   - Environment details (OS, Flutter version, Rust version)
   - Full error message

## Contributing Issue Documentation

If you encounter and resolve an issue:

1. Document the problem, cause, and solution
2. Add it to this README
3. Create a detailed markdown file in this directory if needed
4. Submit a PR to help other users
