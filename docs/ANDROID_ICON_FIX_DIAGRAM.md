# Android Adaptive Icon Layer Structure

This diagram illustrates the correct adaptive icon structure before and after the fix.

## BEFORE FIX (Incorrect)

```
┌─────────────────────────────────────┐
│   Adaptive Icon Configuration       │
│                                     │
│  ┌──────────────────────────────┐  │
│  │  Background Layer            │  │
│  │  (White fill)                │  │
│  │  ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓  │  │
│  │  ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓  │  │
│  │  ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓  │  │
│  └──────────────────────────────┘  │
│                                     │
│  ┌──────────────────────────────┐  │
│  │  Foreground Layer ❌         │  │
│  │  (WHITE bg + BLACK icon)     │  │
│  │  ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓  │  │  ← White background
│  │  ▓▓▓ ███ ███ ███ ███ ▓▓▓  │  │  ← Black icon
│  │  ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓  │  │
│  └──────────────────────────────┘  │
│                                     │
│  Result: White on White = 🚫       │
│  Icon appears pure white!          │
└─────────────────────────────────────┘
```

## AFTER FIX (Correct)

```
┌─────────────────────────────────────┐
│   Adaptive Icon Configuration       │
│                                     │
│  ┌──────────────────────────────┐  │
│  │  Background Layer            │  │
│  │  (White fill)                │  │
│  │  ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓  │  │
│  │  ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓  │  │
│  │  ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓  │  │
│  └──────────────────────────────┘  │
│                                     │
│  ┌──────────────────────────────┐  │
│  │  Foreground Layer ✅         │  │
│  │  (Transparent + BLACK icon)  │  │
│  │  ░░░░░░░░░░░░░░░░░░░░░░░░░░  │  │  ← Transparent
│  │  ░░░ ███ ███ ███ ███ ░░░  │  │  ← Black icon visible
│  │  ░░░░░░░░░░░░░░░░░░░░░░░░░░  │  │
│  └──────────────────────────────┘  │
│                                     │
│  Result: Black on White = ✅       │
│  Icon displays correctly!          │
└─────────────────────────────────────┘
```

## Layer Compositing

### Before Fix
```
Background (White) + Foreground (White bg + Black icon) = White icon (no contrast!)
```

### After Fix  
```
Background (White) + Foreground (Transparent + Black icon) = Black icon on white (proper contrast!)
```

## Material You Theming (Android 13+)

The monochrome layer references the foreground drawable. Android applies theming:

```
User Theme: Purple
┌─────────────────────────┐
│  Monochrome Layer       │
│  (System recolors)      │
│  ░░░ ███ ███ ███ ░░░   │ → Android applies purple tint
│                         │
│  Result: Purple icon ✅ │
└─────────────────────────┘
```

## Key Principles

1. **Background Layer**: Provides the background color/pattern
2. **Foreground Layer**: Contains ONLY the icon shape (no background)
3. **Monochrome Layer**: Single-color version for system theming
4. **Transparency**: Foreground must have transparent background
5. **Separation**: Background and foreground must be separate layers

## Files Changed

- `ic_launcher_foreground.xml` - Removed white background path
- `ic_launcher_background.xml` - Unchanged (already correct)
- `ic_launcher.xml` - Unchanged (already correct)
- `generate-android-icons.sh` - Updated to generate correct structure

## References

- [Android Adaptive Icons](https://developer.android.com/develop/ui/views/launch/icon_design_adaptive)
- [Material You Design](https://m3.material.io/styles/icons/overview)
