use ia_get::{
    archive_metadata::JsonFile,
    cli::{Cli, SourceType},
    filters::filter_files,
};

#[test]
fn test_source_filtering_basic() {
    // Test default: original only
    let files = vec![
        JsonFile {
            name: "original_file.txt".to_string(),
            source: "original".to_string(),
            mtime: None,
            size: Some(1024),
            format: None,
            rotation: None,
            md5: None,
            crc32: None,
            sha1: None,
            btih: None,
            summation: None,
            original: None,
        },
        JsonFile {
            name: "metadata_file.xml".to_string(),
            source: "metadata".to_string(),
            mtime: None,
            size: Some(512),
            format: None,
            rotation: None,
            md5: None,
            crc32: None,
            sha1: None,
            btih: None,
            summation: None,
            original: None,
        },
    ];

    let cli_original_only = Cli {
        source_types: vec![SourceType::Original],
        ..Default::default()
    };
    let filtered = filter_files(files, &cli_original_only);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].name, "original_file.txt");

    // Test metadata only
    let files2 = vec![
        JsonFile {
            name: "original_file.txt".to_string(),
            source: "original".to_string(),
            mtime: None,
            size: Some(1024),
            format: None,
            rotation: None,
            md5: None,
            crc32: None,
            sha1: None,
            btih: None,
            summation: None,
            original: None,
        },
        JsonFile {
            name: "metadata_file.xml".to_string(),
            source: "metadata".to_string(),
            mtime: None,
            size: Some(512),
            format: None,
            rotation: None,
            md5: None,
            crc32: None,
            sha1: None,
            btih: None,
            summation: None,
            original: None,
        },
    ];

    let cli_metadata_only = Cli {
        source_types: vec![SourceType::Metadata],
        ..Default::default()
    };
    let filtered = filter_files(files2, &cli_metadata_only);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].name, "metadata_file.xml");
}

