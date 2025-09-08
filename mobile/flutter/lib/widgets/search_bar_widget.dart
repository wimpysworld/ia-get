import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import '../services/ia_get_service.dart';

class SearchBarWidget extends StatefulWidget {
  const SearchBarWidget({super.key});

  @override
  State<SearchBarWidget> createState() => _SearchBarWidgetState();
}

class _SearchBarWidgetState extends State<SearchBarWidget> {
  final TextEditingController _controller = TextEditingController();
  final FocusNode _focusNode = FocusNode();

  @override
  void dispose() {
    _controller.dispose();
    _focusNode.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.all(16),
      child: Row(
        children: [
          Expanded(
            child: TextField(
              controller: _controller,
              focusNode: _focusNode,
              decoration: InputDecoration(
                hintText: 'Enter Internet Archive identifier',
                prefixIcon: const Icon(Icons.search),
                suffixIcon: _controller.text.isNotEmpty
                    ? IconButton(
                        icon: const Icon(Icons.clear),
                        onPressed: () {
                          _controller.clear();
                          setState(() {});
                        },
                      )
                    : null,
                border: OutlineInputBorder(
                  borderRadius: BorderRadius.circular(12),
                ),
              ),
              onChanged: (value) => setState(() {}),
              onSubmitted: _searchArchive,
            ),
          ),
          const SizedBox(width: 8),
          Consumer<IaGetService>(
            builder: (context, service, child) {
              return ElevatedButton(
                onPressed: service.isLoading || _controller.text.trim().isEmpty
                    ? null
                    : () => _searchArchive(_controller.text),
                child: service.isLoading
                    ? const SizedBox(
                        width: 16,
                        height: 16,
                        child: CircularProgressIndicator(
                          strokeWidth: 2,
                          valueColor: AlwaysStoppedAnimation<Color>(Colors.white),
                        ),
                      )
                    : const Text('Search'),
              );
            },
          ),
        ],
      ),
    );
  }

  void _searchArchive(String identifier) {
    if (identifier.trim().isEmpty) return;
    
    _focusNode.unfocus();
    context.read<IaGetService>().fetchMetadata(identifier.trim());
  }
}