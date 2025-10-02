#!/bin/bash
# Script to generate Android app icons from ia-helper.svg
# Generates PNG icons for all densities and vector drawable for adaptive icon

set -e

# Color codes for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Get script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Source and target paths
SOURCE_SVG="$PROJECT_ROOT/assets/ia-helper.svg"
ANDROID_RES="$PROJECT_ROOT/mobile/flutter/android/app/src/main/res"

echo -e "${GREEN}Generating Android icons from ia-helper.svg${NC}"

# Check if source SVG exists
if [ ! -f "$SOURCE_SVG" ]; then
    echo -e "${RED}Error: Source SVG not found at $SOURCE_SVG${NC}"
    exit 1
fi

# Check for required tools
if ! command -v inkscape &> /dev/null; then
    echo -e "${RED}Error: inkscape is not installed${NC}"
    echo "Install it with: sudo apt-get install inkscape"
    exit 1
fi

if ! command -v convert &> /dev/null; then
    echo -e "${RED}Error: ImageMagick convert is not installed${NC}"
    echo "Install it with: sudo apt-get install imagemagick"
    exit 1
fi

echo -e "${YELLOW}Generating PNG icons for different densities...${NC}"

# Define icon sizes for different densities
# Android launcher icon sizes
declare -A DENSITIES=(
    ["mdpi"]=48
    ["hdpi"]=72
    ["xhdpi"]=96
    ["xxhdpi"]=144
    ["xxxhdpi"]=192
)

# Generate PNG icons for each density
for density in "${!DENSITIES[@]}"; do
    size=${DENSITIES[$density]}
    output_dir="$ANDROID_RES/mipmap-$density"
    output_file="$output_dir/ic_launcher.png"
    
    echo "  Generating $density icon (${size}x${size}px)..."
    
    # Create directory if it doesn't exist
    mkdir -p "$output_dir"
    
    # Export SVG to PNG with Inkscape
    inkscape "$SOURCE_SVG" \
        --export-type=png \
        --export-filename="$output_file" \
        --export-width=$size \
        --export-height=$size \
        --export-background-opacity=1.0 \
        2>/dev/null
    
    echo -e "    ${GREEN}✓${NC} Created: $output_file"
done

# Generate high-res icon for Play Store (1024x1024)
echo -e "${YELLOW}Generating Play Store icon (1024x1024px)...${NC}"
PLAY_STORE_ICON="$PROJECT_ROOT/assets/ia-helper_1024.png"
inkscape "$SOURCE_SVG" \
    --export-type=png \
    --export-filename="$PLAY_STORE_ICON" \
    --export-width=1024 \
    --export-height=1024 \
    --export-background-opacity=1.0 \
    2>/dev/null
echo -e "  ${GREEN}✓${NC} Created: $PLAY_STORE_ICON"

# Generate medium-res icon (512x512)
echo -e "${YELLOW}Generating 512x512 icon...${NC}"
ICON_512="$PROJECT_ROOT/assets/ia-helper_512.png"
inkscape "$SOURCE_SVG" \
    --export-type=png \
    --export-filename="$ICON_512" \
    --export-width=512 \
    --export-height=512 \
    --export-background-opacity=1.0 \
    2>/dev/null
echo -e "  ${GREEN}✓${NC} Created: $ICON_512"

# Generate standard icon for general use
echo -e "${YELLOW}Generating standard icon (128x128)...${NC}"
ICON_STANDARD="$PROJECT_ROOT/assets/ia-helper.png"
inkscape "$SOURCE_SVG" \
    --export-type=png \
    --export-filename="$ICON_STANDARD" \
    --export-width=128 \
    --export-height=128 \
    --export-background-opacity=1.0 \
    2>/dev/null
echo -e "  ${GREEN}✓${NC} Created: $ICON_STANDARD"

echo -e "${YELLOW}Generating adaptive icon vector drawables...${NC}"

# Generate the foreground vector drawable
# We need to extract the path from the SVG and convert it to Android vector format
FOREGROUND_XML="$ANDROID_RES/drawable/ic_launcher_foreground.xml"

echo "  Creating adaptive icon foreground..."

