# Advanced CLI Guide for ia-get

## Overview

This guide covers advanced usage patterns for the ia-get CLI tool, optimized for servers and power users.

## Terminal-First Philosophy

The ia-get CLI is designed with a terminal-first approach:
- **No GUI dependencies** - Works perfectly in headless environments
- **Scriptable** - All operations can be automated
- **Server-optimized** - Minimal resource usage, efficient batch operations
- **Power user features** - Advanced filtering, search, and batch operations

## Core CLI Features

### Basic Download
```bash
# Download an entire archive
ia-get <identifier>

# Specify output directory
ia-get <identifier> --output /path/to/downloads

# Dry run to see what would be downloaded
ia-get <identifier> --dry-run
```

### Concurrent Downloads
```bash
# Use 8 parallel downloads (default is 4)
ia-get <identifier> --concurrent 8

# Maximum 16 concurrent downloads supported
ia-get <identifier> -c 16
```

### File Filtering
```bash
# Include only specific formats
ia-get <identifier> --include pdf --include epub

# Use format categories
ia-get <identifier> --include-formats documents,images

# Exclude certain formats
ia-get <identifier> --exclude-formats metadata

# Filter by file size
ia-get <identifier> --max-size 100MB

# Combine filters
ia-get <identifier> --include-formats documents --max-size 50MB
```

### Source Type Filtering
```bash
# Original files only (excludes derivatives)
ia-get <identifier> --original-only

# Include derivatives (thumbnails, different formats)
ia-get <identifier> --include-derivatives

# Include metadata files
ia-get <identifier> --include-metadata

# Specify exact source types
ia-get <identifier> --source-types original,derivative
```

## Advanced Features

### Search Functionality
```bash
# Search Internet Archive (future feature)
ia-get search "vintage computers" --limit 20

# Filter search by media type
ia-get search "nasa missions" --mediatype movies

# Filter by year range
ia-get search "documentaries" --year 1970-1980

# Sort results
ia-get search "classic books" --sort downloads
```

### Batch Operations
```bash
# Download multiple archives from a file (future feature)
ia-get batch identifiers.txt

# Parallel batch processing
ia-get batch urls.txt --parallel 3

# Batch with specific output directory
ia-get batch list.txt --output ./downloads

# Resume interrupted batch operations
ia-get batch list.txt --resume
```

### Configuration Management
```bash
# Show current configuration
ia-get config --show

# Set configuration values
ia-get config --set concurrent_downloads=8
ia-get config --set default_output_path=/data/archives

# Show configuration file location
ia-get config --location

# Reset to defaults
ia-get config --reset

# Validate configuration
ia-get config --validate
```

### Download History
```bash
# View download history
ia-get history --show

# Limit number of entries
ia-get history --show --limit 10

# Filter by status
ia-get history --show --status completed

# Show detailed information
ia-get history --show --detailed

# View statistics
ia-get history --stats

# Clear history
ia-get history --clear

# Remove specific entry
ia-get history --remove <id>
```

### API Health Monitoring
```bash
# Check Internet Archive API health
ia-get --api-health

# Analyze metadata for an archive
ia-get --analyze-metadata <identifier>
```

## Server and Automation Use Cases

### Headless Server Operation
```bash
# Completely non-interactive
ia-get --headless <identifier> --output /data

# Log to file
ia-get <identifier> --log-file /var/log/ia-get.log

# JSON progress output for parsing
ia-get <identifier> --progress-format json > progress.json
```

### Systemd Service
Create `/etc/systemd/system/ia-get-downloader.service`:
```ini
[Unit]
Description=Internet Archive Downloader
After=network-online.target
Wants=network-online.target

[Service]
Type=oneshot
User=ia-downloader
WorkingDirectory=/data/archives
ExecStart=/usr/local/bin/ia-get batch /etc/ia-get/download-list.txt --output /data/archives
StandardOutput=journal
StandardError=journal
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

### Cron Jobs
```bash
# Daily download of specific archives
0 2 * * * /usr/local/bin/ia-get batch /home/user/daily-archives.txt --output /data/archives 2>&1 | logger -t ia-get

# Weekly metadata updates
0 3 * * 0 /usr/local/bin/ia-get --analyze-metadata my-archive >> /var/log/ia-get-analysis.log
```

### Scripting Examples

**Download with error handling:**
```bash
#!/bin/bash
set -e

