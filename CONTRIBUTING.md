# Contributing to Internet Archive Helper

We welcome contributions from developers, researchers, and Internet Archive enthusiasts! Whether you're fixing bugs, adding features, improving documentation, or helping with translations, your help makes Internet Archive content more accessible to everyone.

## 🚀 Quick Start

1. **Fork the repository** on GitHub
2. **Clone your fork** locally
3. **Set up the development environment**
4. **Make your changes**
5. **Test thoroughly**
6. **Submit a pull request**

## 🛠️ Development Setup

### Prerequisites

**Required Versions:**
- **Rust**: Latest stable (1.75.0 or higher)
- **Flutter**: 3.27.1 or higher (for mobile development)
- **Dart**: 3.8.0 or higher (included with Flutter 3.27.1+)
- **Android SDK**: API 33+ (for mobile development)
- **Android NDK**: 26.1.10909125 or compatible (for mobile development)
- **Java**: JDK 17 (for Android builds)

**Need help with setup?** See **[TROUBLESHOOTING.md](TROUBLESHOOTING.md)** for detailed instructions.

### Rust CLI/GUI Development

```bash
# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Clone the repository
git clone https://github.com/Gameaday/ia-get-cli.git
cd ia-get-cli

# Build CLI version (fastest for development)
cargo build --no-default-features --features cli

# Build with GUI support
cargo build --features gui

# Run tests
cargo test --no-default-features --features cli

# Check formatting and linting
cargo fmt --check
cargo clippy --no-default-features --features cli -- -D warnings
```

### Mobile App Development

**Important**: Make sure you have Flutter 3.27.1+ installed. If you encounter dependency issues, run:
```bash
./scripts/fix-flutter-deps.sh
```

```bash
# Install Flutter SDK (version 3.27.1 or higher required)
# See: https://flutter.dev/docs/get-started/install

# Install Android SDK and NDK
# Set ANDROID_HOME and ANDROID_NDK_HOME environment variables

# Navigate to Flutter app
cd mobile/flutter

# Get dependencies
flutter pub get

# Run on Android device/emulator
flutter run

# Build APK
flutter build apk

# Run Flutter analyzer
flutter analyze
```

## 📝 Code Style Guidelines

### Rust Code
- Follow standard Rust conventions and idioms
- Use `cargo fmt` for consistent formatting
- Ensure `cargo clippy` passes without warnings
- Write comprehensive tests for new functionality
- Use meaningful variable and function names
- Add documentation comments for public APIs

### Flutter/Dart Code
- Follow [Dart style guide](https://dart.dev/guides/language/effective-dart/style)
- Use `flutter format` for consistent formatting
- Ensure `flutter analyze` passes without issues
- Follow Material 3 design principles
- Use meaningful widget names and structure
- Add widget tests for UI components

### Kotlin Code
- Follow [Kotlin coding conventions](https://kotlinlang.org/docs/coding-conventions.html)
- Use consistent indentation (4 spaces)
- Use meaningful class and method names
- Add KDoc comments for public APIs

## 🐛 Bug Reports

When reporting bugs, please include:

- **Description**: Clear description of the issue
- **Steps to reproduce**: Detailed steps to reproduce the problem
- **Expected behavior**: What you expected to happen
- **Actual behavior**: What actually happened
- **Environment**: OS, version, device details
- **Screenshots**: If applicable, add screenshots
- **Logs**: Include relevant error messages or logs

## ✨ Feature Requests

We love new ideas! When suggesting features:

- **Use case**: Explain why this feature would be useful
- **Description**: Detailed description of the proposed feature
- **Mockups**: If UI changes, include mockups or sketches
- **Alternatives**: Describe any alternative solutions considered
- **Internet Archive alignment**: How does this support the Internet Archive mission?

## 🔧 Types of Contributions

### Code Contributions
- **Bug fixes**: Fix existing issues
- **New features**: Add functionality that benefits users
- **Performance improvements**: Optimize speed, memory usage, or battery life
- **Code cleanup**: Refactor code for better maintainability
- **Test coverage**: Add missing tests

### Documentation Contributions
- **README updates**: Improve setup and usage instructions
- **Code comments**: Add or improve inline documentation
- **Help content**: Enhance in-app help and onboarding
- **Guides**: Create tutorials or how-to guides

### Design Contributions
- **UI/UX improvements**: Enhance user interface and experience
- **Accessibility**: Improve app accessibility for all users
- **Icons and graphics**: Create or improve visual assets
- **Material 3**: Ensure compliance with latest design standards

### Translation Contributions
- **Localization**: Help translate the app to other languages
- **Cultural adaptation**: Adapt content for different regions
- **Testing**: Test translated versions for accuracy

## 🔍 Code Review Process

1. **Automated checks**: All PRs must pass automated tests and linting
2. **Manual review**: Core maintainers will review code for quality and fit
3. **Testing**: Changes should be tested thoroughly
4. **Documentation**: Update documentation if needed
5. **Feedback**: Address any review comments
6. **Approval**: PRs need approval from at least one maintainer

## 📱 Mobile App Contributions

For mobile app contributions:

- **Material 3**: Follow Material 3 design principles
- **Accessibility**: Ensure features are accessible
- **Performance**: Test on various devices and Android versions
- **Permissions**: Minimize permission requests
- **Privacy**: Maintain our privacy-first approach

## 🌟 Recognition

Contributors are recognized in:
- **Release notes**: Major contributions mentioned in releases
- **Contributors section**: Listed in project documentation
- **Commit history**: Your commits become part of the project history

## 📞 Getting Help

Need help contributing?

- **GitHub Discussions**: Ask questions and get help
- **Issues**: Browse existing issues for contribution ideas
- **Email**: Contact maintainers for significant contributions

## 📄 License

By contributing to Internet Archive Helper, you agree that your contributions will be licensed under the same license as the project.

## 🤝 Code of Conduct

We are committed to providing a welcoming and inclusive environment for all contributors. Please be:

- **Respectful**: Treat everyone with respect and professionalism
- **Inclusive**: Welcome contributors from all backgrounds
- **Constructive**: Provide helpful feedback and suggestions
- **Collaborative**: Work together towards common goals
- **Patient**: Remember that everyone is learning

## 🎯 Project Goals

Keep these goals in mind when contributing:

- **Accessibility**: Make Internet Archive content more accessible
- **Privacy**: Maintain user privacy and data protection
- **Performance**: Ensure fast, efficient downloads and browsing
- **Reliability**: Create stable, dependable software
- **Community**: Support the Internet Archive community

---

**Thank you for contributing to Internet Archive Helper!** 

Your efforts help preserve and provide access to human knowledge for everyone. 📚✨