#[test]
fn test_comprehensive_source_filtering() {
    // Create test files representing real Internet Archive structure
    let create_test_files = || {
        vec![
            // Original files (user uploaded)
            JsonFile {
                name: "document.pdf".to_string(),
                source: "original".to_string(),
                mtime: Some(1609459200), // 2021-01-01
                size: Some(1048576),     // 1MB
                format: Some("PDF".to_string()),
                rotation: None,
                md5: Some("abc123".to_string()),
                crc32: None,
                sha1: None,
                btih: None,
                summation: None,
                original: None,
            },
            JsonFile {
                name: "audio.mp3".to_string(),
                source: "original".to_string(),
                mtime: Some(1609459200),
                size: Some(5242880), // 5MB
                format: Some("MP3".to_string()),
                rotation: None,
                md5: Some("def456".to_string()),
                crc32: None,
                sha1: None,
                btih: None,
                summation: None,
                original: None,
            },
            // Derivative files (generated from originals)
            JsonFile {
                name: "audio_64kb.mp3".to_string(),
                source: "derivative".to_string(),
                mtime: Some(1609459260), // Generated later
                size: Some(524288),      // 512KB
                format: Some("64Kbps MP3".to_string()),
                rotation: None,
                md5: Some("ghi789".to_string()),
                crc32: None,
                sha1: None,
                btih: None,
                summation: None,
                original: Some("audio.mp3".to_string()),
            },
            JsonFile {
                name: "document_text.txt".to_string(),
                source: "derivative".to_string(),
                mtime: Some(1609459260),
                size: Some(102400), // 100KB
                format: Some("Extracted Text".to_string()),
                rotation: None,
                md5: Some("jkl012".to_string()),
                crc32: None,
                sha1: None,
                btih: None,
                summation: None,
                original: Some("document.pdf".to_string()),
            },
            // Metadata files (archive system generated)
            JsonFile {
                name: "item_meta.xml".to_string(),
                source: "metadata".to_string(),
                mtime: Some(1609459300),
                size: Some(2048), // 2KB
                format: Some("Metadata".to_string()),
                rotation: None,
                md5: Some("mno345".to_string()),
                crc32: None,
                sha1: None,
                btih: None,
                summation: None,
                original: None,
            },
            JsonFile {
                name: "item_files.xml".to_string(),
                source: "metadata".to_string(),
                mtime: Some(1609459300),
                size: Some(4096), // 4KB
                format: Some("Metadata".to_string()),
                rotation: None,
                md5: Some("pqr678".to_string()),
                crc32: None,
                sha1: None,
                btih: None,
                summation: None,
                original: None,
            },
            JsonFile {
                name: "item.torrent".to_string(),
                source: "metadata".to_string(),
                mtime: Some(1609459300),
                size: Some(1024), // 1KB
                format: Some("Archive BitTorrent".to_string()),
                rotation: None,
                md5: Some("stu901".to_string()),
                crc32: None,
                sha1: None,
                btih: Some("0139f082bda90d39a851b4d9c17d6062cdb070dc".to_string()),
                summation: None,
                original: None,
            },
        ]
    };

    // Test 1: Default behavior (original files only)
    let files = create_test_files();
    let cli = Cli::default(); // Should default to original only
    let filtered = filter_files(files, &cli);
    assert_eq!(
        filtered.len(),
        2,
        "Should only include original files by default"
    );
    assert!(filtered.iter().any(|f| f.name == "document.pdf"));
    assert!(filtered.iter().any(|f| f.name == "audio.mp3"));
    assert!(filtered.iter().all(|f| f.source == "original"));

    // Test 2: Original files only (explicit)
    let files = create_test_files();
    let cli = Cli {
        source_types: vec![SourceType::Original],
        original_only: true,
        ..Default::default()
    };
    let filtered = filter_files(files, &cli);
    assert_eq!(filtered.len(), 2, "Should only include original files");
    assert!(filtered.iter().all(|f| f.source == "original"));

    // Test 3: Metadata files only
    let files = create_test_files();
    let cli = Cli {
        source_types: vec![SourceType::Metadata],
        ..Default::default()
    };
    let filtered = filter_files(files, &cli);
    assert_eq!(filtered.len(), 3, "Should include all metadata files");
    assert!(filtered.iter().any(|f| f.name == "item_meta.xml"));
    assert!(filtered.iter().any(|f| f.name == "item_files.xml"));
    assert!(filtered.iter().any(|f| f.name == "item.torrent"));
    assert!(filtered.iter().all(|f| f.source == "metadata"));

    // Test 4: Derivative files only
    let files = create_test_files();
    let cli = Cli {
        source_types: vec![SourceType::Derivative],
        ..Default::default()
    };
    let filtered = filter_files(files, &cli);
    assert_eq!(filtered.len(), 2, "Should include all derivative files");
    assert!(filtered.iter().any(|f| f.name == "audio_64kb.mp3"));
    assert!(filtered.iter().any(|f| f.name == "document_text.txt"));
    assert!(filtered.iter().all(|f| f.source == "derivative"));

    // Test 5: Multiple source types
    let files = create_test_files();
    let cli = Cli {
        source_types: vec![SourceType::Original, SourceType::Derivative],
        ..Default::default()
    };
    let filtered = filter_files(files, &cli);
    assert_eq!(
        filtered.len(),
        4,
        "Should include original and derivative files"
    );
    assert!(filtered.iter().any(|f| f.source == "original"));
    assert!(filtered.iter().any(|f| f.source == "derivative"));
    assert!(!filtered.iter().any(|f| f.source == "metadata"));

    // Test 6: All source types
    let files = create_test_files();
    let cli = Cli {
        source_types: vec![
            SourceType::Original,
            SourceType::Derivative,
            SourceType::Metadata,
        ],
        ..Default::default()
    };
    let filtered = filter_files(files, &cli);
    assert_eq!(filtered.len(), 7, "Should include all files");

    // Test 7: Source filtering combined with extension filtering
    let files = create_test_files();
    let cli = Cli {
        source_types: vec![SourceType::Original, SourceType::Derivative],
        include_ext: Some("mp3".to_string()),
        ..Default::default()
    };
    let filtered = filter_files(files, &cli);
    assert_eq!(
        filtered.len(),
        2,
        "Should include only MP3 files from original and derivative"
    );
    assert!(filtered.iter().any(|f| f.name == "audio.mp3"));
    assert!(filtered.iter().any(|f| f.name == "audio_64kb.mp3"));

    // Test 8: Source filtering combined with size filtering
    let files = create_test_files();
    let cli = Cli {
        source_types: vec![
            SourceType::Original,
            SourceType::Derivative,
            SourceType::Metadata,
        ],
        max_file_size: Some("1MB".to_string()),
        ..Default::default()
    };
    let filtered = filter_files(files, &cli);
    assert_eq!(
        filtered.len(),
        6,
        "Should include files 1MB and under from all sources"
    );
    // Should exclude audio.mp3 (5MB) but include document.pdf (1MB exactly)
    assert!(!filtered.iter().any(|f| f.name == "audio.mp3"));
    assert!(filtered.iter().any(|f| f.name == "document.pdf")); // Exactly 1MB should be included

    // Test 9: Convenience flags - include_derivatives
    let files = create_test_files();
    let cli = Cli {
        include_derivatives: true,
        ..Default::default()
    };
    let source_types = cli.get_source_types();
    assert!(source_types.contains(&SourceType::Original));
    assert!(source_types.contains(&SourceType::Derivative));
    assert!(!source_types.contains(&SourceType::Metadata));

    let filtered = filter_files(files, &cli);
    assert_eq!(
        filtered.len(),
        4,
        "Should include original and derivative files"
    );

    // Test 10: Convenience flags - include_metadata
    let files = create_test_files();
    let cli = Cli {
        include_metadata: true,
        ..Default::default()
    };
    let source_types = cli.get_source_types();
    assert!(source_types.contains(&SourceType::Original));
    assert!(!source_types.contains(&SourceType::Derivative));
    assert!(source_types.contains(&SourceType::Metadata));

    let filtered = filter_files(files, &cli);
    assert_eq!(
        filtered.len(),
        5,
        "Should include original and metadata files"
    );

    // Test 11: Helper function should_download_source
    let cli = Cli {
        source_types: vec![SourceType::Original, SourceType::Metadata],
        ..Default::default()
    };
    assert!(cli.should_download_source("original"));
    assert!(!cli.should_download_source("derivative"));
    assert!(cli.should_download_source("metadata"));

    // Test 12: SourceType enum methods
    assert_eq!(SourceType::Original.as_str(), "original");
    assert_eq!(SourceType::Derivative.as_str(), "derivative");
    assert_eq!(SourceType::Metadata.as_str(), "metadata");

    assert!(SourceType::Original.matches("original"));
    assert!(!SourceType::Original.matches("derivative"));
    assert!(SourceType::Metadata.matches("metadata"));

    // Test 13: Edge case - empty files list
    let files: Vec<JsonFile> = vec![];
    let cli = Cli::default();
    let filtered = filter_files(files, &cli);
    assert_eq!(filtered.len(), 0, "Empty list should remain empty");

    // Test 14: Edge case - unknown source type
    let files = vec![JsonFile {
        name: "unknown_source.dat".to_string(),
        source: "unknown".to_string(), // Non-standard source
        mtime: None,
        size: Some(1024),
        format: None,
        rotation: None,
        md5: None,
        crc32: None,
        sha1: None,
        btih: None,
        summation: None,
        original: None,
    }];
    let cli = Cli::default();
    let filtered = filter_files(files, &cli);
    assert_eq!(
        filtered.len(),
        0,
        "Unknown source types should be filtered out"
    );
}

