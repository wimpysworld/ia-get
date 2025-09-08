// Flutter integration example for ia-get FFI
// This demonstrates how the Rust FFI would be used in a Flutter app

import 'dart:ffi';
import 'dart:typed_data';
import 'package:ffi/ffi.dart';

// FFI function signatures matching the Rust interface
typedef IaGetInitC = Int32 Function();
typedef IaGetInit = int Function();

typedef IaGetFetchMetadataC = Int32 Function(
  Pointer<Utf8> identifier,
  Pointer<NativeFunction<ProgressCallbackC>> progressCallback,
  Pointer<NativeFunction<CompletionCallbackC>> completionCallback,
  Pointer<Void> userData,
);
typedef IaGetFetchMetadata = int Function(
  Pointer<Utf8> identifier,
  Pointer<NativeFunction<ProgressCallbackC>> progressCallback,
  Pointer<NativeFunction<CompletionCallbackC>> completionCallback,
  Pointer<Void> userData,
);

typedef ProgressCallbackC = Void Function(Double progress, Pointer<Utf8> message, Pointer<Void> userData);
typedef CompletionCallbackC = Void Function(Bool success, Pointer<Utf8> errorMessage, Pointer<Void> userData);

typedef IaGetFilterFilesC = Pointer<Utf8> Function(
  Pointer<Utf8> metadataJson,
  Pointer<Utf8> includeFormats,
  Pointer<Utf8> excludeFormats,
  Pointer<Utf8> maxSizeStr,
);
typedef IaGetFilterFiles = Pointer<Utf8> Function(
  Pointer<Utf8> metadataJson,
  Pointer<Utf8> includeFormats,
  Pointer<Utf8> excludeFormats,
  Pointer<Utf8> maxSizeStr,
);

typedef IaGetFreeStringC = Void Function(Pointer<Utf8> ptr);
typedef IaGetFreeString = void Function(Pointer<Utf8> ptr);

class IaGetNative {
  static const String _libName = 'ia_get';
  static late final DynamicLibrary _dylib = _openDylib();
  
  static DynamicLibrary _openDylib() {
    if (Platform.isAndroid) {
      return DynamicLibrary.open('lib$_libName.so');
    } else if (Platform.isIOS) {
      return DynamicLibrary.process();
    } else {
      throw UnsupportedError('Unsupported platform');
    }
  }

  // FFI function bindings
  static final IaGetInit _iaGetInit = _dylib
      .lookup<NativeFunction<IaGetInitC>>('ia_get_init')
      .asFunction();

  static final IaGetFetchMetadata _iaGetFetchMetadata = _dylib
      .lookup<NativeFunction<IaGetFetchMetadataC>>('ia_get_fetch_metadata')
      .asFunction();

  static final IaGetFilterFiles _iaGetFilterFiles = _dylib
      .lookup<NativeFunction<IaGetFilterFilesC>>('ia_get_filter_files')
      .asFunction();

  static final IaGetFreeString _iaGetFreeString = _dylib
      .lookup<NativeFunction<IaGetFreeStringC>>('ia_get_free_string')
      .asFunction();

  // Progress callback wrapper
  static void _progressCallback(double progress, Pointer<Utf8> message, Pointer<Void> userData) {
    final callback = _callbackMap[userData.address];
    if (callback != null && message != nullptr) {
      final messageStr = message.toDartString();
      callback(progress, messageStr);
    }
  }

  // Completion callback wrapper
  static void _completionCallback(bool success, Pointer<Utf8> errorMessage, Pointer<Void> userData) {
    final callback = _completionCallbackMap[userData.address];
    if (callback != null) {
      String? error;
      if (!success && errorMessage != nullptr) {
        error = errorMessage.toDartString();
      }
      callback(success, error);
    }
  }

  // Callback storage (in real implementation, use proper cleanup)
  static final Map<int, Function(double, String)> _callbackMap = {};
  static final Map<int, Function(bool, String?)> _completionCallbackMap = {};
  
  static int _nextCallbackId = 1;

  /// Initialize the native library
  static int init() {
    return _iaGetInit();
  }

