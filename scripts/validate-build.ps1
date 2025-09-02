# Build validation script for Windows
# Catches issues early in the development process

Write-Host "🔍 Build Validation Script" -ForegroundColor Cyan
Write-Host "===========================" -ForegroundColor Cyan

# Function to print status
function Write-Status {
    param([string]$Status, [string]$Message)
    switch ($Status) {
        "success" { Write-Host "✅ $Message" -ForegroundColor Green }
        "warning" { Write-Host "⚠️  $Message" -ForegroundColor Yellow }
        "error" { Write-Host "❌ $Message" -ForegroundColor Red }
    }
}

Write-Host "📋 Checking prerequisites..." -ForegroundColor Blue

# Check required tools
try {
    $null = Get-Command cargo -ErrorAction Stop
    Write-Status "success" "Cargo found"
} catch {
    Write-Status "error" "Cargo not found. Please install Rust."
    exit 1
}

try {
    $null = Get-Command rustc -ErrorAction Stop
    Write-Status "success" "Rustc found"
} catch {
    Write-Status "error" "Rustc not found. Please install Rust."
    exit 1
}

Write-Host ""
Write-Host "📋 Step 1: Checking code formatting..." -ForegroundColor Blue
try {
    & cargo fmt --check --all 2>$null
    if ($LASTEXITCODE -eq 0) {
        Write-Status "success" "Code formatting is correct"
    } else {
        Write-Status "error" "Code formatting issues found. Run 'cargo fmt' to fix."
        Write-Host "💡 Tip: Run 'cargo fmt' to automatically fix formatting issues" -ForegroundColor Cyan
        exit 1
    }
} catch {
    Write-Status "error" "Failed to check formatting: $_"
    exit 1
}

Write-Host ""
Write-Host "📋 Step 2: Running clippy (CLI features)..." -ForegroundColor Blue
try {
    & cargo clippy --no-default-features --features cli --all-targets -- -D warnings 2>$null
    if ($LASTEXITCODE -eq 0) {
        Write-Status "success" "Clippy (CLI) passed with no warnings"
    } else {
        Write-Status "error" "Clippy (CLI) found issues"
        exit 1
    }
} catch {
    Write-Status "error" "Failed to run clippy (CLI): $_"
    exit 1
}

Write-Host ""
Write-Host "📋 Step 3: Running clippy (GUI features)..." -ForegroundColor Blue
try {
    & cargo clippy --features gui --all-targets -- -D warnings 2>$null
    if ($LASTEXITCODE -eq 0) {
        Write-Status "success" "Clippy (GUI) passed with no warnings"
    } else {
        Write-Status "error" "Clippy (GUI) found issues"
        exit 1
    }
} catch {
    Write-Status "error" "Failed to run clippy (GUI): $_"
    exit 1
}

Write-Host ""
Write-Host "📋 Step 4: Checking compilation (CLI)..." -ForegroundColor Blue
try {
    & cargo check --no-default-features --features cli 2>$null
    if ($LASTEXITCODE -eq 0) {
        Write-Status "success" "CLI compilation successful"
    } else {
        Write-Status "error" "CLI compilation failed"
        exit 1
    }
} catch {
    Write-Status "error" "Failed to check CLI compilation: $_"
    exit 1
}

Write-Host ""
Write-Host "📋 Step 5: Checking compilation (GUI)..." -ForegroundColor Blue
try {
    & cargo check --features gui 2>$null
    if ($LASTEXITCODE -eq 0) {
        Write-Status "success" "GUI compilation successful"
    } else {
        Write-Status "error" "GUI compilation failed"
        exit 1
    }
} catch {
    Write-Status "error" "Failed to check GUI compilation: $_"
    exit 1
}

Write-Host ""
Write-Host "📋 Step 6: Running tests (CLI)..." -ForegroundColor Blue
try {
    & cargo test --no-default-features --features cli --quiet 2>$null
    if ($LASTEXITCODE -eq 0) {
        Write-Status "success" "CLI tests passed"
    } else {
        Write-Status "error" "CLI tests failed"
        exit 1
    }
} catch {
    Write-Status "error" "Failed to run CLI tests: $_"
    exit 1
}

Write-Host ""
Write-Host "📋 Step 7: Running tests (GUI)..." -ForegroundColor Blue
try {
    & cargo test --features gui --quiet 2>$null
    if ($LASTEXITCODE -eq 0) {
        Write-Status "success" "GUI tests passed"
    } else {
        Write-Status "error" "GUI tests failed"
        exit 1
    }
} catch {
    Write-Status "error" "Failed to run GUI tests: $_"
    exit 1
}

Write-Host ""
Write-Host "📋 Step 8: Checking for security vulnerabilities..." -ForegroundColor Blue
try {
    $null = Get-Command cargo-audit -ErrorAction Stop
    & cargo audit --quiet 2>$null | Out-Null
    if ($LASTEXITCODE -eq 0) {
        Write-Status "success" "Security audit passed"
    } else {
        Write-Status "warning" "Security vulnerabilities found (non-blocking)"
    }
} catch {
    Write-Status "warning" "cargo-audit not installed. Install with: cargo install cargo-audit"
}

Write-Host ""
Write-Host "📋 Step 9: Checking for outdated dependencies..." -ForegroundColor Blue
try {
    $null = Get-Command cargo-outdated -ErrorAction Stop
    $result = & cargo outdated --quiet 2>$null
    if ($LASTEXITCODE -eq 0) {
        Write-Status "success" "No major dependency updates available"
    } else {
        Write-Status "warning" "Outdated dependencies found (non-blocking)"
    }
} catch {
    Write-Status "warning" "cargo-outdated not installed. Install with: cargo install cargo-outdated"
}

Write-Host ""
Write-Host "🎉 Build validation completed successfully!" -ForegroundColor Green
Write-Host ""
Write-Host "📊 Summary:" -ForegroundColor Cyan
Write-Host "   - Code formatting: ✅" -ForegroundColor Green
Write-Host "   - Clippy (CLI): ✅" -ForegroundColor Green
Write-Host "   - Clippy (GUI): ✅" -ForegroundColor Green
Write-Host "   - Compilation (CLI): ✅" -ForegroundColor Green
Write-Host "   - Compilation (GUI): ✅" -ForegroundColor Green
Write-Host "   - Tests (CLI): ✅" -ForegroundColor Green
Write-Host "   - Tests (GUI): ✅" -ForegroundColor Green
Write-Host ""
Write-Host "💡 Your code is ready for commit and CI/CD pipeline!" -ForegroundColor Cyan