ARCHIVE_ID="$1"
OUTPUT_DIR="/data/archives"
LOG_FILE="/var/log/ia-get-$(date +%Y%m%d).log"

echo "Starting download of ${ARCHIVE_ID}" >> "${LOG_FILE}"

if ia-get "${ARCHIVE_ID}" --output "${OUTPUT_DIR}" 2>> "${LOG_FILE}"; then
    echo "Successfully downloaded ${ARCHIVE_ID}" >> "${LOG_FILE}"
    # Send success notification
    echo "Archive ${ARCHIVE_ID} downloaded successfully" | mail -s "Download Success" admin@example.com
else
    echo "Failed to download ${ARCHIVE_ID}" >> "${LOG_FILE}"
    # Send failure notification
    echo "Archive ${ARCHIVE_ID} download failed. Check logs." | mail -s "Download Failed" admin@example.com
    exit 1
fi
```

**Batch download with progress monitoring:**
```bash
#!/bin/bash

BATCH_FILE="archives.txt"
TOTAL=$(wc -l < "${BATCH_FILE}")
CURRENT=0

while IFS= read -r identifier; do
    CURRENT=$((CURRENT + 1))
    echo "[${CURRENT}/${TOTAL}] Downloading: ${identifier}"
    
    if ia-get "${identifier}" --output /data/archives; then
        echo "${identifier}" >> completed.txt
    else
        echo "${identifier}" >> failed.txt
    fi
done < "${BATCH_FILE}"

echo "Batch download complete!"
echo "Successful: $(wc -l < completed.txt)"
echo "Failed: $(wc -l < failed.txt)"
```

## Performance Tuning

### Network Optimization
```bash
# Disable compression for faster downloads on slow CPUs
ia-get <identifier> --no-compress

# Enable compression to save bandwidth
ia-get <identifier>  # Compression enabled by default
```

### Resource Management
```bash
# Limit concurrent downloads to reduce load
ia-get <identifier> -c 2

# Increase for faster downloads (if bandwidth allows)
ia-get <identifier> -c 16
```

### Disk Space Management
```bash
# Check space before downloading
ia-get <identifier> --dry-run  # Shows what will be downloaded

# Filter by size to fit available space
ia-get <identifier> --max-size 500MB
```

## Troubleshooting

### Common Issues

**Downloads fail or timeout:**
```bash
# Increase retries
ia-get <identifier> --max-retries 5

# Check API health first
ia-get --api-health
```

**Rate limiting:**
```bash
# The CLI automatically handles rate limiting
# It will wait and retry when rate limited

# Check current rate limit status
ia-get --api-health
```

**Disk space issues:**
```bash
# Preview download size
ia-get <identifier> --dry-run

# Use size filtering
ia-get <identifier> --max-size 1GB
```

### Debug Mode
```bash
# Enable verbose output
ia-get <identifier> --verbose

# Log to file for analysis
ia-get <identifier> --verbose --log-file debug.log
```

## Exit Codes

The CLI uses standard exit codes for scripting:
- `0` - Success
- `1` - General error
- `2` - Invalid arguments
- `3` - Network error
- `4` - Disk error (no space, permission denied)
- `5` - Archive not found
- `6` - Rate limited (temporary)

## Best Practices

1. **Always use `--dry-run` first** for large archives
2. **Set appropriate concurrent downloads** based on your network
3. **Use configuration file** for consistent settings
4. **Monitor API health** before large batch operations
5. **Log operations** for auditing and debugging
6. **Handle errors** in scripts with proper exit code checking
7. **Respect rate limits** - the CLI does this automatically
8. **Use filters** to download only what you need
9. **Test in dev environment** before production deployments
10. **Keep CLI updated** for latest features and fixes

## Additional Resources

- **GitHub Repository**: https://github.com/Gameaday/ia-get-cli
- **Internet Archive API**: https://archive.org/developers/
- **Issue Tracker**: https://github.com/Gameaday/ia-get-cli/issues
- **Releases**: https://github.com/Gameaday/ia-get-cli/releases

## Contributing

The ia-get CLI is open source and welcomes contributions:
- Report bugs and request features via GitHub Issues
- Submit pull requests for improvements
- Share your use cases and automation scripts
- Help improve documentation

## License

This project is licensed under the GPL-3.0 license - see the LICENSE file for details.
