# Configuration File Documentation

## ia-get.conf Format

The configuration file uses TOML format and is located at:
- **Linux/macOS**: `~/.config/ia-get/ia-get.conf`
- **Windows**: `%APPDATA%\ia-get\ia-get.conf`

### Example Configuration

```toml
# Default download settings
default_output_path = "/home/user/Downloads/ia-get"
concurrent_downloads = 5
max_retries = 3
http_timeout = 60

# Default file filtering
default_include_ext = "pdf,txt,epub"
default_exclude_ext = "xml,log"
default_max_file_size = "100MB"

# Default behavior
default_resume = true
default_verbose = false
default_log_hash_errors = true
default_dry_run = false
default_compress = true
default_decompress = false

# Advanced settings
user_agent_override = "MyApp/1.0"
max_recent_urls = 20

# Filter presets
[[filter_presets]]
name = "Documents"
description = "Common document formats"
include_ext = "pdf,doc,docx,txt,rtf,odt"
max_file_size = "100MB"

[[filter_presets]]
name = "Images"
description = "Image files only"
include_ext = "jpg,jpeg,png,gif,bmp,tiff,webp"
max_file_size = "50MB"
```

### Configuration Keys

#### Download Settings
- `default_output_path` (string, optional): Default directory for downloads
- `concurrent_downloads` (integer, 1-20): Maximum concurrent downloads
- `max_retries` (integer, 0-50): Maximum retry attempts for failed downloads
- `http_timeout` (integer, 5-600): HTTP timeout in seconds

#### Filter Settings
- `default_include_ext` (string, optional): Comma-separated file extensions to include
- `default_exclude_ext` (string, optional): Comma-separated file extensions to exclude
- `default_min_file_size` (string, optional): Minimum file size (e.g., "1MB")
- `default_max_file_size` (string, optional): Maximum file size (e.g., "100MB")

#### Behavior Settings
- `default_resume` (boolean): Resume interrupted downloads by default
- `default_verbose` (boolean): Enable verbose output by default
- `default_log_hash_errors` (boolean): Log hash verification errors by default
- `default_dry_run` (boolean): Enable dry run mode by default
- `default_compress` (boolean): Enable HTTP compression by default
- `default_decompress` (boolean): Auto-decompress downloaded files by default
- `default_decompress_formats` (string, optional): Formats to auto-decompress

#### Advanced Settings
- `user_agent_override` (string, optional): Custom User-Agent string
- `max_recent_urls` (integer): Maximum number of recent URLs to remember

### Filter Presets

Filter presets allow you to save commonly used filter combinations:

```toml
[[filter_presets]]
name = "MyPreset"
description = "Description of preset"
include_ext = "pdf,txt"        # Optional
exclude_ext = "xml,log"        # Optional
max_file_size = "50MB"         # Optional
```

## Priority System

Configuration values are applied in the following priority order:

### CLI Mode
1. **Command line arguments** (highest priority)
2. **Configuration file** (`ia-get.conf`)
3. **Default values** (lowest priority)

### GUI Mode
1. **One-time GUI selections** (highest priority)
2. **Configuration file** (`ia-get.conf`)
3. **Default values** (lowest priority)

## CLI Configuration Commands

### View Configuration
```bash
# Show current configuration
ia-get config show

# Show configuration file location
ia-get config location
```

### Modify Configuration
```bash
# Set a value
ia-get config set concurrent_downloads 8
ia-get config set default_output_path "/path/to/downloads"
ia-get config set default_verbose true

# Remove a value (reset to default)
ia-get config unset concurrent_downloads

# Reset all configuration to defaults
ia-get config reset

# Validate current configuration
ia-get config validate
```

## Download History Database

Download history is stored in `ia-get-db.json` in the same directory as the configuration file.

### Database Structure

```json
{
  "version": "1.0.0",
  "created_at": "2025-01-01T00:00:00Z",
  "last_updated": "2025-01-01T12:00:00Z",
  "entries": [
    {
      "id": "archive-identifier-1234567890",
      "archive_identifier": "archive-name",
      "original_input": "https://archive.org/details/archive-name",
      "output_directory": "/path/to/output",
      "status": "Success",
      "started_at": "2025-01-01T10:00:00Z",
      "completed_at": "2025-01-01T10:05:00Z",
      "download_config": {
        "output_dir": "/path/to/output",
        "max_concurrent": 3,
        "format_filters": ["pdf", "txt"],
        "enable_compression": true,
        "verify_md5": true,
        "preserve_mtime": true
      },
      "total_files": 10,
      "completed_files": 10,
      "failed_files": 0,
      "bytes_downloaded": 1048576,
      "total_bytes": 1048576,
      "error_message": null,
      "metadata": {}
    }
  ],
  "max_entries": 1000
}
```

### History CLI Commands

```bash
# View recent downloads
ia-get history show

# View detailed history
ia-get history show --detailed

# View only failed downloads
ia-get history show --status failed

# View statistics
ia-get history stats

# Clear all history
ia-get history clear

# Remove specific entry
ia-get history remove <entry-id>
```

### Status Values

- `InProgress`: Download is currently running
- `Success`: Download completed successfully
- `Failed`: Download failed with error
- `Cancelled`: Download was cancelled by user
- `Paused`: Download was paused/interrupted

## Migration

When upgrading from older versions, configuration files are automatically migrated:
- `config.toml` → `ia-get.conf` (maintains TOML format)
- Settings and preferences are preserved
- Old files are removed after successful migration