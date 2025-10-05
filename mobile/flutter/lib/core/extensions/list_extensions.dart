/// List extensions for enhanced functionality
extension ListExtensions<T> on List<T> {
  /// Splits list into chunks of specified size
  List<List<T>> chunked(int size) {
    final chunks = <List<T>>[];
    for (var i = 0; i < length; i += size) {
      chunks.add(sublist(i, i + size > length ? length : i + size));
    }
    return chunks;
  }

  /// Groups list elements by a key selector
  Map<K, List<T>> groupBy<K>(K Function(T) keySelector) {
    final map = <K, List<T>>{};
    for (final element in this) {
      final key = keySelector(element);
      (map[key] ??= []).add(element);
    }
    return map;
  }

  /// Returns distinct elements
  List<T> distinct() {
    return toSet().toList();
  }

  /// Sorts list by key
  List<T> sortedBy<K extends Comparable>(K Function(T) keySelector) {
    final copy = [...this];
    copy.sort((a, b) => keySelector(a).compareTo(keySelector(b)));
    return copy;
  }

  /// Sums numeric values
  num sumBy(num Function(T) selector) {
    return fold(0, (sum, element) => sum + selector(element));
  }

  /// Returns first element or null
  T? get firstOrNull => isEmpty ? null : first;

  /// Returns last element or null
  T? get lastOrNull => isEmpty ? null : last;
}