  /// Fetch archive metadata
  static Future<Map<String, dynamic>?> fetchMetadata(
    String identifier, {
    required Function(double progress, String message) onProgress,
    required Function(bool success, String? error) onComplete,
  }) async {
    final identifierPtr = identifier.toNativeUtf8();
    final callbackId = _nextCallbackId++;
    
    _callbackMap[callbackId] = onProgress;
    _completionCallbackMap[callbackId] = onComplete;
    
    final progressCallbackPtr = Pointer.fromFunction<ProgressCallbackC>(_progressCallback);
    final completionCallbackPtr = Pointer.fromFunction<CompletionCallbackC>(_completionCallback);
    final userDataPtr = Pointer<Void>.fromAddress(callbackId);

    final requestId = _iaGetFetchMetadata(
      identifierPtr,
      progressCallbackPtr,
      completionCallbackPtr,
      userDataPtr,
    );

    malloc.free(identifierPtr);
    
    return requestId > 0 ? {} : null; // Return placeholder data
  }

  /// Filter files based on criteria
  static List<Map<String, dynamic>> filterFiles(
    Map<String, dynamic> metadata, {
    List<String>? includeFormats,
    List<String>? excludeFormats,
    String? maxSize,
  }) {
    final metadataJson = jsonEncode(metadata);
    final metadataPtr = metadataJson.toNativeUtf8();
    
    final includePtr = includeFormats?.join(',').toNativeUtf8() ?? nullptr;
    final excludePtr = excludeFormats?.join(',').toNativeUtf8() ?? nullptr;
    final maxSizePtr = maxSize?.toNativeUtf8() ?? nullptr;

    final resultPtr = _iaGetFilterFiles(metadataPtr, includePtr, excludePtr, maxSizePtr);
    
    List<Map<String, dynamic>> result = [];
    if (resultPtr != nullptr) {
      final resultJson = resultPtr.toDartString();
      try {
        final decoded = jsonDecode(resultJson);
        if (decoded is List) {
          result = decoded.cast<Map<String, dynamic>>();
        }
      } catch (e) {
        print('Failed to parse filter results: $e');
      }
      
      _iaGetFreeString(resultPtr);
    }

    // Cleanup
    malloc.free(metadataPtr);
    if (includePtr != nullptr) malloc.free(includePtr);
    if (excludePtr != nullptr) malloc.free(excludePtr);
    if (maxSizePtr != nullptr) malloc.free(maxSizePtr);

    return result;
  }
}

// Example Flutter widget using the FFI
class ArchiveBrowserWidget extends StatefulWidget {
  final String archiveIdentifier;

  const ArchiveBrowserWidget({Key? key, required this.archiveIdentifier}) : super(key: key);

  @override
  _ArchiveBrowserWidgetState createState() => _ArchiveBrowserWidgetState();
}

class _ArchiveBrowserWidgetState extends State<ArchiveBrowserWidget> {
  Map<String, dynamic>? _metadata;
  List<Map<String, dynamic>> _filteredFiles = [];
  bool _isLoading = false;
  double _progress = 0.0;
  String _statusMessage = '';
  String _errorMessage = '';
  
  // Filter state
  List<String> _selectedFormats = [];
  String _maxSizeFilter = '';

  @override
  void initState() {
    super.initState();
    IaGetNative.init();
    _fetchMetadata();
  }

  void _fetchMetadata() async {
    setState(() {
      _isLoading = true;
      _progress = 0.0;
      _statusMessage = 'Starting...';
      _errorMessage = '';
    });

    final metadata = await IaGetNative.fetchMetadata(
      widget.archiveIdentifier,
      onProgress: (progress, message) {
        setState(() {
          _progress = progress;
          _statusMessage = message;
        });
      },
      onComplete: (success, error) {
        setState(() {
          _isLoading = false;
          if (!success) {
            _errorMessage = error ?? 'Unknown error occurred';
          } else {
            _statusMessage = 'Metadata loaded successfully';
            _applyFilters();
          }
        });
      },
    );

    if (metadata != null) {
      setState(() {
        _metadata = metadata;
      });
    }
  }

