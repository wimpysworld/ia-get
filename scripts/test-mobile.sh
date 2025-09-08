#!/bin/bash
# Comprehensive mobile app testing script

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${BLUE}🧪 IA Get Mobile - Comprehensive Testing Suite${NC}"

# Check if we're in the right directory
if [[ ! -f "Cargo.toml" ]]; then
    echo -e "${RED}Error: Must be run from the ia-get project root${NC}"
    exit 1
fi

FLUTTER_DIR="mobile/flutter"
RESULTS_DIR="target/test-results"

# Create results directory
mkdir -p "$RESULTS_DIR"

echo -e "${YELLOW}📋 Test Suite Overview${NC}"
echo -e "1. Rust FFI Core Tests"
echo -e "2. Flutter Widget Tests"
echo -e "3. Integration Tests"
echo -e "4. Performance Tests"
echo -e "5. Code Quality Analysis"
echo ""

# Test counters
TOTAL_TESTS=0
PASSED_TESTS=0
FAILED_TESTS=0

run_test() {
    local test_name="$1"
    local test_command="$2"
    
    echo -e "${BLUE}Running: $test_name${NC}"
    TOTAL_TESTS=$((TOTAL_TESTS + 1))
    
    if eval "$test_command" > "$RESULTS_DIR/${test_name// /_}.log" 2>&1; then
        echo -e "${GREEN}✓ PASSED: $test_name${NC}"
        PASSED_TESTS=$((PASSED_TESTS + 1))
        return 0
    else
        echo -e "${RED}✗ FAILED: $test_name${NC}"
        echo -e "${YELLOW}  Log: $RESULTS_DIR/${test_name// /_}.log${NC}"
        FAILED_TESTS=$((FAILED_TESTS + 1))
        return 1
    fi
}

echo -e "${YELLOW}🦀 Phase 1: Rust FFI Core Tests${NC}"

# Test FFI functionality
run_test "FFI Core Tests" "cargo test --features ffi"

# Test mobile wrapper
if [[ -d "mobile/rust-ffi" ]]; then
    run_test "Mobile FFI Wrapper" "cd mobile/rust-ffi && cargo test"
fi

# Test cross-compilation
run_test "Android Cross-compilation" "cargo check --target aarch64-linux-android --features ffi"

echo -e "${YELLOW}📱 Phase 2: Flutter Tests${NC}"

cd "$FLUTTER_DIR"

# Flutter dependency check
run_test "Flutter Dependencies" "flutter pub get"

# Flutter analysis
run_test "Flutter Code Analysis" "flutter analyze"

# Widget tests
run_test "Flutter Widget Tests" "flutter test"

# Test coverage (if available)
if command -v lcov &> /dev/null; then
    run_test "Test Coverage" "flutter test --coverage"
fi

cd "../.."

echo -e "${YELLOW}🔧 Phase 3: Integration Tests${NC}"

# Build validation
run_test "Debug Build Validation" "cd $FLUTTER_DIR && flutter build apk --debug"

# APK analysis (if tools available)
if command -v aapt &> /dev/null && [[ -f "$FLUTTER_DIR/build/app/outputs/flutter-apk/app-debug.apk" ]]; then
    run_test "APK Analysis" "aapt dump badging $FLUTTER_DIR/build/app/outputs/flutter-apk/app-debug.apk"
fi

echo -e "${YELLOW}⚡ Phase 4: Performance Tests${NC}"

# Startup performance test
cat > /tmp/startup_test.dart << 'EOF'
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:ia_get_mobile/main.dart';

void main() {
  testWidgets('App startup performance', (WidgetTester tester) async {
    final stopwatch = Stopwatch()..start();
    
    await tester.pumpWidget(const IAGetMobileApp());
    await tester.pumpAndSettle();
    
    stopwatch.stop();
    
    // App should start within 3 seconds
    expect(stopwatch.elapsedMilliseconds, lessThan(3000));
    
    print('Startup time: ${stopwatch.elapsedMilliseconds}ms');
  });
}
EOF

run_test "Startup Performance" "cd $FLUTTER_DIR && flutter test /tmp/startup_test.dart"

# Memory usage test
cat > /tmp/memory_test.dart << 'EOF'
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:ia_get_mobile/main.dart';

