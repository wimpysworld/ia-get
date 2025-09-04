//! Format listing and help functionality
//!
//! This module provides functionality to list available file format categories
//! and their associated file extensions. It can be used both as a standalone
//! command and as help text in interactive mode.

use crate::utilities::filters::{FileFormats, FormatCategory};
use colored::Colorize;

/// Display all available format categories and their descriptions
pub fn list_format_categories() {
    println!(
        "{}",
        "📁 Internet Archive File Format Categories".green().bold()
    );
    println!();

    let file_formats = FileFormats::new();

    for category in FormatCategory::all() {
        let formats = file_formats.get_formats(&category);
        let sample_formats: Vec<String> = formats.iter().take(5).cloned().collect();

        println!(
            "  {} {}",
            format!("{}:", category.display_name()).cyan().bold(),
            category.description().dimmed()
        );

        println!(
            "    {} {}{}",
            "Examples:".yellow(),
            sample_formats.join(", "),
            if formats.len() > 5 {
                format!(" (+{} more)", formats.len() - 5)
            } else {
                String::new()
            }
        );
        println!();
    }
}

/// Display detailed format information for specific categories
pub fn list_detailed_formats(categories: Option<Vec<String>>) {
    println!(
        "{}",
        "📁 Internet Archive File Formats (Detailed)".green().bold()
    );
    println!();

    let file_formats = FileFormats::new();
    let categories_to_show = if let Some(cats) = categories {
        let mut result = Vec::new();
        for cat_name in cats {
            let cat_name_lower = cat_name.to_lowercase();
            for category in FormatCategory::all() {
                if category.display_name().to_lowercase() == cat_name_lower {
                    result.push(category);
                    break;
                }
            }
        }
        result
    } else {
        FormatCategory::all()
    };

    for category in categories_to_show {
        let formats = file_formats.get_formats(&category);

        println!(
            "  {} {} ({} formats)",
            format!("{}:", category.display_name()).cyan().bold(),
            category.description().dimmed(),
            formats.len().to_string().yellow()
        );

        // Print formats in a nice grid layout
        let mut current_line = String::new();
        for (i, format) in formats.iter().enumerate() {
            if current_line.len() + format.len() + 2 > 70 {
                println!("    {}", current_line);
                current_line = String::new();
            }
            if i > 0 && !current_line.is_empty() {
                current_line.push_str(", ");
            }
            current_line.push_str(format);
        }
        if !current_line.is_empty() {
            println!("    {}", current_line);
        }

        println!();
    }
}

/// Display common format presets
pub fn list_format_presets() {
    println!("{}", "🎯 Common Format Presets".green().bold());
    println!();

    let presets = FileFormats::get_common_presets();

    for (name, description, extensions) in presets {
        println!(
            "  {} {}",
            format!("{}:", name).cyan().bold(),
            description.dimmed()
        );

        if !extensions.is_empty() {
            println!("    {} {}", "Formats:".yellow(), extensions.join(", "));
        } else {
            println!("    {} Use with --exclude-formats", "Usage:".yellow());
        }
        println!();
    }
}

/// Display usage examples for format filtering
pub fn show_format_usage_examples() {
    println!("{}", "💡 Format Filtering Usage Examples".green().bold());
    println!();

    println!("  {} Include only documents:", "Example 1:".cyan().bold());
    println!(
        "    {}",
        "ia-get --include-formats documents https://archive.org/details/example".dimmed()
    );
    println!();

    println!(
        "  {} Include multiple categories:",
        "Example 2:".cyan().bold()
    );
    println!(
        "    {}",
        "ia-get --include-formats documents,images https://archive.org/details/example".dimmed()
    );
    println!();

    println!(
        "  {} Exclude metadata and system files:",
        "Example 3:".cyan().bold()
    );
    println!(
        "    {}",
        "ia-get --exclude-formats metadata https://archive.org/details/example".dimmed()
    );
    println!();

    println!(
        "  {} Combine category and extension filtering:",
        "Example 4:".cyan().bold()
    );
    println!("    {}", "ia-get --include-formats documents --exclude-ext log,tmp https://archive.org/details/example".dimmed());
    println!();

    println!("  {} Use with size filtering:", "Example 5:".cyan().bold());
    println!(
        "    {}",
        "ia-get --include-formats video --max-file-size 100MB https://archive.org/details/example"
            .dimmed()
    );
    println!();
}

/// Complete format help display (combines all sections)
pub fn show_complete_format_help() {
    list_format_categories();
    println!("{}", "─".repeat(80).dimmed());
    println!();
    list_format_presets();
    println!("{}", "─".repeat(80).dimmed());
    println!();
    show_format_usage_examples();
}
