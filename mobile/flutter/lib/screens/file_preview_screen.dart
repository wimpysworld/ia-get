import 'package:flutter/material.dart';
import 'package:http/http.dart' as http;
import 'dart:typed_data';
import '../models/archive_metadata.dart';

/// Screen for previewing files in memory without downloading
class FilePreviewScreen extends StatefulWidget {
  final ArchiveFile file;

  const FilePreviewScreen({super.key, required this.file});

  @override
  State<FilePreviewScreen> createState() => _FilePreviewScreenState();
}

class _FilePreviewScreenState extends State<FilePreviewScreen> {
  bool _isLoading = true;
  String? _error;
  Uint8List? _fileData;

  @override
  void initState() {
    super.initState();
    _loadFilePreview();
  }

  Future<void> _loadFilePreview() async {
    if (widget.file.downloadUrl == null) {
      setState(() {
        _error = 'No download URL available for this file';
        _isLoading = false;
      });
      return;
    }

    try {
      setState(() {
        _isLoading = true;
        _error = null;
      });

      // Check file size before downloading (limit preview to 10MB)
      final fileSize = widget.file.size ?? 0;
      if (fileSize > 10 * 1024 * 1024) {
        setState(() {
          _error = 'File too large for preview (${(fileSize / 1024 / 1024).toStringAsFixed(1)}MB). Maximum is 10MB.';
          _isLoading = false;
        });
        return;
      }

      // Fetch file data in memory with timeout
      final response = await http.get(Uri.parse(widget.file.downloadUrl!))
          .timeout(const Duration(seconds: 30));

      if (response.statusCode == 200) {
        setState(() {
          _fileData = response.bodyBytes;
          _isLoading = false;
        });
      } else {
        setState(() {
          _error = 'Failed to load file: HTTP ${response.statusCode}';
          _isLoading = false;
        });
      }
    } catch (e) {
      setState(() {
        _error = 'Error loading file: $e';
        _isLoading = false;
      });
    }
  }

  bool _isImageFormat() {
    if (widget.file.format == null) return false;
    final format = widget.file.format!.toLowerCase();
    return ['jpg', 'jpeg', 'png', 'gif', 'bmp', 'webp', 'svg'].contains(format);
  }

  bool _isTextFormat() {
    if (widget.file.format == null) return false;
    final format = widget.file.format!.toLowerCase();
    return ['txt', 'json', 'xml', 'html', 'htm', 'md', 'markdown', 'log', 
            'csv', 'yaml', 'yml', 'ini', 'conf', 'cfg'].contains(format);
  }

  bool _isVideoFormat() {
    if (widget.file.format == null) return false;
    final format = widget.file.format!.toLowerCase();
    return ['mp4', 'webm', 'mkv', 'avi', 'mov', 'flv', 'wmv', 'm4v'].contains(format);
  }

  bool _isAudioFormat() {
    if (widget.file.format == null) return false;
    final format = widget.file.format!.toLowerCase();
    return ['mp3', 'wav', 'flac', 'aac', 'm4a', 'ogg', 'wma', 'opus'].contains(format);
  }

