# App Store Assets for Internet Archive Helper

This directory contains all assets required for Google Play Store submission and app distribution.

## Icon Specifications

### App Icon (ic_launcher)
- **Size**: 1024x1024px (high-resolution for Play Store)
- **Format**: Adaptive icon with PNG fallbacks
- **Design**: Internet Archive building with 5 columns (iconic architecture)
- **Colors**: High-contrast black and white design for improved brand recognition
- **Implementation**: 
  - ✅ PNG icons in 5 densities (mdpi through xxxhdpi)
  - ✅ Adaptive icon with foreground/background layers (Android 8.0+)
  - ✅ Monochrome layer for dynamic theming (Android 13+)
  - ✅ Vector drawable for perfect scaling
- **Location**: `android/app/src/main/res/mipmap-*/ic_launcher.png`
- **Source**: `assets/ia-helper.svg` (master source of truth)
- **Generation**: Use `scripts/generate-android-icons.sh` to regenerate all icons

### Feature Graphic
- **Size**: 1024x500px
- **Purpose**: Google Play Store listing header
- **Content**: App name, key features, Internet Archive integration

### Screenshots
Required for Play Store listing:
- **Phone**: 16:9 or 9:16 aspect ratio, minimum 320px
- **Tablet**: Screenshots showing tablet-optimized UI
- **Quantity**: 2-8 screenshots per device type

## App Store Metadata

### Short Description (80 characters)
"Download files from Internet Archive with smart filtering and mobile optimization"

### Full Description (4000 characters max)
```
Internet Archive Helper brings the power of the Internet Archive to your Android device with a fast, efficient, and user-friendly downloading experience.

🚀 KEY FEATURES
• Smart Search: Find archives quickly with real-time search
• Advanced Filtering: Filter by file type, size, and format before downloading
• Batch Downloads: Select multiple files with intelligent size management
• Mobile Optimized: Designed specifically for mobile storage constraints
• Progress Tracking: Real-time download progress with speed and ETA
• Background Downloads: Continue downloads even when app is minimized

📱 MOBILE-FIRST DESIGN
• Material Design 3 interface
• Light and dark theme support
• Responsive layout for phones and tablets
• Touch-optimized controls
• Efficient data usage

🎯 SMART FILE MANAGEMENT
• Pre-download filtering saves storage space
• Format-based selection (Text, Audio, Video, Images, Data)
• Size-aware selection prevents device overflow
• Comprehensive file information display

🔧 POWERFUL FEATURES
• Internet Archive URL handling
• Checksum verification for downloaded files
• Auto-extraction of compressed archives
• Session management for interrupted downloads
• Configurable concurrent download limits

Built with Flutter and Rust for maximum performance and reliability. Reuses 85% of the core logic from the desktop version while providing a completely mobile-optimized experience.

Perfect for researchers, students, archivists, and anyone who needs reliable access to Internet Archive content on mobile devices.
```

### Keywords
internet archive, download, mobile, files, research, archive, offline, batch download

### App Category
Tools / Productivity

### Content Rating
Everyone (no inappropriate content)

### Privacy Policy
Required for Play Store - covers data collection, storage, and usage

## Release Preparation

### Version Information
- **Version Name**: 1.0.0
- **Version Code**: 1
- **Target SDK**: 36 (Android 15)
- **Minimum SDK**: 21 (Android 5.0)

### Signing Configuration
- Production APK must be signed with release keystore
- App Bundle format recommended for Play Store
- ProGuard enabled for code optimization

### Testing Requirements
- Internal testing with alpha/beta releases
- Device compatibility testing across Android versions
- Performance testing on low-end devices
- Network condition testing (slow/unstable connections)

## Localization

### Supported Languages (Phase 1)
- English (primary)

### Future Localization
- Spanish
- French  
- German
- Japanese
- Chinese (Simplified)

## Store Listing Optimization

### Search Keywords
- internet archive
- archive downloader
- file download
- mobile archive
- research tools
- offline access

### Competitive Analysis
- No direct competitors for Internet Archive mobile access
- Position as essential tool for IA users
- Emphasize mobile-specific optimizations

### User Reviews Strategy
- Monitor reviews for feature requests
- Quick response to user issues
- Regular updates based on feedback

## Launch Strategy

### Soft Launch
1. Internal testing (alpha)
2. Closed beta with select users
3. Open beta for broader testing
4. Production release

### Post-Launch
- Weekly monitoring of crash reports
- Monthly feature updates
- Quarterly major releases
- Continuous performance optimization

This comprehensive asset package ensures a professional, successful app store launch.