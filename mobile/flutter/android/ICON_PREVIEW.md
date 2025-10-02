# Internet Archive App Icon Preview

## Icon Design

The Internet Archive app icon features a grayscale representation of the iconic Internet Archive building:

- **5 Classical Columns**: Representing the foundation of the archive
- **Pediment Roof**: Classical Greek/Roman architecture style
- **Steps**: Leading up to the entrance
- **Grayscale Colors**: Enables OS-level theming (Material You on Android 13+)

## Visual Representation

```
     /\
    /  \
   /____\
   ______
   |  |  |  |  |
   |  |  |  |  |
   |  |  |  |  |
   |  |  |  |  |
   |  |  |  |  |
   |  |  |  |  |
   __________
  ____________
 ______________
```

## Icon Sizes

The icon is available in multiple densities:

| Density | Size      | DPI  | Usage                    |
|---------|-----------|------|--------------------------|
| mdpi    | 48×48px   | 160  | Baseline density         |
| hdpi    | 72×72px   | 240  | Common older devices     |
| xhdpi   | 96×96px   | 320  | Most common today        |
| xxhdpi  | 144×144px | 480  | High-end devices         |
| xxxhdpi | 192×192px | 640  | Very high-end devices    |
| Play Store | 1024×1024px | - | Google Play Store submission |

## Adaptive Icon Behavior

On Android 8.0+ (API 26+), the icon adapts to different launcher shapes:

- **Circle**: Circular mask applied
- **Rounded Square**: Rounded corners
- **Squircle**: Superellipse shape
- **Teardrop**: Teardrop shape (some OEM launchers)

The safe zone (center 66dp of 108dp) contains the building icon, ensuring it's always visible regardless of mask shape.

## Material You Theming (Android 13+)

On Android 13 and later, the icon can be dynamically themed:

1. **Dynamic Color**: OS can recolor the icon to match user's wallpaper-based theme
2. **Monochrome Mode**: System can display a single-color version
3. **Themed Icons**: Integration with system-wide theming

## Color Scheme

The grayscale palette was chosen to:
- Match the Internet Archive's classic, scholarly aesthetic
- Enable dynamic theming without losing brand identity
- Ensure good contrast on all backgrounds
- Support accessibility requirements

### Colors Used

- `#CCCCCC` - Column highlights (light gray)
- `#AAAAAA` - Column capitals (medium-light gray)
- `#999999` - Building base and roof detail (medium gray)
- `#888888` - Steps and pediment (darker gray)
- `#777777` - Second step (dark gray)
- `#666666` - Foundation base (darkest gray)
- `#FFFFFF` - Background (white)

## Testing the Icon

To see the icon on different Android versions:

1. **Android 7 and earlier**: Shows static PNG from mipmap directory
2. **Android 8-12**: Shows adaptive icon with foreground/background layers
3. **Android 13+**: Shows adaptive icon with Material You theming support

### Visual Verification Checklist

- [ ] Icon displays correctly at all densities
- [ ] Adaptive icon animates smoothly when long-pressed
- [ ] Icon is recognizable at small sizes (status bar)
- [ ] Foreground/background layers don't overlap incorrectly
- [ ] Monochrome version maintains recognizability
- [ ] Dynamic theming works correctly on Android 13+
- [ ] Icon matches Internet Archive branding

## Files Location

- **Source SVG**: `mobile/flutter/assets/icons/internet_archive_logo.svg`
- **PNG Icons**: `mobile/flutter/android/app/src/main/res/mipmap-*/ic_launcher.png`
- **Vector Drawables**: `mobile/flutter/android/app/src/main/res/drawable/ic_launcher_*.xml`
- **Adaptive Config**: `mobile/flutter/android/app/src/main/res/mipmap-anydpi-v26/ic_launcher.xml`
- **Play Store**: `mobile/flutter/assets/icons/ic_launcher_1024.png`

## References

- [Android Adaptive Icons](https://developer.android.com/develop/ui/views/launch/icon_design_adaptive)
- [Material Design Icons](https://m3.material.io/styles/icons/overview)
- [Internet Archive](https://archive.org/about/)
