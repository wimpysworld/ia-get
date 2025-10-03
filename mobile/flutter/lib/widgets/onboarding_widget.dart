import 'package:flutter/material.dart';
import 'package:shared_preferences/shared_preferences.dart';

/// Onboarding widget that helps new users understand Internet Archive Helper
class OnboardingWidget extends StatefulWidget {
  final VoidCallback onComplete;

  const OnboardingWidget({super.key, required this.onComplete});

  @override
  State<OnboardingWidget> createState() => _OnboardingWidgetState();

  static Future<bool> shouldShowOnboarding() async {
    final prefs = await SharedPreferences.getInstance();
    return !(prefs.getBool(_OnboardingWidgetState._onboardingCompleteKey) ??
        false);
  }
}

class _OnboardingWidgetState extends State<OnboardingWidget> {
  final PageController _pageController = PageController();
  int _currentPage = 0;

  static const String _onboardingCompleteKey = 'onboarding_complete';

  @override
  void dispose() {
    _pageController.dispose();
    super.dispose();
  }

  void _nextPage() {
    if (_currentPage < _pages.length - 1) {
      _pageController.nextPage(
        duration: const Duration(milliseconds: 300),
        curve: Curves.easeInOut,
      );
    } else {
      _completeOnboarding();
    }
  }

  void _previousPage() {
    if (_currentPage > 0) {
      _pageController.previousPage(
        duration: const Duration(milliseconds: 300),
        curve: Curves.easeInOut,
      );
    }
  }

  void _completeOnboarding() async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.setBool(_onboardingCompleteKey, true);
    widget.onComplete();
  }

  final List<OnboardingPage> _pages = [
    const OnboardingPage(
      icon: Icons.library_books_rounded,
      title: 'Welcome to Internet Archive Helper',
      description:
          'Your comprehensive companion for accessing the vast digital collection of the Internet Archive.',
      iconColor: Color(0xFF004B87),
    ),
    const OnboardingPage(
      icon: Icons.search_rounded,
      title: 'Find What You Need',
      description:
          'Search and browse millions of books, movies, music, software, and historical documents.',
      iconColor: Color(0xFFFF6B35),
    ),
    const OnboardingPage(
      icon: Icons.download_rounded,
      title: 'Download with Ease',
      description:
          'High-performance downloads with smart resume capability and progress tracking.',
      iconColor: Color(0xFF0088CC),
    ),
    const OnboardingPage(
      icon: Icons.mobile_friendly_rounded,
      title: 'Optimized for Mobile',
      description:
          'Touch-friendly interface designed for seamless browsing and downloading on your phone.',
      iconColor: Color(0xFF2E7D32),
    ),
    const OnboardingPage(
      icon: Icons.security_rounded,
      title: 'Privacy & Security',
      description:
          'Your data stays private. We only access what you choose to download from the Internet Archive.',
      iconColor: Color(0xFFED6C02),
    ),
  ];

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: SafeArea(
        child: Column(
          children: [
            // Progress indicator
            Container(
              padding: const EdgeInsets.all(16),
              child: Row(
                children: List.generate(
                  _pages.length,
                  (index) => Expanded(
                    child: Container(
                      height: 4,
                      margin: const EdgeInsets.symmetric(horizontal: 2),
                      decoration: BoxDecoration(
                        color: index <= _currentPage
                            ? Theme.of(context).primaryColor
                            : Theme.of(context).primaryColor.withValues(alpha: 0.3),
                        borderRadius: BorderRadius.circular(2),
                      ),
                    ),
                  ),
                ),
              ),
            ),

            // Page content
            Expanded(
              child: PageView.builder(
                controller: _pageController,
                onPageChanged: (index) {
                  setState(() {
                    _currentPage = index;
                  });
                },
                itemCount: _pages.length,
                itemBuilder: (context, index) {
                  return Padding(
                    padding: const EdgeInsets.all(24),
                    child: _pages[index],
                  );
                },
              ),
            ),

            // Navigation buttons
            Container(
              padding: const EdgeInsets.all(24),
              child: Row(
                mainAxisAlignment: MainAxisAlignment.spaceBetween,
                children: [
                  // Back button
                  if (_currentPage > 0)
                    TextButton(
                      onPressed: _previousPage,
                      child: const Text('Back'),
                    )
                  else
                    const SizedBox(width: 64), // Placeholder for alignment
                  // Skip button (only on first pages)
                  if (_currentPage < _pages.length - 1)
                    TextButton(
                      onPressed: _completeOnboarding,
                      child: const Text('Skip'),
                    ),

                  // Next/Get Started button
                  FilledButton(
                    onPressed: _nextPage,
                    child: Text(
                      _currentPage == _pages.length - 1
                          ? 'Get Started'
                          : 'Next',
                    ),
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class OnboardingPage extends StatelessWidget {
  final IconData icon;
  final String title;
  final String description;
  final Color iconColor;

  const OnboardingPage({
    super.key,
    required this.icon,
    required this.title,
    required this.description,
    required this.iconColor,
  });

  @override
  Widget build(BuildContext context) {
    return Column(
      mainAxisAlignment: MainAxisAlignment.center,
      children: [
        Container(
          padding: const EdgeInsets.all(32),
          decoration: BoxDecoration(
            color: iconColor.withValues(alpha: 0.1),
            shape: BoxShape.circle,
          ),
          child: Icon(icon, size: 80, color: iconColor),
        ),
        const SizedBox(height: 32),
        Text(
          title,
          style: Theme.of(
            context,
          ).textTheme.headlineSmall?.copyWith(fontWeight: FontWeight.bold),
          textAlign: TextAlign.center,
        ),
        const SizedBox(height: 16),
        Text(
          description,
          style: Theme.of(context).textTheme.bodyLarge?.copyWith(
            color: Theme.of(
              context,
            ).textTheme.bodyLarge?.color?.withValues(alpha: 0.7),
          ),
          textAlign: TextAlign.center,
        ),
      ],
    );
  }
}