  void _applyFilters() {
    if (_metadata == null) return;

    final filtered = IaGetNative.filterFiles(
      _metadata!,
      includeFormats: _selectedFormats.isNotEmpty ? _selectedFormats : null,
      maxSize: _maxSizeFilter.isNotEmpty ? _maxSizeFilter : null,
    );

    setState(() {
      _filteredFiles = filtered;
    });
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: Text('Archive: ${widget.archiveIdentifier}'),
      ),
      body: Column(
        children: [
          // Progress indicator
          if (_isLoading) ...[
            LinearProgressIndicator(value: _progress),
            Padding(
              padding: const EdgeInsets.all(8.0),
              child: Text(_statusMessage),
            ),
          ],
          
          // Error display
          if (_errorMessage.isNotEmpty)
            Container(
              width: double.infinity,
              padding: const EdgeInsets.all(16.0),
              color: Colors.red.shade100,
              child: Text(
                _errorMessage,
                style: TextStyle(color: Colors.red.shade800),
              ),
            ),

          // Filter controls
          if (_metadata != null && !_isLoading) ...[
            ExpansionTile(
              title: const Text('Filters'),
              children: [
                // Format filter
                Wrap(
                  children: ['pdf', 'mp3', 'jpg', 'txt', 'zip'].map((format) {
                    return FilterChip(
                      label: Text(format.toUpperCase()),
                      selected: _selectedFormats.contains(format),
                      onSelected: (selected) {
                        setState(() {
                          if (selected) {
                            _selectedFormats.add(format);
                          } else {
                            _selectedFormats.remove(format);
                          }
                        });
                        _applyFilters();
                      },
                    );
                  }).toList(),
                ),
                
                // Size filter
                Padding(
                  padding: const EdgeInsets.all(8.0),
                  child: TextField(
                    decoration: const InputDecoration(
                      labelText: 'Max file size (e.g., 10MB, 1GB)',
                      border: OutlineInputBorder(),
                    ),
                    onChanged: (value) {
                      _maxSizeFilter = value;
                      _applyFilters();
                    },
                  ),
                ),
              ],
            ),
          ],

          // File list
          Expanded(
            child: _filteredFiles.isEmpty
                ? const Center(child: Text('No files to display'))
                : ListView.builder(
                    itemCount: _filteredFiles.length,
                    itemBuilder: (context, index) {
                      final file = _filteredFiles[index];
                      return ListTile(
                        leading: Icon(_getFileIcon(file['format'] ?? '')),
                        title: Text(file['name'] ?? 'Unknown'),
                        subtitle: Text(_formatFileSize(file['size'] ?? 0)),
                        trailing: IconButton(
                          icon: const Icon(Icons.download),
                          onPressed: () => _downloadFile(file),
                        ),
                      );
                    },
                  ),
          ),
        ],
      ),
    );
  }

  IconData _getFileIcon(String format) {
    switch (format.toLowerCase()) {
      case 'pdf':
        return Icons.picture_as_pdf;
      case 'mp3':
      case 'wav':
      case 'flac':
        return Icons.audiotrack;
      case 'jpg':
      case 'png':
      case 'gif':
        return Icons.image;
      case 'mp4':
      case 'avi':
      case 'mkv':
        return Icons.video_file;
      case 'zip':
      case 'tar':
      case 'gz':
        return Icons.archive;
      default:
        return Icons.insert_drive_file;
    }
  }

  String _formatFileSize(int bytes) {
    if (bytes < 1024) return '$bytes B';
    if (bytes < 1024 * 1024) return '${(bytes / 1024).toStringAsFixed(1)} KB';
    if (bytes < 1024 * 1024 * 1024) return '${(bytes / (1024 * 1024)).toStringAsFixed(1)} MB';
    return '${(bytes / (1024 * 1024 * 1024)).toStringAsFixed(1)} GB';
  }

  void _downloadFile(Map<String, dynamic> file) {
    // Implementation would start download using FFI
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(content: Text('Downloading ${file['name']}...')),
    );
  }
}