#!/bin/bash
# Local CI test script - simulates the CI workflow locally

set -e

echo "🚀 Starting local CI simulation..."
echo "=================================="

echo "📋 Step 1: Check formatting..."
cargo fmt --check
echo "✅ Formatting check passed"

echo "📋 Step 2: Run clippy linting..."
cargo clippy --bin ia-get --lib -- -D warnings
echo "✅ Clippy check passed"

echo "📋 Step 3: Build project..."
cargo build --verbose
echo "✅ Build successful"

echo "📋 Step 4: Build release binary..."
cargo build --release
echo "✅ Release build successful"

echo "📋 Step 5: Test binary..."
./target/release/ia-get --version
echo "✅ Binary test passed"

echo "📋 Step 6: Create artifact..."
mkdir -p artifacts
PROJECT_NAME="ia-get"
TARGET="x86_64-unknown-linux-gnu"
COMMIT_SHA=$(git rev-parse --short HEAD)
PACKAGE_NAME="ia-get-${COMMIT_SHA}-${TARGET}"

cp "target/release/${PROJECT_NAME}" "artifacts/${PROJECT_NAME}-${TARGET}"
cd artifacts
tar czf "${PACKAGE_NAME}.tar.gz" "${PROJECT_NAME}-${TARGET}"

# Calculate SHA256 hashes
sha256sum "${PACKAGE_NAME}.tar.gz" > "${PACKAGE_NAME}.tar.gz.sha256"
cd ..
echo "✅ Artifact created: ${PACKAGE_NAME}.tar.gz"
echo "✅ SHA256 hash: $(cat "artifacts/${PACKAGE_NAME}.tar.gz.sha256" | cut -d' ' -f1)"

echo ""
echo "🎉 Local CI simulation completed successfully!"
echo "📦 Artifact location: artifacts/${PACKAGE_NAME}.tar.gz"
echo "📊 Binary size: $(du -h artifacts/${PROJECT_NAME}-${TARGET} | cut -f1)"
echo "📊 Archive size: $(du -h artifacts/${PACKAGE_NAME}.tar.gz | cut -f1)"