# Android App Icon Assets

## Overview

This directory contains the Internet Archive logo used as the Android app launcher icon.

## Icon Files

- **internet_archive_logo.svg** - Master SVG file for the Internet Archive building logo
- **ic_launcher_1024.png** - High-resolution (1024x1024) icon for Google Play Store submission

## Icon Design

The logo represents the Internet Archive's iconic building with:
- 5 columns representing the archive's foundation
- Classical architecture with pediment/roof
- Grayscale color scheme for OS theming compatibility
- Clean, minimalist design suitable for app icons

## Regenerating Android Icons

If you need to regenerate the Android launcher icons from the SVG:

### Prerequisites

```bash
# Install rsvg-convert (for SVG to PNG conversion)
sudo apt-get install librsvg2-bin

# Or on macOS:
brew install librsvg
```

### Generate Icons

```bash
cd mobile/flutter/assets/icons

# Generate PNG icons at required Android densities
rsvg-convert -w 48 -h 48 internet_archive_logo.svg -o ../../android/app/src/main/res/mipmap-mdpi/ic_launcher.png
rsvg-convert -w 72 -h 72 internet_archive_logo.svg -o ../../android/app/src/main/res/mipmap-hdpi/ic_launcher.png
rsvg-convert -w 96 -h 96 internet_archive_logo.svg -o ../../android/app/src/main/res/mipmap-xhdpi/ic_launcher.png
rsvg-convert -w 144 -h 144 internet_archive_logo.svg -o ../../android/app/src/main/res/mipmap-xxhdpi/ic_launcher.png
rsvg-convert -w 192 -h 192 internet_archive_logo.svg -o ../../android/app/src/main/res/mipmap-xxxhdpi/ic_launcher.png

# Generate high-res icon for Play Store (1024x1024)
rsvg-convert -w 1024 -h 1024 internet_archive_logo.svg -o ic_launcher_1024.png
```

### Update Vector Drawable (Optional)

The vector drawable version (`drawable/ic_launcher_foreground.xml`) is hand-crafted for optimal Android rendering. If you update the SVG significantly, you may need to update the vector drawable paths as well.

## Android Density Guidelines

| Density | Scale | Icon Size | Usage |
|---------|-------|-----------|-------|
| mdpi    | 1.0x  | 48x48 px  | Baseline |
| hdpi    | 1.5x  | 72x72 px  | Common older devices |
| xhdpi   | 2.0x  | 96x96 px  | Most common today |
| xxhdpi  | 3.0x  | 144x144 px | High-end devices |
| xxxhdpi | 4.0x  | 192x192 px | Very high-end devices |

## Adaptive Icon Support

The app uses adaptive icons for Android 8.0+ (API 26+):
- **Foreground**: Vector drawable of the building (`drawable/ic_launcher_foreground.xml`)
- **Background**: White background (`drawable/ic_launcher_background.xml`)
- **Monochrome**: Same as foreground, enables Material You theming on Android 13+

## Using the Logo in Flutter

To use the logo within the Flutter app:

```dart
import 'package:flutter/material.dart';
import 'package:flutter_svg/flutter_svg.dart';

// Use the SVG logo
SvgPicture.asset(
  'assets/icons/internet_archive_logo.svg',
  width: 48,
  height: 48,
  colorFilter: ColorFilter.mode(
    Theme.of(context).primaryColor,
    BlendMode.srcIn,
  ),
)
```

Note: You'll need to add `flutter_svg` to your `pubspec.yaml` dependencies if not already present.

## Resources

- [Android Icon Guidelines](https://developer.android.com/guide/practices/ui_guidelines/icon_design_launcher)
- [Adaptive Icons](https://developer.android.com/develop/ui/views/launch/icon_design_adaptive)
- [Material You Icons](https://m3.material.io/styles/icons/overview)
- [Internet Archive Branding](https://archive.org/about/)