# Create the adaptive icon foreground XML
# This uses the same SVG path data but formatted for Android
# NOTE: Foreground should NOT include a background - that's what the background layer is for
cat > "$FOREGROUND_XML" << 'EOF'
<?xml version="1.0" encoding="utf-8"?>
<vector xmlns:android="http://schemas.android.com/apk/res/android"
    android:width="108dp"
    android:height="108dp"
    android:viewportWidth="512"
    android:viewportHeight="512">
    <!-- Internet Archive icon from ia-helper.svg -->
    <!-- Foreground layer: only the building icon, no background -->
    <!-- Background layer provides the white background separately -->
    
    <!-- Internet Archive building (columns, pediment, steps) -->
    <path
        android:fillColor="#000000"
        android:pathData="M 81,419 
                         L 431,419 
                         L 431,437 
                         L 81,437 
                         Z 
                         M 95,385 
                         L 418,385 
                         L 418,410 
                         L 95,410 
                         Z 
                         M 93,101 
                         L 414,101 
                         L 414,136 
                         L 93,136 
                         Z 
                         M 412,91 
                         L 422,80 
                         L 253,41 
                         L 85,80 
                         L 95,91 
                         L 253,91 
                         Z 
                         M 139,245 
                         L 138,194 
                         L 135,147 
                         C 135,145 135,145 133,145 
                         A 67,67 0 0 0 105,145 
                         C 104,145 103,145 103,147 
                         L 101,194 
                         A 2223,2223 0 0 0 101,321 
                         L 103,364 
                         L 104,372 
                         L 119,375 
                         C 125,374 130,374 135,372 
                         L 136,364 
                         L 138,321 
                         A 1616,1616 0 0 0 139,245 
                         Z 
                         M 227,245 
                         L 225,194 
                         L 223,147 
                         C 223,145 222,145 221,145 
                         A 67,67 0 0 0 193,145 
                         C 191,145 191,145 191,147 
                         L 188,194 
                         A 2223,2223 0 0 0 188,321 
                         L 190,364 
                         L 191,372 
                         C 196,374 202,374 207,375 
                         L 223,372 
                         L 223,364 
                         L 225,321 
                         A 1620,1620 0 0 0 227,245 
                         Z 
                         M 328,245 
                         L 327,194 
                         L 324,147 
                         C 324,145 324,145 322,145 
                         A 67,67 0 0 0 294,145 
                         C 293,145 292,145 292,147 
                         L 290,194 
                         A 2223,2223 0 0 0 290,321 
                         L 292,364 
                         L 293,372 
                         L 308,375 
                         C 313,374 319,374 324,372 
                         L 325,364 
                         L 327,321 
                         A 1624,1624 0 0 0 328,245 
                         Z 
                         M 413,245 
                         L 412,194 
                         L 410,147 
                         C 410,145 409,145 408,145 
                         A 67,67 0 0 0 379,145 
                         L 378,147 
                         L 375,194 
                         A 2227,2227 0 0 0 375,321 
                         L 377,364 
                         L 378,372 
                         C 383,374 388,374 394,375 
                         L 409,372 
                         L 410,364 
                         L 412,321 
                         A 1620,1620 0 0 0 413,245 
                         Z" />
</vector>
EOF

echo -e "  ${GREEN}✓${NC} Created: $FOREGROUND_XML"

# Background remains white (as it is in the original)
BACKGROUND_XML="$ANDROID_RES/drawable/ic_launcher_background.xml"
echo "  Creating adaptive icon background..."

cat > "$BACKGROUND_XML" << 'EOF'
<?xml version="1.0" encoding="utf-8"?>
<vector xmlns:android="http://schemas.android.com/apk/res/android"
    android:width="108dp"
    android:height="108dp"
    android:viewportWidth="108"
    android:viewportHeight="108">
    <!-- White background for adaptive icon -->
    <path
        android:fillColor="#FFFFFF"
        android:pathData="M0,0 L108,0 L108,108 L0,108 Z" />
</vector>
EOF

echo -e "  ${GREEN}✓${NC} Created: $BACKGROUND_XML"

# The adaptive-icon XML doesn't need to change, but let's ensure it exists
ADAPTIVE_ICON_XML="$ANDROID_RES/mipmap-anydpi-v26/ic_launcher.xml"
mkdir -p "$(dirname "$ADAPTIVE_ICON_XML")"

echo "  Creating adaptive icon configuration..."

cat > "$ADAPTIVE_ICON_XML" << 'EOF'
<?xml version="1.0" encoding="utf-8"?>
<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">
    <background android:drawable="@drawable/ic_launcher_background"/>
    <foreground android:drawable="@drawable/ic_launcher_foreground"/>
    <!-- Monochrome icon for themed icons (Android 13+) -->
    <monochrome android:drawable="@drawable/ic_launcher_foreground"/>
</adaptive-icon>
EOF

echo -e "  ${GREEN}✓${NC} Created: $ADAPTIVE_ICON_XML"

echo ""
echo -e "${GREEN}✓ All Android icons generated successfully!${NC}"
echo ""
echo "Generated files:"
echo "  • PNG icons in: $ANDROID_RES/mipmap-*/"
echo "  • Vector drawables in: $ANDROID_RES/drawable/"
echo "  • Adaptive icon config in: $ANDROID_RES/mipmap-anydpi-v26/"
echo "  • Play Store icon: $PLAY_STORE_ICON"
echo ""
echo "You can now build the Android app with:"
echo "  cd mobile/flutter"
echo "  flutter build apk"
