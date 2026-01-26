//! Shared UI icons and emojis.
//!
//! This module provides common emoji constants used across the UI components
//! for consistent visual styling.

use console::Emoji;

// Status indicators
pub static CHECK: Emoji<'_, '_> = Emoji("✅ ", "[OK]");
pub static CROSS: Emoji<'_, '_> = Emoji("❌ ", "[ERR]");
pub static SPARKLE: Emoji<'_, '_> = Emoji("✨ ", "*");

// File indicators
pub static FOLDER: Emoji<'_, '_> = Emoji("📁 ", "");
pub static FILE_NEW: Emoji<'_, '_> = Emoji("📄 ", "+");
pub static FILE_MOD: Emoji<'_, '_> = Emoji("📝 ", "~");
pub static FILE_DEL: Emoji<'_, '_> = Emoji("🗑️  ", "-");

// Progress indicators
pub static PROGRESS: Emoji<'_, '_> = Emoji("📊 ", "[PROG]");
pub static BLOCKER: Emoji<'_, '_> = Emoji("🚧 ", "[BLOCK]");
pub static PIVOT: Emoji<'_, '_> = Emoji("🔄 ", "[PIVOT]");

// DAG-specific indicators
pub static WAVE: Emoji<'_, '_> = Emoji("🌊 ", "[W]");
pub static RUNNING: Emoji<'_, '_> = Emoji("▶️  ", "[>]");
pub static REVIEW: Emoji<'_, '_> = Emoji("🔍 ", "[R]");
pub static CLOCK: Emoji<'_, '_> = Emoji("⏱️  ", "[T]");
