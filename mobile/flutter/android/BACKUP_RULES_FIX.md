# Android Backup Rules Fix

## Issue
The Android build was failing with:
```
Error: backup_rules.xml:11: Error: cache/ is not in an included path [FullBackupContent]
```

## Root Cause
Android's backup system validation requires that all `<exclude>` paths must be within `<include>` paths. The configuration attempted to exclude directories that weren't included in the backup scope.

## Fix Applied
1. **backup_rules.xml**: Removed invalid exclusions for `cache/` and `temp/` directories
2. **data_extraction_rules.xml**: Fixed path mismatches in cloud-backup section

## Validation
Use the validation script to verify the configuration:
```bash
python3 /tmp/validate_backup_rules.py
```

The script checks that all exclude paths are within included paths per Android requirements.

## Impact
- ✅ Resolves Android build error
- ✅ Maintains app security (auth tokens still excluded)
- ✅ Preserves backup size optimization (large files still excluded)
- ✅ No functional changes to app behavior