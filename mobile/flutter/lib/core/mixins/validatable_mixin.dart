import 'package:flutter/widgets.dart';

/// Mixin that provides form validation functionality
mixin ValidatableMixin on State {
  final _validationErrors = <String, String>{};
  final _validators = <String, List<String? Function(String?)>>{};
  bool _autovalidate = false;
  
  /// Get all validation errors
  Map<String, String> get validationErrors => Map.unmodifiable(_validationErrors);
  
  /// Check if form is valid
  bool get isValid => _validationErrors.isEmpty;
  
  /// Check if form has errors
  bool get hasErrors => _validationErrors.isNotEmpty;
  
  /// Enable/disable auto-validation
  bool get autovalidate => _autovalidate;
  set autovalidate(bool value) {
    _autovalidate = value;
    if (value) {
      validateAll();
    }
  }
  
  /// Register a field with validators
  void registerField(String field, List<String? Function(String?)> validators) {
    _validators[field] = validators;
  }
  
  /// Validate a specific field
  String? validateField(String field, String? value) {
    final validators = _validators[field];
    if (validators == null) return null;
    
    for (final validator in validators) {
      final error = validator(value);
      if (error != null) {
        _validationErrors[field] = error;
        if (mounted) setState(() {});
        return error;
      }
    }
    
    _validationErrors.remove(field);
    if (mounted) setState(() {});
    return null;
  }
  
  /// Validate all fields
  bool validateAll() {
    bool allValid = true;
    
    for (final field in _validators.keys) {
      final error = validateField(field, null);
      if (error != null) {
        allValid = false;
      }
    }
    
    return allValid;
  }
  
  /// Get error for specific field
  String? getError(String field) => _validationErrors[field];
  
  /// Clear error for specific field
  void clearError(String field) {
    _validationErrors.remove(field);
    if (mounted) setState(() {});
  }
  
  /// Clear all errors
  void clearAllErrors() {
    _validationErrors.clear();
    if (mounted) setState(() {});
  }
  
  /// Set custom error
  void setError(String field, String error) {
    _validationErrors[field] = error;
    if (mounted) setState(() {});
  }
  
  /// Common validators
  static String? Function(String?) required([String? message]) {
    return (value) {
      if (value == null || value.trim().isEmpty) {
        return message ?? 'This field is required';
      }
      return null;
    };
  }
  
  static String? Function(String?) email([String? message]) {
    return (value) {
      if (value == null || value.isEmpty) return null;
      
      final emailRegex = RegExp(r'^[\w-\.]+@([\w-]+\.)+[\w-]{2,4}$');
      if (!emailRegex.hasMatch(value)) {
        return message ?? 'Please enter a valid email';
      }
      return null;
    };
  }
  
  static String? Function(String?) minLength(int length, [String? message]) {
    return (value) {
      if (value == null || value.isEmpty) return null;
      
      if (value.length < length) {
        return message ?? 'Must be at least $length characters';
      }
      return null;
    };
  }
  
  static String? Function(String?) maxLength(int length, [String? message]) {
    return (value) {
      if (value == null || value.isEmpty) return null;
      
      if (value.length > length) {
        return message ?? 'Must be no more than $length characters';
      }
      return null;
    };
  }
  
  static String? Function(String?) pattern(RegExp regex, [String? message]) {
    return (value) {
      if (value == null || value.isEmpty) return null;
      
      if (!regex.hasMatch(value)) {
        return message ?? 'Invalid format';
      }
      return null;
    };
  }
  
  static String? Function(String?) range(num min, num max, [String? message]) {
    return (value) {
      if (value == null || value.isEmpty) return null;
      
      final number = num.tryParse(value);
      if (number == null) {
        return 'Please enter a valid number';
      }
      
      if (number < min || number > max) {
        return message ?? 'Must be between $min and $max';
      }
      return null;
    };
  }
}
