import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:provider/provider.dart';
import 'services/archive_service.dart';
import 'services/background_download_service.dart';
import 'services/deep_link_service.dart';
import 'providers/download_provider.dart';
import 'screens/home_screen.dart';
import 'screens/archive_detail_screen.dart';
import 'screens/download_screen.dart';
import 'widgets/onboarding_widget.dart';
import 'utils/theme.dart';
import 'utils/permission_utils.dart';

void main() async {
  // Ensure Flutter is initialized
  WidgetsFlutterBinding.ensureInitialized();

  // Set preferred orientations for mobile optimization
  await SystemChrome.setPreferredOrientations([
    DeviceOrientation.portraitUp,
    DeviceOrientation.portraitDown,
    DeviceOrientation.landscapeLeft,
    DeviceOrientation.landscapeRight,
  ]);

  // Configure system UI for immersive experience
  SystemChrome.setSystemUIOverlayStyle(
    const SystemUiOverlayStyle(
      statusBarColor: Colors.transparent,
      statusBarIconBrightness: Brightness.dark,
      systemNavigationBarColor: Colors.transparent,
      systemNavigationBarIconBrightness: Brightness.dark,
    ),
  );

  runApp(const IAGetMobileApp());
}

class IAGetMobileApp extends StatelessWidget {
  const IAGetMobileApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MultiProvider(
      providers: [
        ChangeNotifierProvider<ArchiveService>(
          create: (_) => ArchiveService(),
          lazy: true, // Lazy load for faster startup
        ),
        ChangeNotifierProvider<DownloadProvider>(
          create: (_) => DownloadProvider(),
          lazy: true, // Lazy load for faster startup
        ),
        ChangeNotifierProvider<BackgroundDownloadService>(
          create: (_) => BackgroundDownloadService(),
          lazy: true, // Lazy load for faster startup
        ),
        Provider<DeepLinkService>(
          create: (_) => DeepLinkService(),
          dispose: (_, service) => service.dispose(),
          lazy: true, // Lazy load for faster startup
        ),
      ],
      child: MaterialApp(
        title: 'Internet Archive Helper',
        theme: AppTheme.lightTheme,
        darkTheme: AppTheme.darkTheme,
        themeMode: ThemeMode.system,
        home: const AppInitializer(),
        debugShowCheckedModeBanner: false,

        // Performance optimizations
        builder: (context, child) {
          // Disable text scaling for consistent UI
          final mediaQuery = MediaQuery.of(context);
          final scaleFactor = mediaQuery.textScaler.scale(1.0).clamp(0.8, 1.2);
          return MediaQuery(
            data: mediaQuery.copyWith(
              textScaler: TextScaler.linear(scaleFactor),
            ),
            child: child!,
          );
        },

        // Navigation performance
        onGenerateRoute: (settings) {
          // Implement custom route generation for better performance and state restoration
          switch (settings.name) {
            case '/':
              return MaterialPageRoute(
                builder: (_) => const AppInitializer(),
                settings: settings,
              );
            case '/home':
              return MaterialPageRoute(
                builder: (_) => const HomeScreen(),
                settings: settings,
              );
            case ArchiveDetailScreen.routeName:
              return MaterialPageRoute(
                builder: (_) => const ArchiveDetailScreen(),
                settings: settings,
              );
            case DownloadScreen.routeName:
              return MaterialPageRoute(
                builder: (_) => const DownloadScreen(),
                settings: settings,
              );
            default:
              return MaterialPageRoute(
                builder: (_) => const AppInitializer(),
                settings: settings,
              );
          }
        },
      ),
    );
  }
}

/// App initializer that handles onboarding flow
class AppInitializer extends StatefulWidget {
  const AppInitializer({super.key});

  @override
  State<AppInitializer> createState() => _AppInitializerState();
}

class _AppInitializerState extends State<AppInitializer> {
  bool _isLoading = true;
  bool _shouldShowOnboarding = false;
  String? _initializationError;

  @override
  void initState() {
    super.initState();
    _initializeApp();
  }

