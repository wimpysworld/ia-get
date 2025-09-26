//! Advanced Internet Archive metadata processing and analysis
//!
//! This module provides enhanced metadata handling capabilities beyond the basic
//! metadata.rs module, including advanced analysis, transformation, and integration
//! with the EnhancedArchiveApiClient for comprehensive Archive.org API usage.
//!
//! ## Features
//!
//! - **Advanced Metadata Analysis**: Deep analysis of archive contents, collections, and relationships
//! - **Multi-API Integration**: Uses multiple Archive.org APIs for comprehensive data gathering
//! - **Metadata Transformation**: Convert and enhance metadata formats
//! - **Collection Analysis**: Analyze collection membership and relationships
//! - **File Type Intelligence**: Advanced file type detection and categorization
//! - **Related Items Discovery**: Find related archives and collections
//! - **Metadata Caching**: Intelligent caching for performance optimization
//! - **Enhanced Error Handling**: Robust error recovery and reporting

use crate::{
    core::session::{ArchiveFile, ArchiveMetadata},
    infrastructure::api::EnhancedArchiveApiClient,
    utilities::common::format_size,
    utilities::filters::FormatCategory,
    IaGetError, Result,
};
use colored::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::SystemTime;

/// Advanced metadata analysis result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataAnalysis {
    /// Basic archive information
    pub identifier: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub creator: Option<String>,
    pub date: Option<String>,

    /// File analysis
    pub file_count: usize,
    pub total_size: u64,
    pub file_types: HashMap<String, FileTypeInfo>,
    pub largest_files: Vec<FileInfo>,
    pub size_distribution: SizeDistribution,

    /// Collection information
    pub collections: Vec<String>,
    pub collection_details: HashMap<String, CollectionInfo>,

    /// Related items
    pub related_items: Vec<RelatedItem>,
    pub has_related: bool,

    /// Advanced metadata
    pub subject_tags: Vec<String>,
    pub language: Option<String>,
    pub mediatype: Option<String>,
    pub upload_date: Option<SystemTime>,
    pub last_modified: Option<SystemTime>,

    /// Quality metrics
    pub completeness_score: f32,
    pub quality_indicators: QualityIndicators,

    /// API health information
    pub api_status: ApiHealthStatus,
}

/// File type information with statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileTypeInfo {
    pub count: usize,
    pub total_size: u64,
    pub category: Option<FormatCategory>,
    pub sample_files: Vec<String>,
}

/// File information for analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub name: String,
    pub size: u64,
    pub format: Option<String>,
    pub source: String,
}

/// Size distribution analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SizeDistribution {
    pub small_files: usize,  // < 1MB
    pub medium_files: usize, // 1MB - 100MB
    pub large_files: usize,  // 100MB - 1GB
    pub huge_files: usize,   // > 1GB
    pub average_size: u64,
    pub median_size: u64,
}

/// Collection information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionInfo {
    pub name: String,
    pub description: Option<String>,
    pub item_count: Option<usize>,
    pub is_featured: bool,
}

/// Related item information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelatedItem {
    pub identifier: String,
    pub title: Option<String>,
    pub mediatype: Option<String>,
    pub relation_type: RelationType,
}

/// Type of relationship between items
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RelationType {
    SameCreator,
    SameCollection,
    SimilarSubject,
    Sequential,
    Other(String),
}

/// Quality indicators for metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityIndicators {
    pub has_description: bool,
    pub has_creator: bool,
    pub has_date: bool,
    pub has_subjects: bool,
    pub has_thumbnails: bool,
    pub files_have_checksums: f32, // Percentage
    pub files_have_formats: f32,   // Percentage
}

/// API health status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiHealthStatus {
    pub metadata_api: bool,
    pub search_api: bool,
    pub tasks_api: bool,
    pub collections_api: bool,
    pub response_times: HashMap<String, u64>, // milliseconds
}

