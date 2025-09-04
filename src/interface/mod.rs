//! User interface layer
//!
//! This module contains all user interface components including CLI, GUI, and interactive interfaces.

pub mod cli;
#[cfg(feature = "gui")]
pub mod gui;
pub mod interactive;

// Re-export commonly used interface types
pub use cli::*;
#[cfg(feature = "gui")]
pub use gui::*;
pub use interactive::*;
