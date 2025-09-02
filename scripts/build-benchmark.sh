#!/bin/bash
# Build benchmark script - measures compilation times for different configurations
set -e

echo "🏗️  Build Time Benchmark"
echo "========================"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Function to format time
format_time() {
    local seconds=$1
    # Convert to integer seconds and milliseconds
    local int_seconds=${seconds%.*}
    local decimal_part=${seconds#*.}
    # Pad decimal part to 3 digits
    local milliseconds=$(printf "%.3s" "${decimal_part}000")
    printf "%02d:%02d.%s" $((int_seconds/60)) $((int_seconds%60)) "$milliseconds"
}

# Function to run build and measure time
measure_build() {
    local name="$1"
    local cmd="$2"
    
    echo -e "${YELLOW}📋 $name${NC}"
    cargo clean > /dev/null 2>&1
    
    start_time=$(date +%s.%N)
    eval "$cmd" > /dev/null 2>&1
    end_time=$(date +%s.%N)
    
    duration=$(echo "$end_time - $start_time" | bc)
    formatted_time=$(format_time $duration)
    
    echo -e "${GREEN}✅ $name: $formatted_time${NC}"
    echo "$name,$duration" >> build_times.csv
}

# Function to check if bc is available
check_dependencies() {
    if ! command -v bc &> /dev/null; then
        echo -e "${RED}❌ Error: 'bc' command not found. Please install it to run benchmarks.${NC}"
        echo "On Ubuntu/Debian: sudo apt-get install bc"
        echo "On macOS: brew install bc"
        exit 1
    fi
}

check_dependencies

# Create CSV file for results
echo "Configuration,Time(seconds)" > build_times.csv

echo "Starting build time benchmarks..."
echo ""

# Benchmark different configurations
measure_build "CLI Only (dev)" "cargo build --no-default-features --features cli"
measure_build "CLI Only (release)" "cargo build --no-default-features --features cli --release"
measure_build "GUI (dev)" "cargo build --features gui"
measure_build "GUI (release)" "cargo build --features gui --release"
measure_build "Fast Dev Profile" "cargo build --profile fast-dev --no-default-features --features cli"

echo ""
echo "🧪 Test compilation benchmarks..."

measure_build "CLI Tests" "cargo test --no-default-features --features cli --no-run"
measure_build "GUI Tests" "cargo test --features gui --no-run"

echo ""
echo "📊 Results Summary:"
echo "==================="

# Read and display results
while IFS=',' read -r config time_seconds; do
    if [ "$config" != "Configuration" ]; then
        # Convert to integer seconds for display
        int_seconds=${time_seconds%.*}
        formatted_time=$(printf "%02d:%02d" $((int_seconds/60)) $((int_seconds%60)))
        echo -e "${GREEN}$config: ${formatted_time}${NC}"
    fi
done < build_times.csv

echo ""
echo "💡 Optimization recommendations:"
echo "- Use 'cargo build --no-default-features --features cli' for CLI-only development"
echo "- Use '--profile fast-dev' for fastest iteration during development"
echo "- Full GUI builds are only needed when modifying GUI components"

echo ""
echo "📄 Detailed results saved to: build_times.csv"