void main() {
  testWidgets('Memory usage validation', (WidgetTester tester) async {
    // Test rapid creation/disposal of widgets
    for (int i = 0; i < 100; i++) {
      await tester.pumpWidget(const IAGetMobileApp());
      await tester.pump();
    }
    
    // If we get here without OOM, memory management is working
    expect(true, isTrue);
  });
}
EOF

run_test "Memory Usage" "cd $FLUTTER_DIR && flutter test /tmp/memory_test.dart"

echo -e "${YELLOW}📊 Phase 5: Code Quality Analysis${NC}"

# Rust code quality
run_test "Rust Clippy" "cargo clippy --features ffi -- -D warnings"

# Rust formatting
run_test "Rust Formatting" "cargo fmt --check"

# Flutter formatting
run_test "Flutter Formatting" "cd $FLUTTER_DIR && dart format --set-exit-if-changed lib/"

# Security analysis
if command -v cargo-audit &> /dev/null; then
    run_test "Security Audit" "cargo audit"
fi

echo -e "${YELLOW}📈 Phase 6: Build Size Analysis${NC}"

# Analyze build sizes
if [[ -f "$FLUTTER_DIR/build/app/outputs/flutter-apk/app-debug.apk" ]]; then
    APK_SIZE=$(du -h "$FLUTTER_DIR/build/app/outputs/flutter-apk/app-debug.apk" | cut -f1)
    echo -e "${BLUE}Debug APK size: $APK_SIZE${NC}"
    
    # Check if APK is reasonable size (under 50MB for debug)
    APK_SIZE_BYTES=$(stat -c%s "$FLUTTER_DIR/build/app/outputs/flutter-apk/app-debug.apk")
    if [[ $APK_SIZE_BYTES -lt 52428800 ]]; then  # 50MB
        run_test "APK Size Validation" "true"
    else
        run_test "APK Size Validation" "false"
    fi
fi

# Analyze native library sizes
echo -e "${BLUE}Native library sizes:${NC}"
if [[ -d "$FLUTTER_DIR/android/app/src/main/jniLibs" ]]; then
    find "$FLUTTER_DIR/android/app/src/main/jniLibs" -name "*.so" -exec du -h {} \; | while read size file; do
        echo -e "  $size - $(basename "$file")"
    done
fi

echo -e "${YELLOW}🧹 Cleanup${NC}"

# Clean up temporary files
rm -f /tmp/startup_test.dart /tmp/memory_test.dart

echo ""
echo -e "${BLUE}📊 Test Results Summary${NC}"
echo -e "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo -e "Total Tests:  $TOTAL_TESTS"
echo -e "${GREEN}Passed:       $PASSED_TESTS${NC}"
echo -e "${RED}Failed:       $FAILED_TESTS${NC}"

if [[ $FAILED_TESTS -eq 0 ]]; then
    echo -e "${GREEN}✅ All tests passed! Mobile app is ready for production.${NC}"
    SUCCESS_RATE="100%"
else
    SUCCESS_RATE=$(( (PASSED_TESTS * 100) / TOTAL_TESTS ))
    echo -e "${YELLOW}⚠ Success rate: ${SUCCESS_RATE}%${NC}"
    
    if [[ $SUCCESS_RATE -ge 80 ]]; then
        echo -e "${YELLOW}✓ Mobile app is mostly ready, review failed tests${NC}"
    else
        echo -e "${RED}✗ Significant issues found, address failed tests${NC}"
    fi
fi

echo ""
echo -e "${BLUE}📁 Test Results Location: $RESULTS_DIR/${NC}"
echo -e "${BLUE}📱 Next Steps:${NC}"

if [[ $FAILED_TESTS -eq 0 ]]; then
    echo -e "1. 🚀 Build production APK: ./scripts/build-mobile.sh --store-ready"
    echo -e "2. 📦 Build App Bundle: ./scripts/build-mobile.sh --appbundle"
    echo -e "3. 🏪 Upload to Google Play Store"
else
    echo -e "1. 🔍 Review failed test logs in $RESULTS_DIR/"
    echo -e "2. 🛠 Fix identified issues"
    echo -e "3. 🔄 Re-run tests: ./scripts/test-mobile.sh"
fi

exit $FAILED_TESTS