#[test]
fn test_source_type_enum() {
    // Test string conversions
    assert_eq!(SourceType::Original.as_str(), "original");
    assert_eq!(SourceType::Derivative.as_str(), "derivative");
    assert_eq!(SourceType::Metadata.as_str(), "metadata");

    // Test matching
    assert!(SourceType::Original.matches("original"));
    assert!(!SourceType::Original.matches("derivative"));
    assert!(!SourceType::Original.matches("metadata"));
    assert!(!SourceType::Original.matches("unknown"));

    assert!(SourceType::Derivative.matches("derivative"));
    assert!(!SourceType::Derivative.matches("original"));
    assert!(!SourceType::Derivative.matches("metadata"));

    assert!(SourceType::Metadata.matches("metadata"));
    assert!(!SourceType::Metadata.matches("original"));
    assert!(!SourceType::Metadata.matches("derivative"));

    // Test case sensitivity
    assert!(!SourceType::Original.matches("ORIGINAL"));
    assert!(!SourceType::Original.matches("Original"));
}

#[test]
fn test_cli_source_helpers() {
    // Test default source types
    let cli = Cli::default();
    let source_types = cli.get_source_types();
    assert_eq!(source_types.len(), 1);
    assert_eq!(source_types[0], SourceType::Original);

    // Test original_only flag
    let cli = Cli {
        original_only: true,
        ..Default::default()
    };
    let source_types = cli.get_source_types();
    assert_eq!(source_types.len(), 1);
    assert_eq!(source_types[0], SourceType::Original);

    // Test include_derivatives flag
    let cli = Cli {
        include_derivatives: true,
        ..Default::default()
    };
    let source_types = cli.get_source_types();
    assert_eq!(source_types.len(), 2);
    assert!(source_types.contains(&SourceType::Original));
    assert!(source_types.contains(&SourceType::Derivative));

    // Test include_metadata flag
    let cli = Cli {
        include_metadata: true,
        ..Default::default()
    };
    let source_types = cli.get_source_types();
    assert_eq!(source_types.len(), 2);
    assert!(source_types.contains(&SourceType::Original));
    assert!(source_types.contains(&SourceType::Metadata));

    // Test both include flags
    let cli = Cli {
        include_derivatives: true,
        include_metadata: true,
        ..Default::default()
    };
    let source_types = cli.get_source_types();
    assert_eq!(source_types.len(), 3);
    assert!(source_types.contains(&SourceType::Original));
    assert!(source_types.contains(&SourceType::Derivative));
    assert!(source_types.contains(&SourceType::Metadata));

    // Test explicit source_types override convenience flags
    let cli = Cli {
        source_types: vec![SourceType::Metadata],
        include_derivatives: true, // Should be ignored
        ..Default::default()
    };
    let source_types = cli.get_source_types();
    assert_eq!(source_types.len(), 1);
    assert_eq!(source_types[0], SourceType::Metadata);

    // Test should_download_source
    let cli = Cli {
        source_types: vec![SourceType::Original, SourceType::Derivative],
        ..Default::default()
    };
    assert!(cli.should_download_source("original"));
    assert!(cli.should_download_source("derivative"));
    assert!(!cli.should_download_source("metadata"));
    assert!(!cli.should_download_source("unknown"));
    assert!(!cli.should_download_source(""));
}