  /// Initialize app with proper sequencing and error handling
  /// 
  /// Startup sequence (optimized for fast app launch):
  /// 1. Check onboarding status (fast, local operation)
  /// 2. Show UI immediately (deferred service initialization)
  /// 3. Initialize services lazily on first access
  /// 
  /// Services are now lazy-loaded through Provider, eliminating startup bottleneck.
  Future<void> _initializeApp() async {
    try {
      // Check onboarding status (fast, local check)
      await _checkOnboardingStatus();

      // Initialize critical services after first frame
      WidgetsBinding.instance.addPostFrameCallback((_) async {
        await _initializeCriticalServices();
      });
    } catch (e) {
      if (mounted) {
        setState(() {
          _initializationError = 'Failed to initialize app: ${e.toString()}';
          _isLoading = false;
        });
      }
      debugPrint('App initialization error: $e');
    }
  }

  /// Initialize only critical services that need early setup
  Future<void> _initializeCriticalServices() async {
    if (!mounted) return;

    try {
      // Initialize BackgroundDownloadService (needs early setup for notifications)
      final bgService = context.read<BackgroundDownloadService>();
      // Initialize DeepLinkService (needs early setup for app links)
      final deepLinkService = context.read<DeepLinkService>();
      
      await bgService.initialize().timeout(
        const Duration(seconds: 10),
        onTimeout: () {
          debugPrint('Background service initialization timed out');
        },
      );

      await deepLinkService.initialize().timeout(
        const Duration(seconds: 5),
        onTimeout: () {
          debugPrint('DeepLink service initialization timed out');
        },
      );

      // Set up deep link handler
      deepLinkService.onArchiveLinkReceived = (identifier) {
        if (!mounted) return;
        
        final archiveService = context.read<ArchiveService>();
        archiveService.fetchMetadata(identifier);
      };

      // Request notification permissions (non-blocking)
      _requestNotificationPermissions();

    } catch (e) {
      // Log but don't block app startup for service initialization failures
      debugPrint('Service initialization error: $e');
    }
  }

  /// Request notification permissions for download notifications (non-blocking)
  Future<void> _requestNotificationPermissions() async {
    try {
      // Check if already granted
      final hasPermission = await PermissionUtils.hasNotificationPermissions();
      if (hasPermission) return;

      // Request permission (will be silently skipped on older Android versions)
      await PermissionUtils.requestNotificationPermissions();
    } catch (e) {
      // Non-critical - just log and continue
      debugPrint('Failed to request notification permissions: $e');
    }
  }

  Future<void> _checkOnboardingStatus() async {
    try {
      final shouldShow = await OnboardingWidget.shouldShowOnboarding()
          .timeout(
            const Duration(seconds: 5),
            onTimeout: () => false, // Default to not showing on timeout
          );
      
      if (mounted) {
        setState(() {
          _shouldShowOnboarding = shouldShow;
          _isLoading = false;
        });
      }
    } catch (e) {
      debugPrint('Error checking onboarding status: $e');
      if (mounted) {
        setState(() {
          _shouldShowOnboarding = false;
          _isLoading = false;
        });
      }
    }
  }

  void _completeOnboarding() {
    setState(() {
      _shouldShowOnboarding = false;
    });
  }

  @override
  Widget build(BuildContext context) {
    // Show error state if initialization failed
    if (_initializationError != null) {
      return Scaffold(
        body: Center(
          child: Column(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              const Icon(
                Icons.error_outline,
                size: 64,
                color: Colors.red,
              ),
              const SizedBox(height: 16),
              Text(
                'Initialization Error',
                style: Theme.of(context).textTheme.headlineSmall,
              ),
              const SizedBox(height: 8),
              Padding(
                padding: const EdgeInsets.symmetric(horizontal: 32),
                child: Text(
                  _initializationError!,
                  textAlign: TextAlign.center,
                  style: Theme.of(context).textTheme.bodyMedium,
                ),
              ),
              const SizedBox(height: 32),
              ElevatedButton(
                onPressed: () {
                  setState(() {
                    _initializationError = null;
                    _isLoading = true;
                  });
                  _initializeApp();
                },
                child: const Text('Retry'),
              ),
            ],
          ),
        ),
      );
    }

    if (_isLoading) {
      return Scaffold(
        body: Center(
          child: Column(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              Icon(
                Icons.library_books,
                size: 64,
                color: Theme.of(context).primaryColor,
              ),
              const SizedBox(height: 16),
              Text(
                'Internet Archive Helper',
                style: Theme.of(context).textTheme.headlineSmall,
              ),
              const SizedBox(height: 32),
              const CircularProgressIndicator(),
            ],
          ),
        ),
      );
    }

    if (_shouldShowOnboarding) {
      return OnboardingWidget(onComplete: _completeOnboarding);
    }

    return const HomeScreen();
  }
}
