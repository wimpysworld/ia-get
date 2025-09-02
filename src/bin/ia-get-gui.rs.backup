//! GUI entry point for ia-get
//!
//! Provides a graphical user interface for the Internet Archive downloader.

use ia_get::gui::IaGetApp;

fn main() -> Result<(), eframe::Error> {
    // Set up logging with better error handling
    if let Err(e) = env_logger::try_init() {
        eprintln!("Warning: Failed to initialize logger: {}", e);
    }

    // Create a tokio runtime for async operations
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("Error: Failed to create Tokio runtime: {}", e);
            eprintln!("This error might occur if the system cannot create threads.");
            eprintln!("Press Enter to exit...");
            let mut input = String::new();
            let _ = std::io::stdin().read_line(&mut input);
            std::process::exit(1);
        }
    };

    // Run the GUI application
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 700.0])
            .with_min_inner_size([800.0, 600.0])
            .with_title("ia-get - Internet Archive Downloader")
            .with_icon(load_icon()),
        ..Default::default()
    };

    // Enter the async runtime context and run the GUI
    let _guard = rt.enter();

    // Run with better error handling
    let result = eframe::run_native(
        "ia-get GUI",
        options,
        Box::new(|cc| Ok(Box::new(IaGetApp::new(cc)))),
    );

    // If we get here and there was an error, show it to the user
    if let Err(ref e) = result {
        eprintln!("Error starting GUI: {}", e);
        eprintln!("This might be due to missing graphics drivers or display issues.");
        eprintln!("Press Enter to exit...");
        let mut input = String::new();
        let _ = std::io::stdin().read_line(&mut input);
    }

    result
}

/// Load application icon
fn load_icon() -> egui::IconData {
    // For now, create a simple icon programmatically
    // In a real implementation, you'd load from an icon file
    let size = 32;
    let mut rgba = Vec::with_capacity(size * size * 4);

    for y in 0..size {
        for x in 0..size {
            // Create a simple gradient icon
            let r = (x * 255 / size) as u8;
            let g = (y * 255 / size) as u8;
            let b = 128u8;
            let a = 255u8;

            rgba.push(r);
            rgba.push(g);
            rgba.push(b);
            rgba.push(a);
        }
    }

    egui::IconData {
        rgba,
        width: size as u32,
        height: size as u32,
    }
}