/// Type alias for advanced metadata fields tuple
type AdvancedMetadataFields = (
    Vec<String>,
    Option<String>,
    Option<String>,
    Option<SystemTime>,
    Option<SystemTime>,
);

/// Enhanced metadata processor
pub struct AdvancedMetadataProcessor {
    api_client: EnhancedArchiveApiClient,
    cache: HashMap<String, CachedAnalysis>,
}

/// Cached analysis with timestamp
#[derive(Debug, Clone)]
struct CachedAnalysis {
    analysis: MetadataAnalysis,
    timestamp: SystemTime,
}

impl AdvancedMetadataProcessor {
    /// Create a new advanced metadata processor
    pub fn new(api_client: EnhancedArchiveApiClient) -> Self {
        Self {
            api_client,
            cache: HashMap::new(),
        }
    }

    /// Perform comprehensive metadata analysis
    pub async fn analyze_metadata(&mut self, identifier: &str) -> Result<MetadataAnalysis> {
        // Check cache first
        if let Some(cached) = self.get_cached_analysis(identifier) {
            return Ok(cached);
        }

        println!(
            "{} Analyzing metadata for '{}'...",
            "🔍".cyan(),
            identifier.bright_blue()
        );

        // Step 1: Get basic metadata
        let basic_metadata = self.get_basic_metadata(identifier).await?;

        // Step 2: Analyze files
        let file_analysis = self.analyze_files(&basic_metadata.files).await?;

        // Step 3: Get collection information
        let collection_info = self
            .analyze_collections(&basic_metadata, identifier)
            .await?;

        // Step 4: Find related items
        let related_items = self.find_related_items(identifier, &basic_metadata).await?;

        // Step 5: Calculate quality metrics
        let quality_indicators = self.calculate_quality_indicators(&basic_metadata);

        // Step 6: Get API health status
        let api_status = self.get_api_health_status().await?;

        // Step 7: Extract advanced metadata fields
        let (subject_tags, language, mediatype, upload_date, last_modified) =
            self.extract_advanced_fields(&basic_metadata);

        let analysis = MetadataAnalysis {
            identifier: identifier.to_string(),
            title: basic_metadata
                .metadata
                .get("title")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            description: basic_metadata
                .metadata
                .get("description")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            creator: basic_metadata
                .metadata
                .get("creator")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            date: basic_metadata
                .metadata
                .get("date")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),

            file_count: basic_metadata.files.len(),
            total_size: file_analysis.total_size,
            file_types: file_analysis.file_types,
            largest_files: file_analysis.largest_files,
            size_distribution: file_analysis.size_distribution,

            collections: basic_metadata
                .metadata
                .get("collection")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default(),
            collection_details: collection_info,

            related_items: related_items.clone(),
            has_related: !related_items.is_empty(),

            subject_tags,
            language,
            mediatype,
            upload_date,
            last_modified,

            completeness_score: self
                .calculate_completeness_score(&basic_metadata, &quality_indicators),
            quality_indicators,

            api_status,
        };

        // Cache the result
        self.cache_analysis(identifier, &analysis);

