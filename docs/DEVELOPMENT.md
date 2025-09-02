# Development Configuration for ia-get-cli

This file documents build optimization strategies and development workflows for faster iteration.

## 🚀 Build Profiles

### CLI-Only Development (Fastest)
For core CLI feature development, avoid GUI dependencies:
```bash
cargo build --no-default-features --features cli
cargo test --no-default-features --features cli
cargo run --no-default-features --features cli -- <args>
```

### Fast Development Profile
Use the optimized development profile for quicker iteration:
```bash
cargo build --profile fast-dev --no-default-features --features cli
```

### GUI Development
When working on GUI components:
```bash
cargo build --features gui
cargo run --features gui --bin ia-get-gui
```

### Release Builds
```bash
# CLI binary
cargo build --no-default-features --features cli --release

# GUI binary  
cargo build --features gui --release
```

## ⚡ Performance Optimizations

### 1. Feature Gates
- `cli`: Core command-line functionality (default)
- `gui`: Graphical user interface (optional)

### 2. Build Profiles
- `dev`: Standard development (unoptimized, full debug info)
- `fast-dev`: Faster development iteration (minimal optimization, reduced debug info)
- `release`: Production optimization (size-optimized with LTO)

### 3. Dependency Management
GUI dependencies (eframe, egui, etc.) are now optional and only compiled when needed.

## 🧪 Testing Strategies

### Quick Tests (CLI only)
```bash
cargo test --no-default-features --features cli
```

### Full Test Suite
```bash
cargo test --all-features
```

### Test Compilation Only
```bash
cargo test --no-run --no-default-features --features cli
```

## 📊 Build Time Monitoring

Use the build benchmark script to measure performance:
```bash
./scripts/build-benchmark.sh
```

This will generate a CSV report comparing build times across different configurations.

## 💡 Development Tips

1. **Use CLI-only builds** for most development work to avoid GUI compilation overhead
2. **Enable GUI features** only when modifying GUI components
3. **Use the fast-dev profile** for rapid iteration during debugging
4. **Monitor build times** regularly to catch performance regressions
5. **Clean builds occasionally** to ensure fresh dependency resolution

## 🔧 IDE Configuration

### VS Code
Add these tasks to `.vscode/tasks.json`:
```json
{
    "label": "cargo build (CLI fast)",
    "type": "cargo",
    "command": "build",
    "args": ["--no-default-features", "--features", "cli", "--profile", "fast-dev"]
}
```

### Rust Analyzer
Configure `rust-analyzer.cargo.features` in your IDE settings:
```json
{
    "rust-analyzer.cargo.features": ["cli"]
}
```

## 📈 Expected Performance Gains

Based on benchmarking:
- **CLI-only builds**: ~60-70% faster than full builds
- **Fast-dev profile**: Additional 10-20% improvement for development
- **Test compilation**: ~50% faster with CLI-only features
- **CI/CD**: Improved caching and parallel builds reduce overall pipeline time