# Flutter Project Reorganization Plan

## Current Structure Issues
- Flat organization in `lib/` makes it hard to find related files
- Services, providers, and widgets are mixed together
- No clear separation between features
- Difficult to maintain as project grows

## Proposed Feature-Based Architecture

```
lib/
├── core/                     # Core functionality (shared across features)
│   ├── constants/           # ✅ Already created
│   │   └── internet_archive_constants.dart
│   ├── errors/              # Custom exceptions
│   ├── network/             # HTTP client, interceptors
│   └── utils/               # Utility functions, extensions
│
├── features/                # Feature-based modules
│   ├── search/
│   │   ├── data/           # Data layer (repositories, data sources)
│   │   ├── domain/         # Business logic (entities, use cases)
│   │   └── presentation/   # UI (screens, widgets, providers)
│   │
│   ├── archive_details/
│   │   ├── data/
│   │   ├── domain/
│   │   └── presentation/
│   │
│   ├── download/
│   │   ├── data/
│   │   ├── domain/
│   │   └── presentation/
│   │
│   └── settings/
│       ├── data/
│       ├── domain/
│       └── presentation/
│
├── shared/                  # Shared widgets and components
│   ├── widgets/
│   └── theme/
│
└── main.dart

```

## Migration Strategy

### Phase 1: Create new structure (non-breaking)
1. Create `features/` directory structure
2. Move files to new locations while keeping old ones
3. Update imports gradually

### Phase 2: Feature modules
1. **Search Feature**
   - Move search_bar_widget, advanced_filters_screen
   - Create SearchRepository
   - Clean separation of concerns

2. **Archive Details Feature**
   - Move archive_detail_screen, archive_info_widget
   - Move archive_service (rename to ArchiveRepository)
   - Add use cases for metadata fetching

3. **Download Feature**
   - Move download_screen, download_provider
   - Move download-related widgets
   - Create DownloadRepository

4. **Settings Feature**
   - Move settings_screen, help_screen
   - Add preferences management

### Phase 3: Cleanup
1. Remove old files
2. Update all imports
3. Verify functionality

## Benefits
- ✅ Clear feature boundaries
- ✅ Easier to find related code
- ✅ Better testability (can test features in isolation)
- ✅ Scalable architecture
- ✅ Follows clean architecture principles
- ✅ Each feature is self-contained

## Implementation Notes
- Keep backward compatibility during migration
- Test each feature after moving
- Update documentation
- Use barrel files (index.dart) for clean exports