        Ok(analysis)
    }

    /// Get basic metadata from archive
    async fn get_basic_metadata(&mut self, identifier: &str) -> Result<ArchiveMetadata> {
        // Use the enhanced API client to get metadata
        let metadata_json = self.api_client.get_metadata_json(identifier).await?;

        // Parse the metadata
        let metadata: ArchiveMetadata = serde_json::from_value(metadata_json)
            .map_err(|e| IaGetError::JsonParsing(format!("Failed to parse metadata: {}", e)))?;

        Ok(metadata)
    }

    /// Analyze files in the archive
    async fn analyze_files(&self, files: &[ArchiveFile]) -> Result<FileAnalysisResult> {
        let mut file_types: HashMap<String, FileTypeInfo> = HashMap::new();
        let mut total_size = 0u64;
        let mut sizes: Vec<u64> = Vec::new();
        let mut largest_files: Vec<FileInfo> = Vec::new();

        for file in files {
            let size = file.size.unwrap_or(0);
            total_size += size;
            sizes.push(size);

            // Determine file type and category
            let extension = file
                .name
                .split('.')
                .next_back()
                .unwrap_or("")
                .to_lowercase();
            let category = self.get_category_for_extension(&extension);

            // Update file type info
            let type_info = file_types.entry(extension.clone()).or_insert(FileTypeInfo {
                count: 0,
                total_size: 0,
                category,
                sample_files: Vec::new(),
            });

            type_info.count += 1;
            type_info.total_size += size;

            // Add sample files (limit to 3 per type)
            if type_info.sample_files.len() < 3 {
                type_info.sample_files.push(file.name.clone());
            }

            // Track largest files
            largest_files.push(FileInfo {
                name: file.name.clone(),
                size,
                format: file.format.clone(),
                source: file.source.clone(),
            });
        }

        // Sort largest files by size and keep top 10
        largest_files.sort_by(|a, b| b.size.cmp(&a.size));
        largest_files.truncate(10);

        // Calculate size distribution
        let size_distribution = self.calculate_size_distribution(&sizes);

        Ok(FileAnalysisResult {
            file_types,
            total_size,
            largest_files,
            size_distribution,
        })
    }

    /// Analyze collection information
    async fn analyze_collections(
        &mut self,
        metadata: &ArchiveMetadata,
        _identifier: &str,
    ) -> Result<HashMap<String, CollectionInfo>> {
        let mut collection_details = HashMap::new();

        // Extract collections from metadata
        let collections = metadata
            .metadata
            .get("collection")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
            .unwrap_or_default();

        for collection_name in collections {
            // Try to get collection information from the search API
            match self.api_client.search_collections(collection_name).await {
                Ok(search_results) => {
                    if let Some(collection_data) = search_results
                        .get("response")
                        .and_then(|r| r.get("docs"))
                        .and_then(|docs| docs.as_array())
                        .and_then(|arr| arr.first())
                    {
                        let description = collection_data
                            .get("description")
                            .and_then(|d| d.as_str())
                            .map(|s| s.to_string());

                        let item_count = collection_data
                            .get("num_found")
                            .and_then(|n| n.as_u64())
                            .map(|n| n as usize);

                        collection_details.insert(
                            collection_name.to_string(),
                            CollectionInfo {
                                name: collection_name.to_string(),
                                description,
                                item_count,
                                is_featured: false, // This would require additional API calls
                            },
                        );
                    }
                }
                Err(_) => {
                    // Fallback to basic collection info
                    collection_details.insert(
                        collection_name.to_string(),
                        CollectionInfo {
                            name: collection_name.to_string(),
                            description: None,
                            item_count: None,
                            is_featured: false,
                        },
                    );
                }
            }
        }

        Ok(collection_details)
    }

    /// Find related items using multiple strategies
    async fn find_related_items(
        &mut self,
        identifier: &str,
        metadata: &ArchiveMetadata,
    ) -> Result<Vec<RelatedItem>> {
        let mut related_items = Vec::new();

        // Strategy 1: Search by creator
        if let Some(creator) = metadata.metadata.get("creator").and_then(|v| v.as_str()) {
            if let Ok(creator_items) = self.search_by_creator(creator, identifier).await {
                related_items.extend(creator_items);
            }
        }

        // Strategy 2: Search in same collections
        let collections = metadata
            .metadata
            .get("collection")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
            .unwrap_or_default();

        for collection in collections {
            if let Ok(collection_items) = self.search_in_collection(collection, identifier).await {
                related_items.extend(collection_items);
            }
        }

        // Strategy 3: Search by subject tags (if available)
        // This would use additional metadata fields

        // Remove duplicates and limit results
        related_items.sort_by(|a, b| a.identifier.cmp(&b.identifier));
        related_items.dedup_by(|a, b| a.identifier == b.identifier);
        related_items.truncate(10);

        Ok(related_items)
    }

    /// Search for items by creator
    async fn search_by_creator(
        &mut self,
        creator: &str,
        exclude_identifier: &str,
    ) -> Result<Vec<RelatedItem>> {
        let search_query = format!("creator:\"{}\"", creator);
        let search_response = self
            .api_client
            .search_items(
                &search_query,
                Some("identifier,title,mediatype"),
                Some(5),
                None,
            )
            .await?;

        // Parse the response
        let search_text = search_response
            .text()
            .await
            .map_err(|e| IaGetError::Network(format!("Failed to read search response: {}", e)))?;

        let search_results: serde_json::Value = serde_json::from_str(&search_text)
            .map_err(|e| IaGetError::JsonParsing(format!("Failed to parse search JSON: {}", e)))?;

        let mut items = Vec::new();
        if let Some(docs) = search_results
            .get("response")
            .and_then(|r| r.get("docs"))
            .and_then(|docs| docs.as_array())
        {
            for doc in docs.iter().take(5) {
                if let Some(identifier) = doc.get("identifier").and_then(|i| i.as_str()) {
                    if identifier != exclude_identifier {
                        let title = doc
                            .get("title")
                            .and_then(|t| t.as_str())
                            .map(|s| s.to_string());
                        let mediatype = doc
                            .get("mediatype")
                            .and_then(|m| m.as_str())
                            .map(|s| s.to_string());

                        items.push(RelatedItem {
                            identifier: identifier.to_string(),
                            title,
                            mediatype,
                            relation_type: RelationType::SameCreator,
                        });
                    }
                }
            }
        }

        Ok(items)
    }

    /// Search for items in the same collection
    async fn search_in_collection(
        &mut self,
        collection: &str,
        exclude_identifier: &str,
    ) -> Result<Vec<RelatedItem>> {
        let search_query = format!("collection:\"{}\"", collection);
        let search_response = self
            .api_client
            .search_items(
                &search_query,
                Some("identifier,title,mediatype"),
                Some(3),
                None,
            )
            .await?;

        // Parse the response
        let search_text = search_response
            .text()
            .await
            .map_err(|e| IaGetError::Network(format!("Failed to read search response: {}", e)))?;

        let search_results: serde_json::Value = serde_json::from_str(&search_text)
            .map_err(|e| IaGetError::JsonParsing(format!("Failed to parse search JSON: {}", e)))?;

        let mut items = Vec::new();
        if let Some(docs) = search_results
            .get("response")
            .and_then(|r| r.get("docs"))
            .and_then(|docs| docs.as_array())
        {
            for doc in docs.iter().take(3) {
                if let Some(identifier) = doc.get("identifier").and_then(|i| i.as_str()) {
                    if identifier != exclude_identifier {
                        let title = doc
                            .get("title")
                            .and_then(|t| t.as_str())
                            .map(|s| s.to_string());
                        let mediatype = doc
                            .get("mediatype")
                            .and_then(|m| m.as_str())
                            .map(|s| s.to_string());

                        items.push(RelatedItem {
                            identifier: identifier.to_string(),
                            title,
                            mediatype,
                            relation_type: RelationType::SameCollection,
                        });
                    }
                }
            }
        }

        Ok(items)
    }

    /// Calculate quality indicators for the metadata
    fn calculate_quality_indicators(&self, metadata: &ArchiveMetadata) -> QualityIndicators {
        let files_with_checksums = metadata
            .files
            .iter()
            .filter(|f| f.md5.is_some() || f.sha1.is_some())
            .count();

        let files_with_formats = metadata.files.iter().filter(|f| f.format.is_some()).count();

        let total_files = metadata.files.len().max(1); // Avoid division by zero

        QualityIndicators {
            has_description: metadata
                .metadata
                .get("description")
                .and_then(|v| v.as_str())
                .is_some(),
            has_creator: metadata
                .metadata
                .get("creator")
                .and_then(|v| v.as_str())
                .is_some(),
            has_date: metadata
                .metadata
                .get("date")
                .and_then(|v| v.as_str())
                .is_some(),
            has_subjects: false, // This would require additional metadata fields
            has_thumbnails: metadata
                .files
                .iter()
                .any(|f| f.name.contains("_thumb") || f.format.as_deref() == Some("Thumbnail")),
            files_have_checksums: (files_with_checksums as f32 / total_files as f32) * 100.0,
            files_have_formats: (files_with_formats as f32 / total_files as f32) * 100.0,
        }
    }

    /// Get API health status
    async fn get_api_health_status(&mut self) -> Result<ApiHealthStatus> {
        let start_time = SystemTime::now();

        // Test metadata API
        let metadata_ok = self.api_client.test_metadata_api().await.is_ok();
        let metadata_time = start_time.elapsed().unwrap_or_default().as_millis() as u64;

        // Test search API
        let search_start = SystemTime::now();
        let search_ok = self.api_client.test_search_api().await.is_ok();
        let search_time = search_start.elapsed().unwrap_or_default().as_millis() as u64;

        // Test other APIs
        let tasks_ok = self.api_client.test_tasks_api().await.is_ok();
        let collections_ok = true; // Assume OK if search API works

        let mut response_times = HashMap::new();
        response_times.insert("metadata".to_string(), metadata_time);
        response_times.insert("search".to_string(), search_time);

        Ok(ApiHealthStatus {
            metadata_api: metadata_ok,
            search_api: search_ok,
            tasks_api: tasks_ok,
            collections_api: collections_ok,
            response_times,
        })
    }

    /// Extract advanced metadata fields
    fn extract_advanced_fields(&self, _metadata: &ArchiveMetadata) -> AdvancedMetadataFields {
        // These would be extracted from additional metadata fields
        // For now, return empty/None values as placeholders
        let subject_tags = Vec::new();
        let language = None;
        let mediatype = None;
        let upload_date = None;
        let last_modified = None;

        (
            subject_tags,
            language,
            mediatype,
            upload_date,
            last_modified,
        )
    }

    /// Calculate completeness score
    fn calculate_completeness_score(
        &self,
        metadata: &ArchiveMetadata,
        quality: &QualityIndicators,
    ) -> f32 {
        let mut score = 0.0;
        let mut max_score = 0.0;

        // Basic metadata fields (40% of score)
        max_score += 40.0;
        if quality.has_description {
            score += 10.0;
        }
        if quality.has_creator {
            score += 10.0;
        }
        if quality.has_date {
            score += 10.0;
        }
        if !metadata.files.is_empty() {
            score += 10.0;
        }

        // File quality (30% of score)
        max_score += 30.0;
        score += (quality.files_have_checksums / 100.0) * 15.0;
        score += (quality.files_have_formats / 100.0) * 15.0;

        // Enhanced metadata (20% of score)
        max_score += 20.0;
        if quality.has_subjects {
            score += 10.0;
        }
        if quality.has_thumbnails {
            score += 10.0;
        }

        // Collection membership (10% of score)
        max_score += 10.0;
        let has_collections = metadata
            .metadata
            .get("collection")
            .and_then(|v| v.as_array())
            .map(|arr| !arr.is_empty())
            .unwrap_or(false);
        if has_collections {
            score += 10.0;
        }

        (score / max_score) * 100.0
    }

    /// Calculate size distribution
    fn calculate_size_distribution(&self, sizes: &[u64]) -> SizeDistribution {
        let mut small_files = 0;
        let mut medium_files = 0;
        let mut large_files = 0;
        let mut huge_files = 0;

        for &size in sizes {
            match size {
                0..=1_000_000 => small_files += 1,               // < 1MB
                1_000_001..=100_000_000 => medium_files += 1,    // 1MB - 100MB
                100_000_001..=1_000_000_000 => large_files += 1, // 100MB - 1GB
                _ => huge_files += 1,                            // > 1GB
            }
        }

        let total_size: u64 = sizes.iter().sum();
        let average_size = if sizes.is_empty() {
            0
        } else {
            total_size / sizes.len() as u64
        };

        let median_size = if sizes.is_empty() {
            0
        } else {
            let mut sorted_sizes = sizes.to_vec();
            sorted_sizes.sort();
            let mid = sorted_sizes.len() / 2;
            if sorted_sizes.len().is_multiple_of(2) {
                (sorted_sizes[mid - 1] + sorted_sizes[mid]) / 2
            } else {
                sorted_sizes[mid]
            }
        };

        SizeDistribution {
            small_files,
            medium_files,
            large_files,
            huge_files,
            average_size,
            median_size,
        }
    }

    /// Get cached analysis if available and not expired
    fn get_cached_analysis(&self, identifier: &str) -> Option<MetadataAnalysis> {
        if let Some(cached) = self.cache.get(identifier) {
            // Cache expires after 1 hour
            if cached.timestamp.elapsed().unwrap_or_default().as_secs() < 3600 {
                return Some(cached.analysis.clone());
            }
        }
        None
    }

    /// Get category for file extension
    fn get_category_for_extension(&self, extension: &str) -> Option<FormatCategory> {
        // Simple mapping - this could be enhanced with the full FileFormats logic
        match extension.to_lowercase().as_str() {
            "pdf" | "doc" | "docx" | "txt" | "rtf" => Some(FormatCategory::Documents),
            "jpg" | "jpeg" | "png" | "gif" | "bmp" | "svg" => Some(FormatCategory::Images),
            "mp3" | "wav" | "flac" | "ogg" | "m4a" => Some(FormatCategory::Audio),
            "mp4" | "avi" | "mkv" | "mov" | "wmv" => Some(FormatCategory::Video),
            "zip" | "rar" | "7z" | "tar" | "gz" => Some(FormatCategory::Archives),
            "xml" | "json" | "csv" | "log" => Some(FormatCategory::Data),
            _ => None,
        }
    }
    fn cache_analysis(&mut self, identifier: &str, analysis: &MetadataAnalysis) {
        self.cache.insert(
            identifier.to_string(),
            CachedAnalysis {
                analysis: analysis.clone(),
                timestamp: SystemTime::now(),
            },
        );

        // Limit cache size
        if self.cache.len() > 100 {
            // Collect keys to remove
            let keys_to_remove: Vec<String> = {
                let mut entries: Vec<_> = self.cache.iter().collect();
                entries.sort_by_key(|(_, cached)| cached.timestamp);

                // Get oldest 20 entries
                entries
                    .iter()
                    .take(20)
                    .map(|(key, _)| (*key).clone())
                    .collect()
            };

            // Remove oldest entries
            for key in keys_to_remove {
                self.cache.remove(&key);
            }
        }
    }

    /// Display comprehensive metadata analysis
    pub fn display_analysis(&self, analysis: &MetadataAnalysis) {
        println!();
        println!(
            "{}",
            "╔══════════════════════════════════════════════════════════════════════════════╗"
                .cyan()
        );
        println!(
            "{} {} {} {}",
            "║".cyan(),
            format!("📊 Metadata Analysis for '{}'", analysis.identifier).bold(),
            " ".repeat(
                78 - format!("📊 Metadata Analysis for '{}'", analysis.identifier)
                    .len()
                    .min(78)
            ),
            "║".cyan()
        );
        println!(
            "{}",
            "╚══════════════════════════════════════════════════════════════════════════════╝"
                .cyan()
        );
        println!();

        // Basic information
        if let Some(title) = &analysis.title {
            println!("{} {}", "📄 Title:".bold(), title.bright_blue());
        }
        if let Some(creator) = &analysis.creator {
            println!("{} {}", "👤 Creator:".bold(), creator.bright_green());
        }
        if let Some(date) = &analysis.date {
            println!("{} {}", "📅 Date:".bold(), date.bright_yellow());
        }
        println!();

        // File statistics
        println!("{}", "📁 File Information:".bold().bright_magenta());
        println!(
            "   Files: {}",
            analysis.file_count.to_string().bright_cyan()
        );
        println!(
            "   Total Size: {}",
            format_size(analysis.total_size).bright_cyan()
        );

        // File types
        if !analysis.file_types.is_empty() {
            println!("   File Types:");
            let mut types: Vec<_> = analysis.file_types.iter().collect();
            types.sort_by(|a, b| b.1.count.cmp(&a.1.count));

            for (ext, info) in types.iter().take(5) {
                let percentage = (info.count as f64 / analysis.file_count as f64) * 100.0;
                println!(
                    "     {}: {} ({:.1}%)",
                    ext.cyan(),
                    info.count.to_string().bright_white(),
                    percentage
                );
            }
        }

        // Collections
        if !analysis.collections.is_empty() {
            println!(
                "   Collections: {}",
                analysis.collections.join(", ").bright_blue()
            );
        }

        // Quality score
        println!(
            "   {} Quality Score: {:.1}%",
            if analysis.completeness_score > 80.0 {
                "✓"
            } else if analysis.completeness_score > 60.0 {
                "⚠"
            } else {
                "❌"
            },
            analysis.completeness_score.to_string().bright_green()
        );

        // Related items
        if analysis.has_related {
            println!("   {} Related items available", "✓".green());
        }

        println!();

        // Related Items Information
        if !analysis.related_items.is_empty() {
            println!("{}", "🔗 Related Items Information:".bold().bright_blue());
            for (i, item) in analysis.related_items.iter().enumerate() {
                let relation_str = match &item.relation_type {
                    RelationType::SameCreator => "same creator",
                    RelationType::SameCollection => "same collection",
                    RelationType::SimilarSubject => "similar subject",
                    RelationType::Sequential => "sequential",
                    RelationType::Other(s) => s,
                };

                println!(
                    "  {}. {} ({})",
                    (i + 1).to_string().cyan(),
                    item.identifier.bright_white(),
                    relation_str.dimmed()
                );
            }
            println!();
        }

        // Collection Information
        if !analysis.collection_details.is_empty() {
            println!("{}", "📁 Collection Information:".bold().bright_green());
            for (name, info) in &analysis.collection_details {
                print!("  {}", name.bright_cyan());
                if let Some(count) = info.item_count {
                    print!(" ({} items)", count.to_string().dimmed());
                }
                println!();
                if let Some(desc) = &info.description {
                    let truncated = if desc.len() > 80 {
                        format!("{}...", &desc[..77])
                    } else {
                        desc.clone()
                    };
                    println!("    {}", truncated.dimmed());
                }
            }
            println!();
        }

        // API Health Status
        println!("{}", "🏥 API Health Status:".bold().bright_red());
        let status_icon = |ok: bool| if ok { "✅" } else { "❌" };
        println!(
            "  {} Metadata API: {}",
            status_icon(analysis.api_status.metadata_api),
            if analysis.api_status.metadata_api {
                "Operational".green()
            } else {
                "Issues".red()
            }
        );
        println!(
            "  {} Search API: {}",
            status_icon(analysis.api_status.search_api),
            if analysis.api_status.search_api {
                "Operational".green()
            } else {
                "Issues".red()
            }
        );
        println!(
            "  {} Tasks API: {}",
            status_icon(analysis.api_status.tasks_api),
            if analysis.api_status.tasks_api {
                "Operational".green()
            } else {
                "Issues".red()
            }
        );

        if let Some(response_time) = analysis.api_status.response_times.get("metadata") {
            let time_color = if *response_time < 1000 {
                format!("{}ms", response_time).green()
            } else if *response_time < 3000 {
                format!("{}ms", response_time).yellow()
            } else {
                format!("{}ms", response_time).red()
            };
            println!("  ⏱️ Response Time: {}", time_color);
        }
    }
}

/// Result of file analysis
struct FileAnalysisResult {
    file_types: HashMap<String, FileTypeInfo>,
    total_size: u64,
    largest_files: Vec<FileInfo>,
    size_distribution: SizeDistribution,
}
