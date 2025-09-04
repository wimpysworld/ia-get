# File Format Categories Implementation

This document describes the implementation of predefined file format categories for the ia-get CLI tool, addressing issue #26.

## Overview

The implementation provides a comprehensive system for filtering Internet Archive downloads by predefined file format categories, making it easier for users to download specific types of content without manually specifying individual file extensions.

## Features Implemented

### 1. Core Format Categories Module (`src/file_formats.rs`)

- **9 predefined categories** with 200+ file extensions total:
  - Documents: PDF, eBooks, office documents (27 formats)
  - Images: Photos, graphics, artwork (22 formats)  
  - Audio: Music, recordings, podcasts (18 formats)
  - Video: Movies, TV shows, clips (19 formats)
  - Software: Applications, code, installers (23 formats)
  - Data: Datasets, databases, structured data (17 formats)
  - Web: Web pages, sites, archives (14 formats)
  - Archives: Compressed files, backups (24 formats)
  - Metadata: Archive metadata, checksums (15 formats)

- **Smart categorization** with priority-based lookup for overlapping formats
- **Format validation** and suggestion functionality
- **Common presets** for quick selection

### 2. Enhanced CLI Interface

- **New command-line options**:
  - `--include-formats <CATEGORIES>`: Include files by category
  - `--exclude-formats <CATEGORIES>`: Exclude files by category  
  - `--list-formats`: List available categories
  - `--list-formats-detailed`: Show detailed format information

- **Backward compatibility** with existing `--include` and `--exclude` options
- **Combined filtering** supports both categories and manual extensions

### 3. Interactive CLI Enhancements (`src/interactive_cli.rs`)

- **Enhanced format filtering wizard** with 4 options:
  1. Use predefined format categories
  2. Manually specify file extensions
  3. Use both categories and manual extensions
  4. Skip format filtering

- **Category selection interface** with numbered options and name-based selection
- **Common presets** integration for quick setup

### 4. GUI Enhancements (`src/gui/panels/filters.rs`)

- **Visual category checkboxes** with emoji icons and descriptions
- **Include/exclude toggles** for each category
- **Quick preset buttons**:
  - "Documents Only": Select only document formats
  - "Media Only": Select images, audio, video
  - "No Metadata": Exclude metadata files
  - "Clear Categories": Reset all selections

- **Combined filtering** merges category selections with manual format entry

### 5. Format Help System (`src/format_help.rs`)

- **Comprehensive help displays** for format categories
- **Usage examples** with real command-line scenarios
- **Format preset explanations** 
- **Complete format listing** with detailed information

### 6. Enhanced Filtering Logic (`src/filters.rs`)

- **Updated FilterOptions trait** with resolved extension methods
- **Category-aware filtering** that combines manual and category-based extensions
- **Priority-based format resolution** for overlapping categories

## Usage Examples

### Command Line Interface

```bash
# List available format categories
ia-get --list-formats

# Download only documents  
ia-get --include-formats documents https://archive.org/details/example

# Download images and audio, exclude metadata
ia-get --include-formats images,audio --exclude-formats metadata https://archive.org/details/example

# Combine categories with manual extensions
ia-get --include-formats documents --exclude-ext log,tmp https://archive.org/details/example

# Get detailed format information
ia-get --list-formats-detailed
```

### Interactive CLI

The interactive mode now offers a format filtering wizard:

1. Choose filtering method (categories, manual, both, or skip)
2. Select categories by number or name  
3. Configure manual extensions if desired
4. Apply combined filters

### GUI Interface

The GUI filters panel includes:

- Format category checkboxes with visual icons
- Include/exclude options for each category
- Quick preset buttons for common scenarios
- Real-time filter summary display

## Technical Implementation Details

### Format Priority System

When a format appears in multiple categories, the system uses a priority order:

1. Metadata (most specific)
2. Web (web-specific files)  
3. Software (code/applications)
4. Data (structured data)
5. Archives (compression)
6. Documents, Images, Audio, Video (content types)

### Backward Compatibility

- All existing CLI options continue to work unchanged
- Existing configuration files remain compatible
- Manual extension filtering works alongside category filtering
- No breaking changes to the API

### Testing

Comprehensive test coverage includes:

- Format categorization accuracy
- Category priority resolution
- Filter combination logic
- Preset functionality
- Suggestion algorithms
- Edge cases and error handling

## Benefits

1. **Ease of Use**: Users can select broad categories instead of memorizing file extensions
2. **Comprehensive Coverage**: 200+ predefined formats based on common Internet Archive content
3. **Flexibility**: Combine category and manual filtering as needed
4. **Discoverability**: Built-in help system shows available options
5. **Consistency**: Same filtering system across CLI, interactive mode, and GUI

## Future Enhancements

- User-defined custom categories
- Format statistics and recommendations
- Integration with Internet Archive collection metadata
- Advanced filtering with format combinations
- Export/import of filter presets

This implementation fully addresses issue #26 by providing an intuitive, comprehensive system for file format filtering while maintaining full backward compatibility with existing functionality.