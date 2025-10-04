/*
 * ia-get Simplified FFI C Header
 * 
 * This header provides C bindings for the simplified FFI interface.
 * Only 6 core functions for stateless operations.
 * 
 * IMPORTANT: All functions are thread-safe. Error messages are stored
 * in thread-local storage.
 * 
 * Memory Management:
 * - Strings returned by ia_get_fetch_metadata() and ia_get_decompress_file()
 *   MUST be freed with ia_get_free_string()
 * - The string returned by ia_get_last_error() must NOT be freed
 * 
 * Version: 1.6.0
 * Generated: 2024
 */

#ifndef IA_GET_SIMPLE_H
#define IA_GET_SIMPLE_H

#ifdef __cplusplus
extern "C" {
#endif

#include <stdint.h>

/**
 * Result codes for FFI operations
 */
typedef enum IaGetResult {
    /** Operation completed successfully */
    IaGetResult_Success = 0,
    /** Network error (connection, timeout, HTTP error) */
    IaGetResult_ErrorNetwork = 1,
    /** File system error (permission, disk space, I/O) */
    IaGetResult_ErrorFileSystem = 2,
    /** Invalid input parameter */
    IaGetResult_ErrorInvalidInput = 3,
    /** Internal error */
    IaGetResult_ErrorInternal = 4,
} IaGetResult;

/**
 * Progress callback type for downloads
 * 
 * @param downloaded Number of bytes downloaded so far
 * @param total Total number of bytes to download (0 if unknown)
 * @param user_data User data pointer passed to ia_get_download_file()
 */
typedef void (*ProgressCallback)(uint64_t downloaded, uint64_t total, void* user_data);

/**
 * Fetch metadata for an Internet Archive item
 * 
 * Returns a JSON string containing the metadata. The caller MUST free
 * the returned string using ia_get_free_string().
 * 
 * @param identifier Archive.org identifier (e.g., "commute_test")
 * @return Pointer to JSON string on success (must be freed), NULL on error
 * 
 * Example:
 * ```c
 * char* json = ia_get_fetch_metadata("commute_test");
 * if (json) {
 *     printf("Metadata: %s\n", json);
 *     ia_get_free_string(json);
 * } else {
 *     const char* error = ia_get_last_error();
 *     printf("Error: %s\n", error);
 * }
 * ```
 */
char* ia_get_fetch_metadata(const char* identifier);

/**
 * Download a file from URL to specified path
 * 
 * This is a BLOCKING operation - the caller should run it in a background thread.
 * 
 * @param url Source URL
 * @param output_path Destination file path
 * @param progress_callback Optional callback for progress updates (can be NULL)
 * @param user_data User data passed to callback (can be NULL)
 * @return IaGetResult_Success on success, error code on failure
 * 
 * Example:
 * ```c
 * void progress(uint64_t downloaded, uint64_t total, void* data) {
 *     printf("Progress: %llu / %llu bytes\n", downloaded, total);
 * }
 * 
 * IaGetResult result = ia_get_download_file(
 *     "https://archive.org/download/example/file.pdf",
 *     "/path/to/output.pdf",
 *     progress,
 *     NULL
 * );
 * ```
 */
IaGetResult ia_get_download_file(
    const char* url,
    const char* output_path,
    ProgressCallback progress_callback,
    void* user_data
);

/**
 * Decompress an archive file
 * 
 * Supports: zip, gzip, bzip2, xz, tar, tar.gz, tar.bz2, tar.xz
 * 
 * Returns a JSON array of extracted file paths. The caller MUST free
 * the returned string using ia_get_free_string().
 * 
 * @param archive_path Path to archive file
 * @param output_dir Directory to extract to
 * @return Pointer to JSON array on success (must be freed), NULL on error
 * 
 * Example:
 * ```c
 * char* files = ia_get_decompress_file("archive.tar.gz", "/tmp/output");
 * if (files) {
 *     printf("Extracted files: %s\n", files);
 *     ia_get_free_string(files);
 * }
 * ```
 */
char* ia_get_decompress_file(const char* archive_path, const char* output_dir);

/**
 * Validate file checksum
 * 
 * @param file_path Path to file to validate
 * @param expected_hash Expected hash value (hex string)
 * @param hash_type Hash algorithm: "md5", "sha1", or "sha256"
 * @return 1 if hash matches, 0 if mismatch, -1 on error
 * 
 * Example:
 * ```c
 * int result = ia_get_validate_checksum(
 *     "/path/to/file.pdf",
 *     "d41d8cd98f00b204e9800998ecf8427e",
 *     "md5"
 * );
 * if (result == 1) {
 *     printf("Checksum valid\n");
 * } else if (result == 0) {
 *     printf("Checksum mismatch\n");
 * } else {
 *     printf("Error: %s\n", ia_get_last_error());
 * }
 * ```
 */
int ia_get_validate_checksum(
    const char* file_path,
    const char* expected_hash,
    const char* hash_type
);

/**
 * Get last error message
 * 
 * Returns a pointer to a static string containing the last error message.
 * The returned pointer is valid until the next FFI call in the same thread.
 * DO NOT FREE this pointer.
 * 
 * @return Pointer to error message string (do NOT free), NULL if no error
 * 
 * Example:
 * ```c
 * if (!ia_get_fetch_metadata("invalid")) {
 *     const char* error = ia_get_last_error();
 *     if (error) {
 *         fprintf(stderr, "Error: %s\n", error);
 *     }
 * }
 * ```
 */
const char* ia_get_last_error(void);

/**
 * Free a string returned by this library
 * 
 * Use this to free strings returned by ia_get_fetch_metadata() and
 * ia_get_decompress_file().
 * 
 * DO NOT use this to free ia_get_last_error() results.
 * 
 * @param s Pointer to string to free (can be NULL)
 * 
 * Example:
 * ```c
 * char* json = ia_get_fetch_metadata("example");
 * if (json) {
 *     // Use json...
 *     ia_get_free_string(json);  // Free when done
 * }
 * ```
 */
void ia_get_free_string(char* s);

#ifdef __cplusplus
}
#endif

#endif /* IA_GET_SIMPLE_H */
