# ia-get GUI Documentation

## Overview

The ia-get GUI provides a cross-platform graphical interface for downloading files from the Internet Archive. It wraps all the functionality of the CLI version in an easy-to-use graphical interface.

## Running the GUI

To start the GUI application:

```bash
cargo run --bin ia-get-gui
```

Or if you have built the project:

```bash
./target/debug/ia-get-gui    # Linux/macOS
./target/debug/ia-get-gui.exe    # Windows
```

## GUI Features

### Main Interface

The GUI provides a tabbed interface with four main sections:

1. **Download Tab** - Main download interface
2. **Filters Tab** - File filtering configuration  
3. **Settings Tab** - Application configuration
4. **History Tab** - Recent download history

### Download Tab

- **Archive Identifier/URL**: Enter an Internet Archive identifier (e.g., `commute_test`) or full URL
- **Output Directory**: Select where files will be downloaded (with browse button)
- **Download Options**:
  - Verbose output
  - Resume downloads
  - Dry run (preview only)
- **Concurrent Downloads**: Slider to control parallel downloads (1-16)
- **Download Button**: Start the download process
- **Progress Display**: Real-time progress tracking with file counts and speed

### Filters Tab

- **Quick Presets**: Pre-configured filter sets for common file types
  - Documents (pdf, doc, txt, etc.)
  - Images (jpg, png, gif, etc.)
  - Audio (mp3, wav, flac, etc.)
  - Video (mp4, avi, mkv, etc.)
- **Manual Filters**:
  - Include formats (comma-separated)
  - Exclude formats (comma-separated)
  - Minimum file size
  - Maximum file size
- **Filter Summary**: Shows currently active filters

### Settings Tab

- **Download Settings**:
  - Max concurrent downloads (1-20)
  - Max retries (1-10)
  - HTTP timeout (5-300 seconds)
- **Default Behavior**:
  - Resume downloads by default
  - Verbose output by default
  - Log hash errors by default
  - Dry run mode by default
- **Advanced Settings**:
  - Custom User Agent override
- **Filter Presets Management**:
  - Add new presets
  - Remove existing presets
- **Configuration Actions**:
  - Save configuration
  - Reset to defaults

### History Tab

- **Recent Downloads**: List of previously downloaded archives
- **Quick Actions**:
  - Click folder icon to reload an archive identifier
  - Click X to remove from history
- **Clear History**: Remove all history entries

## Feature Parity with CLI

The GUI provides complete feature parity with the CLI version:

| CLI Feature | GUI Location | Notes |
|-------------|--------------|-------|
| `<IDENTIFIER>` | Download Tab | Archive Identifier/URL input |
| `-o, --output` | Download Tab | Output directory with browse button |
| `-v, --verbose` | Download Tab | Verbose output checkbox |
| `--dry-run` | Download Tab | Dry run checkbox |
| `-c, --concurrent` | Download Tab | Concurrent downloads slider |
| `-i, --include` | Filters Tab | Include formats field |
| `--max-size` | Filters Tab | Max file size field |
| `--no-compress` | Settings Tab | Advanced settings |
| `--decompress` | Settings Tab | Advanced settings |
| `--decompress-formats` | Settings Tab | Advanced settings |

## Configuration

The GUI uses the same configuration system as the CLI:

- Configuration file: `~/.config/ia-get/config.toml` (Linux/macOS) or `%APPDATA%\ia-get\config.toml` (Windows)
- Settings are automatically saved when modified in the GUI
- Configuration can be shared between CLI and GUI versions

## Error Handling

The GUI provides user-friendly error messages with:

- **Network Errors**: Suggestions for connectivity issues
- **File System Errors**: Guidance on disk space and permissions
- **Configuration Errors**: Validation feedback for settings

## Progress Tracking

Real-time download progress includes:

- Current file being downloaded
- Files completed/total files
- Failed file count
- Download speed (MB/s)
- Estimated time remaining
- Overall progress percentage

## Cross-Platform Support

The GUI is built with egui and supports:

- **Windows**: Native window with system integration
- **macOS**: Native macOS app with menu bar integration  
- **Linux**: GTK-based interface with desktop integration

## Keyboard Shortcuts

- **Ctrl/Cmd + Q**: Quit application
- **Tab**: Navigate between interface elements
- **Enter**: Activate default button (Download)
- **Escape**: Cancel dialogs

## Tips

1. **Bulk Downloads**: Use filter presets to quickly configure common download types
2. **Resume Support**: Interrupted downloads can be resumed by running the same download again
3. **Dry Run**: Always test filters with dry run before downloading large archives
4. **Concurrent Downloads**: Higher concurrency is faster but may be rate-limited by archive.org
5. **History**: Use the history tab to quickly re-download or check previous archives

## Troubleshooting

### GUI Won't Start
- Ensure all dependencies are installed: `cargo build --bin ia-get-gui`
- Check that display/graphics drivers are properly configured

### Downloads Fail
- Check internet connectivity
- Verify archive identifier is correct
- Try reducing concurrent downloads
- Check available disk space

### Configuration Issues
- Reset to defaults in Settings tab
- Delete configuration file to start fresh
- Check file permissions on config directory

## Integration with CLI

The GUI and CLI share the same:

- Configuration files
- Session data for resume functionality
- Download history
- Core download engine

You can start a download in the GUI and resume it from the CLI or vice versa.