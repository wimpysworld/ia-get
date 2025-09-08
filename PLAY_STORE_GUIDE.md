# Google Play Store Submission Guide

This guide covers the complete process for submitting the IA Get Android app to the Google Play Store.

## Prerequisites

### 1. Google Play Console Account
- Create a Google Play Console developer account ($25 one-time fee)
- Set up your developer profile and payment methods

### 2. App Signing Setup
The app uses Google Play App Signing for security and easier key management:

```bash
# Generate upload keystore (do this once)
keytool -genkey -v -keystore upload-keystore.jks -keyalg RSA -keysize 2048 -validity 10000 -alias upload
```

Create `mobile/flutter/android/key.properties`:
```properties
uploadKeyStore=upload-keystore.jks
uploadKeyAlias=upload
uploadStorePassword=your-secure-password
uploadKeyPassword=your-secure-password
```

### 3. Build Store-Ready App Bundle

```bash
# Production App Bundle for Play Store
./scripts/build-mobile.sh --production --appbundle --store-ready

# Alternative: All variants
./scripts/build-mobile.sh --production --store-ready  # APK + split APKs
```

## Store Listing Requirements

### App Information
- **App Name**: IA Get
- **Package Name**: `com.gameaday.ia_get_mobile`
- **Category**: Tools
- **Content Rating**: Everyone (pending content review)

### Store Listing Assets Required

#### Icons & Graphics
- **App Icon**: 512×512 PNG (already configured)
- **Feature Graphic**: 1024×500 PNG
- **Screenshots**: 
  - Phone: 2-8 screenshots (16:9 or 9:16 ratio)
  - Tablet: 1-8 screenshots (optional but recommended)

#### Descriptions
- **Short Description** (80 chars max):
  "Download files from Internet Archive - books, movies, music & more"

- **Full Description** (4000 chars max):
  See template below

### Privacy Policy
Required for apps that handle user data. Create and host a privacy policy covering:
- Data collection (download history, preferences)
- Storage permissions usage
- Network usage for Internet Archive access

## Submission Process

### 1. Create App in Play Console
1. Go to [Google Play Console](https://play.google.com/console)
2. Click "Create app"
3. Fill in basic app information
4. Select "App Bundle" as the release format

### 2. Upload App Bundle
1. Go to "Release" → "Production"
2. Click "Create new release"
3. Upload `target/mobile/ia-get-mobile-production.aab`
4. Fill in release notes

### 3. Complete Store Listing
1. **Main store listing**: Descriptions, graphics, categorization
2. **Content rating**: Complete IARC questionnaire
3. **Target audience**: Age groups and family-friendly status
4. **Privacy & security**: Privacy policy, data collection disclosure
5. **App content**: Declarations about app functionality

### 4. Set Up Release Management
1. **Country availability**: Select target countries
2. **Pricing**: Free (current plan)
3. **In-app products**: None (current plan)

### 5. Review and Publish
1. Complete all required sections
2. Submit for review
3. Monitor review process (typically 1-3 days)

## Store Listing Content Templates

### Short Description
```
Download files from Internet Archive - books, movies, music & more
```

### Full Description
```
Access millions of free resources from the Internet Archive with IA Get - the fastest way to download books, movies, music, software, and historical documents directly to your Android device.

📚 WHAT YOU CAN ACCESS
• Books: Classic literature, textbooks, research papers
• Movies: Public domain films, documentaries, educational content  
• Music: Live concerts, podcasts, historical recordings
• Software: Vintage games, applications, operating systems
• Data: Research datasets, government documents, web archives

🚀 KEY FEATURES
• Fast, reliable downloads from archive.org
• Support for all file types and formats
• Background downloading with progress notifications
• Direct integration with Internet Archive URLs
• Offline access to downloaded content
• Clean, intuitive interface designed for mobile

🔒 PRIVACY & SECURITY
• No account required - anonymous browsing and downloading
• All data stored locally on your device
• No tracking or data collection
• Open source and transparent

🌍 ABOUT INTERNET ARCHIVE
The Internet Archive is a non-profit organization dedicated to providing universal access to human knowledge. With over 735 billion web pages, 41 million books, and millions of other items, it's one of the world's largest digital libraries.

Perfect for students, researchers, educators, and anyone interested in exploring human knowledge and culture.

Download IA Get today and unlock the world's digital heritage!
```

### Release Notes Template
```
🎉 Initial Release - v1.6.0

Features:
• Fast downloads from Internet Archive
• Support for all file types  
• Background downloading
• URL integration
• Offline access

Experience the full Internet Archive library on your Android device!
```

## Post-Launch Checklist

### Immediate (Week 1)
- [ ] Monitor crash reports in Play Console
- [ ] Respond to user reviews
- [ ] Check download metrics and user feedback

### Short Term (Month 1)
- [ ] Gather user feedback for improvements
- [ ] Plan feature updates based on usage patterns
- [ ] Optimize based on performance metrics

### Ongoing
- [ ] Regular app updates with new features
- [ ] Monitor Play Store policy changes
- [ ] Maintain compatibility with new Android versions

## Build Variants for Different Stages

```bash
# Development testing
./scripts/build-mobile.sh --dev

# Internal testing (small group)
./scripts/build-mobile.sh --staging --appbundle

# Production release
./scripts/build-mobile.sh --production --appbundle --store-ready
```

## Troubleshooting Common Issues

### Upload Issues
- **Bundle too large**: Check if native libraries are optimized
- **Missing permissions**: Verify AndroidManifest.xml permissions
- **Signing errors**: Check keystore configuration

### Review Rejections
- **Content policy**: Ensure compliance with Play Store content policies
- **Privacy policy**: Make sure privacy policy is accessible and complete
- **Permissions**: Justify all requested permissions in store listing

### Technical Issues
- **Crashes**: Monitor Play Console crash reports
- **Performance**: Use Play Console vitals to identify issues
- **Compatibility**: Test on various devices and Android versions

## Resources

- [Google Play Console](https://play.google.com/console)
- [Play Store Policy](https://support.google.com/googleplay/android-developer/answer/10787469)
- [App Bundle Guide](https://developer.android.com/guide/app-bundle)
- [Play Console Help](https://support.google.com/googleplay/android-developer)