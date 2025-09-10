#!/bin/bash
# Local CI test script - simulates the CI workflow locally

set -e

echo "🚀 Starting local CI simulation..."
echo "=================================="

echo "📋 Step 1: Check formatting..."
cargo fmt --check
echo "✅ Formatting check passed"

echo "📋 Step 2: Run clippy linting..."
cargo clippy --no-default-features --features cli --all-targets -- -D warnings
echo "✅ Clippy check passed"

echo "📋 Step 3: Build project (using optimized CI profile)..."
cargo build --profile ci --no-default-features --features cli --verbose
echo "✅ CI build successful"

echo "📋 Step 4: Build release binary..."
cargo build --release --no-default-features --features cli
echo "✅ Release build successful"

echo "📋 Step 5: Test binary..."
./target/release/ia-get --version
echo "✅ Binary test passed"

echo "📋 Step 6: Run tests..."
cargo test --no-default-features --features cli --quiet
echo "✅ Tests passed"

echo "📋 Step 7: Create artifact..."
mkdir -p artifacts
PROJECT_NAME="ia-get"
TARGET="x86_64-unknown-linux-gnu"
COMMIT_SHA=$(git rev-parse --short HEAD)
PACKAGE_NAME="ia-get-${COMMIT_SHA}-${TARGET}"

cp "target/release/${PROJECT_NAME}" "artifacts/${PROJECT_NAME}-${TARGET}"
cd artifacts
zip "${PACKAGE_NAME}.zip" "${PROJECT_NAME}-${TARGET}"

# Calculate SHA256 hashes
sha256sum "${PACKAGE_NAME}.zip" > "${PACKAGE_NAME}.zip.sha256"
cd ..
echo "✅ Artifact created: ${PACKAGE_NAME}.zip"
echo "✅ SHA256 hash: $(cat "artifacts/${PACKAGE_NAME}.zip.sha256" | cut -d' ' -f1)"

echo ""
echo "🎉 Local CI simulation completed successfully!"
echo "📦 Artifact location: artifacts/${PACKAGE_NAME}.zip"
echo "📊 Binary size: $(du -h artifacts/${PROJECT_NAME}-${TARGET} | cut -f1)"
echo "📊 Archive size: $(du -h artifacts/${PACKAGE_NAME}.zip | cut -f1)"