  bool _isPDFFormat() {
    if (widget.file.format == null) return false;
    return widget.file.format!.toLowerCase() == 'pdf';
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: Text(widget.file.displayName),
        actions: [
          if (_fileData != null)
            IconButton(
              icon: const Icon(Icons.refresh),
              onPressed: _loadFilePreview,
              tooltip: 'Reload',
            ),
        ],
      ),
      body: _buildBody(),
    );
  }

  Widget _buildBody() {
    if (_isLoading) {
      return Center(
        child: Column(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            const CircularProgressIndicator(),
            const SizedBox(height: 16),
            Text(
              'Loading preview...',
              style: Theme.of(context).textTheme.bodyLarge,
            ),
          ],
        ),
      );
    }

    if (_error != null) {
      return Center(
        child: Padding(
          padding: const EdgeInsets.all(16),
          child: Column(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              const Icon(Icons.error_outline, size: 64, color: Colors.red),
              const SizedBox(height: 16),
              Text(
                _error!,
                textAlign: TextAlign.center,
                style: Theme.of(context).textTheme.bodyLarge,
              ),
              const SizedBox(height: 16),
              ElevatedButton(
                onPressed: _loadFilePreview,
                child: const Text('Retry'),
              ),
            ],
          ),
        ),
      );
    }

    if (_fileData == null) {
      return const Center(child: Text('No data available'));
    }

    // Display based on file type
    if (_isImageFormat()) {
      return _buildImagePreview();
    } else if (_isTextFormat()) {
      return _buildTextPreview();
    } else if (_isPDFFormat()) {
      return _buildPDFPreview();
    } else if (_isAudioFormat()) {
      return _buildAudioPreview();
    } else if (_isVideoFormat()) {
      return _buildVideoPreview();
    } else {
      return _buildUnsupportedPreview();
    }
  }

  Widget _buildImagePreview() {
    return InteractiveViewer(
      minScale: 0.5,
      maxScale: 4.0,
      child: Center(
        child: Image.memory(
          _fileData!,
          fit: BoxFit.contain,
          errorBuilder: (context, error, stackTrace) {
            return Center(
              child: Column(
                mainAxisAlignment: MainAxisAlignment.center,
                children: [
                  const Icon(Icons.broken_image, size: 64, color: Colors.grey),
                  const SizedBox(height: 16),
                  Text('Failed to load image: $error'),
                ],
              ),
            );
          },
        ),
      ),
    );
  }

  Widget _buildTextPreview() {
    try {
      final text = String.fromCharCodes(_fileData!);
      return SingleChildScrollView(
        padding: const EdgeInsets.all(16),
        child: SelectableText(
          text,
          style: const TextStyle(fontFamily: 'monospace', fontSize: 14),
        ),
      );
    } catch (e) {
      return Center(child: Text('Failed to decode text: $e'));
    }
  }

  Widget _buildVideoPreview() {
    return Center(
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          const Icon(Icons.video_library, size: 64, color: Colors.grey),
          const SizedBox(height: 16),
          Text(
            'Video preview loaded (${_formatSize(_fileData!.length)})',
            style: Theme.of(context).textTheme.bodyLarge,
          ),
          const SizedBox(height: 8),
          const Text(
            'Video playback requires additional dependencies.',
            textAlign: TextAlign.center,
            style: TextStyle(color: Colors.grey),
          ),
          const SizedBox(height: 16),
          const Padding(
            padding: EdgeInsets.all(16),
            child: Text(
              'Note: In-memory video playback is supported, but the video_player package needs to be integrated for full playback functionality.',
              textAlign: TextAlign.center,
              style: TextStyle(fontSize: 12, fontStyle: FontStyle.italic),
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildUnsupportedPreview() {
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            const Icon(Icons.description, size: 64, color: Colors.grey),
            const SizedBox(height: 16),
            Text(
              'Preview not supported for ${widget.file.format ?? "unknown"} format',
              textAlign: TextAlign.center,
              style: Theme.of(context).textTheme.bodyLarge,
            ),
            const SizedBox(height: 8),
            Text(
              'File loaded in memory (${_formatSize(_fileData!.length)})',
              style: const TextStyle(color: Colors.grey),
            ),
            const SizedBox(height: 16),
            const Text(
              'Supported formats:\n• Images: JPG, PNG, GIF, BMP, WebP, SVG\n• Text: TXT, JSON, XML, HTML, MD, CSV, YAML\n• PDF, Audio, Video (preview only)',
              textAlign: TextAlign.center,
              style: TextStyle(fontSize: 12),
            ),
          ],
        ),
      ),
    );
  }

  Widget _buildPDFPreview() {
    return Center(
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          const Icon(Icons.picture_as_pdf, size: 64, color: Colors.red),
          const SizedBox(height: 16),
          Text(
            'PDF preview loaded (${_formatSize(_fileData!.length)})',
            style: Theme.of(context).textTheme.bodyLarge,
          ),
          const SizedBox(height: 8),
          const Text(
            'PDF rendering requires additional package.',
            textAlign: TextAlign.center,
            style: TextStyle(color: Colors.grey),
          ),
          const SizedBox(height: 16),
          const Padding(
            padding: EdgeInsets.all(16),
            child: Text(
              'Note: PDF preview is available but requires the pdf_render or similar package for full rendering functionality.',
              textAlign: TextAlign.center,
              style: TextStyle(fontSize: 12, fontStyle: FontStyle.italic),
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildAudioPreview() {
    return Center(
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          const Icon(Icons.audiotrack, size: 64, color: Colors.blue),
          const SizedBox(height: 16),
          Text(
            'Audio file loaded (${_formatSize(_fileData!.length)})',
            style: Theme.of(context).textTheme.bodyLarge,
          ),
          const SizedBox(height: 8),
          Text(
            'Format: ${widget.file.format?.toUpperCase()}',
            style: const TextStyle(color: Colors.grey),
          ),
          const SizedBox(height: 16),
          const Padding(
            padding: EdgeInsets.all(16),
            child: Text(
              'Note: Audio playback is supported but requires the just_audio or audioplayers package for full playback functionality.',
              textAlign: TextAlign.center,
              style: TextStyle(fontSize: 12, fontStyle: FontStyle.italic),
            ),
          ),
        ],
      ),
    );
  }

  String _formatSize(int bytes) {
    const units = ['B', 'KB', 'MB', 'GB'];
    double size = bytes.toDouble();
    int unitIndex = 0;

    while (size >= 1024 && unitIndex < units.length - 1) {
      size /= 1024;
      unitIndex++;
    }

    return '${size.toStringAsFixed(size >= 100 ? 0 : 1)} ${units[unitIndex]}';
  }
}
