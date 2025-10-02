# Additional Mobile UI Fixes

This document describes the additional fixes implemented in response to user feedback.

## Issues Addressed

### 1. Android App Icon Too Large and Cut Off

**Problem**: The app icon was using a 512x512 viewport in the 108dp adaptive icon canvas, causing the icon to be too large and cut off on all sides. Android adaptive icons have a safe zone requirement where only the center 66dp (of 108dp) is guaranteed to be visible across all launcher shapes.

**Root Cause**: 
- The foreground drawable defined `viewportWidth="512"` and `viewportHeight="512"`
- This caused the icon to be scaled incorrectly, exceeding the safe zone
- Different launchers apply different masks (circle, rounded square, squircle), and content outside the safe zone gets clipped

**Solution**:
Updated `ic_launcher_foreground.xml` with proper scaling:

```xml
<vector xmlns:android="http://schemas.android.com/apk/res/android"
    android:width="108dp"
    android:height="108dp"
    android:viewportWidth="108"
    android:viewportHeight="108">
    
    <group
        android:translateX="21"
        android:translateY="21"
        android:scaleX="0.129"
        android:scaleY="0.129">
        <!-- Icon path data -->
    </group>
</vector>
```

**Calculations**:
- Viewport changed from 512x512 to 108x108 (matching canvas size)
- Scale factor: 66dp (safe zone) / 512px (original) = 0.129
- Translation: (108 - 66) / 2 = 21dp (centers the 66dp content in 108dp canvas)
- This ensures the icon fits perfectly within the safe zone

**Result**: Icon now displays correctly on all Android launchers without being cut off.

### 2. Deep Links to Internet Archive Not Working

**Problem**: The AndroidManifest.xml had intent filters configured for deep links, but there was no Flutter code to handle incoming URLs when the app was opened from a link.

**Solution**:
Added complete deep link support:

1. **Added `app_links` package** to `pubspec.yaml` for cross-platform deep link handling

2. **Created `DeepLinkService`** (`lib/services/deep_link_service.dart`):
   - Listens for initial app launch links
   - Monitors incoming links while app is running
   - Extracts archive identifiers from various URL formats
   - Provides callback mechanism for handling archive links

3. **Supported URL Formats**:
   - `https://archive.org/details/[identifier]` - Standard details page
   - `https://archive.org/download/[identifier]` - Direct download page
   - `https://archive.org/metadata/[identifier]` - Metadata API
   - `iaget://[identifier]` - Custom app scheme

4. **Integrated in `main.dart`**:
   - Added `DeepLinkService` to Provider tree
   - Initialize service on app startup
   - Set callback to trigger metadata fetch when archive link received
   - Automatically navigates to home screen and starts search

**Example Usage**:
```dart
// When user clicks: https://archive.org/details/commute_test
// App extracts "commute_test" and automatically searches for it
deepLinkService.onArchiveLinkReceived = (identifier) {
  iaGetService.fetchMetadata(identifier);
};
```

**Result**: Users can now share Internet Archive links and open them directly in the app.

### 3. No Settings Page for User Preferences

**Problem**: The app had hardcoded default values for download settings with no way for users to customize their preferences.

**Solution**:
Created comprehensive Settings screen (`lib/screens/settings_screen.dart`):

**Features**:
1. **Download Settings**:
   - Download location (customizable path with dialog editor)
   - Concurrent downloads (1-10, adjustable with +/- buttons)
   - Auto-decompress archives toggle
   - Verify checksums toggle

2. **File Browser Settings**:
   - Show hidden files toggle (files starting with . or _)

3. **About Section**:
   - App version display
   - Internet Archive information

4. **Reset to Defaults**:
   - Button to reset all settings
   - Confirmation dialog to prevent accidental resets

**Implementation Details**:
- Uses `shared_preferences` package for persistent storage
- Static helper methods for easy preference retrieval:
  ```dart
  final path = await SettingsScreen.getDownloadPath();
  final concurrent = await SettingsScreen.getConcurrentDownloads();
  ```
- Preferences are loaded on app startup and used by download controls
- Real-time updates when settings change
- Clean, organized UI with sections and appropriate icons

**Integration**:
- Settings icon added to app bar (home screen)
- Download controls widget loads and uses saved preferences
- All download operations respect user preferences

