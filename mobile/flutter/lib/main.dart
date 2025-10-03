import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:provider/provider.dart';
import 'services/ia_get_service.dart';
import 'services/background_download_service.dart';
import 'services/deep_link_service.dart';
import 'screens/home_screen.dart';
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
        ChangeNotifierProvider<IaGetService>(
          create: (_) => IaGetService(),
          lazy: false, // Initialize immediately for better startup performance
        ),
        ChangeNotifierProvider<BackgroundDownloadService>(
          create: (_) => BackgroundDownloadService(),
          lazy: false, // Initialize immediately for background downloads
        ),
        Provider<DeepLinkService>(
          create: (_) => DeepLinkService(),
          dispose: (_, service) => service.dispose(),
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
          return MediaQuery(
            data: MediaQuery.of(context).copyWith(
              textScaler: TextScaler.linear(
                MediaQuery.of(context).textScaler.scale(1.0).clamp(0.8, 1.2),
              ),
            ),
            child: child!,
          );
        },

        // Navigation performance
        onGenerateRoute: (settings) {
          // Implement custom route generation for better performance
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

  @override
  void initState() {
    super.initState();
    _checkOnboardingStatus();

    // Initialize background download service
    WidgetsBinding.instance.addPostFrameCallback((_) {
      context.read<BackgroundDownloadService>().initialize();

      // Initialize deep link service
      final deepLinkService = context.read<DeepLinkService>();
      deepLinkService.initialize();

      // Handle incoming archive links
      deepLinkService.onArchiveLinkReceived = (identifier) {
        // Navigate to home and trigger search
        if (mounted) {
          final iaGetService = context.read<IaGetService>();
          iaGetService.fetchMetadata(identifier);
        }
      };

      // Request notification permissions (non-blocking, Android 13+)
      _requestNotificationPermissions();
    });
  }

  /// Request notification permissions for download notifications
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
    final shouldShow = await OnboardingWidget.shouldShowOnboarding();
    setState(() {
      _shouldShowOnboarding = shouldShow;
      _isLoading = false;
    });
  }

  void _completeOnboarding() {
    setState(() {
      _shouldShowOnboarding = false;
    });
  }

  @override
  Widget build(BuildContext context) {
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
