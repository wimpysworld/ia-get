# Internet Archive Logo Android Implementation

## Overview

This document describes the implementation of the Internet Archive logo as the Android application icon, following Android best practices and Material Design guidelines.

## Implementation Details

### 1. App Icon Assets

The Internet Archive building logo has been added in multiple densities to support all Android devices:

- **mipmap-mdpi**: 48x48px (baseline)
- **mipmap-hdpi**: 72x72px (1.5x)
- **mipmap-xhdpi**: 96x96px (2x)
- **mipmap-xxhdpi**: 144x144px (3x)
- **mipmap-xxxhdpi**: 192x192px (4x)

### 2. Adaptive Icon Support (Android 8.0+)

Implemented adaptive icons for modern Android devices:

- **Foreground layer**: Vector drawable of the Internet Archive building with columns
- **Background layer**: White background for contrast
- **Monochrome layer**: Same as foreground, enables Material You theming on Android 13+

Files:
- `drawable/ic_launcher_foreground.xml` - Vector drawable of the building
- `drawable/ic_launcher_background.xml` - Background layer
- `mipmap-anydpi-v26/ic_launcher.xml` - Adaptive icon configuration

### 3. Material You Theming Support

The monochrome layer in the adaptive icon enables:
- Dynamic theming on Android 13+ (API 33+)
- OS can recolor the icon to match user's chosen theme
- Maintains accessibility and brand recognition

### 4. Logo Design

The logo represents the Internet Archive's iconic building with:
- 5 columns representing the archive's foundation
- Classical architecture pediment/roof
- Grayscale color scheme for theming compatibility
- Vector format for scalability

### 5. Build Configuration Updates

Updated `mobile/flutter/android/app/build.gradle` to reference the new launcher icon:
- Changed `appIcon` from `@android:drawable/ic_menu_gallery` to `@mipmap/ic_launcher`
- Applied to all product flavors: development, staging, production

## Android Best Practices Followed

✅ **Multiple Densities**: Icons provided for all standard Android densities
✅ **Adaptive Icons**: Full adaptive icon support for Android 8.0+ (API 26+)
✅ **Vector Drawables**: Foreground uses vector format for perfect scaling
✅ **Material You**: Monochrome layer for dynamic theming (Android 13+)
✅ **Proper Structure**: Icons placed in standard Android resource directories
✅ **Grayscale Design**: Enables OS theming while maintaining brand identity

## Testing Recommendations

To verify the implementation:

1. **Build the app**:
   ```bash
   cd mobile/flutter
   flutter build apk --flavor production
   ```

2. **Install on device**:
   ```bash
   flutter install
   ```

3. **Verify icon appearance**:
   - Check app icon in launcher
   - Long-press icon to see adaptive icon animation
   - On Android 13+, verify dynamic theming matches system theme
   - Test on multiple devices with different densities

4. **Test adaptive icon**:
   - Use different launcher apps (Google Pixel Launcher, Samsung One UI, etc.)
   - Verify icon masks correctly (circular, rounded square, etc.)

## Resources

- **Logo SVG**: `assets/icons/internet_archive_logo.svg`
- **Android Icon Guidelines**: https://developer.android.com/guide/practices/ui_guidelines/icon_design_launcher
- **Adaptive Icons**: https://developer.android.com/develop/ui/views/launch/icon_design_adaptive
- **Material You**: https://m3.material.io/styles/icons/overview

## Future Enhancements

Potential improvements:
- [ ] Create round icon variant (`ic_launcher_round`) for legacy devices
- [ ] Add notification icon variant
- [ ] Create app icon animations for special events
- [ ] Generate icon variants for different themes (light/dark)