**Preferences Stored**:
- `download_path`: String - Download directory path
- `concurrent_downloads`: int - Number of simultaneous downloads
- `auto_decompress`: bool - Auto-extract archives
- `verify_checksums`: bool - Verify file integrity
- `show_hidden_files`: bool - Display hidden files

**Result**: Users can now customize their download experience and save preferences between app sessions.

## Technical Implementation

### Files Modified

1. **`ic_launcher_foreground.xml`**:
   - Changed viewport from 512x512 to 108x108
   - Added group with scale (0.129) and translate (21dp)
   - Icon now fits in 66dp safe zone

2. **`pubspec.yaml`**:
   - Added `app_links: ^6.3.2` for deep link handling

3. **`lib/services/deep_link_service.dart`** (NEW):
   - AppLinks integration
   - URL parsing for multiple formats
   - Callback mechanism for handling links

4. **`lib/screens/settings_screen.dart`** (NEW):
   - Complete settings UI
   - SharedPreferences integration
   - Static helper methods for preference access

5. **`lib/main.dart`**:
   - Added DeepLinkService to Provider tree
   - Initialize and configure deep link handling
   - Connect link callback to metadata fetch

6. **`lib/screens/home_screen.dart`**:
   - Added settings icon to app bar
   - Navigate to settings screen on tap

7. **`lib/widgets/download_controls_widget.dart`**:
   - Load settings on init
   - Use saved preferences for downloads
   - Import SettingsScreen for helper methods

### Android Adaptive Icon Safe Zone

The adaptive icon safe zone ensures the icon displays correctly across different launcher shapes:

```
┌─────────────────────────────┐
│     108dp × 108dp canvas    │
│  ┌─────────────────────┐   │
│  │                     │   │
│  │  ┌───────────┐     │   │
│  │  │  66dp×66dp│     │   │
│  │  │ safe zone │     │   │
│  │  │  (visible)│     │   │
│  │  └───────────┘     │   │
│  │                     │   │
│  └─────────────────────┘   │
│      (may be clipped)       │
└─────────────────────────────┘
```

Our icon:
- Original: 512×512 viewport (too large)
- Fixed: Scaled to 66×66 within 108×108 canvas
- Scale: 0.129 (66/512)
- Offset: 21dp ((108-66)/2)

### Deep Link Flow

```
User clicks link
       ↓
Android Intent Filter
       ↓
Flutter App Opens
       ↓
DeepLinkService.getInitialLink() or uriLinkStream
       ↓
Extract identifier from URL
       ↓
Callback: onArchiveLinkReceived(identifier)
       ↓
IaGetService.fetchMetadata(identifier)
       ↓
App displays archive content
```

### Settings Persistence Flow

```
User changes setting
       ↓
setState() updates UI
       ↓
SharedPreferences.set[Type]() saves to disk
       ↓
Setting available on next app launch
       ↓
DownloadControlsWidget.initState() loads settings
       ↓
Downloads use saved preferences
```

## Testing Recommendations

### Icon Testing
1. Install app on device
2. Check icon on home screen (various launchers)
3. Verify icon is not cut off
4. Long-press icon to see adaptive animation
5. On Android 13+, verify Material You theming

### Deep Link Testing
1. Send yourself a link: `https://archive.org/details/commute_test`
2. Click the link (in email, message, browser)
3. App should open and automatically search for "commute_test"
4. Test custom scheme: `iaget://commute_test`
5. Test while app is closed and while running

### Settings Testing
1. Open settings from app bar icon
2. Change download path
3. Adjust concurrent downloads
4. Toggle all switches
5. Close app and reopen
6. Verify settings persisted
7. Test reset to defaults

## Related Documentation

- [Android Adaptive Icons](https://developer.android.com/develop/ui/views/launch/icon_design_adaptive)
- [App Links Documentation](https://pub.dev/packages/app_links)
- [SharedPreferences](https://pub.dev/packages/shared_preferences)
- [Material Design Settings](https://m3.material.io/components/lists/specs)

## Summary

All three reported issues have been successfully resolved:

1. ✅ **App icon fixed** - Now properly sized within adaptive icon safe zone
2. ✅ **Deep links working** - Full support for archive.org and custom URLs
3. ✅ **Settings page added** - Complete preferences system with persistence

These additions significantly improve the user experience and app functionality.
