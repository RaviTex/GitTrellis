//! GTK4 / libadwaita user interface.

pub mod app;
pub mod askpass;
pub mod branches_view;
pub mod changes_view;
pub mod clone_dialog;
pub mod commit_panel;
pub mod confirm;
pub mod conflict_view;
pub mod diagnostics_dialog;
pub mod diff_view;
pub mod error_dialog;
pub mod graph;
pub mod history_view;
pub mod remotes_view;
pub mod repo_page;
pub mod settings_dialog;
pub mod shortcuts;
pub mod stashes_view;
pub mod tags_view;
pub mod tasks;
pub mod util;
pub mod window;

use std::cell::RefCell;

use adw::prelude::*;

/// Keep secondary windows (preferences, diagnostics, clone) alive for as
/// long as they are open. GTK holds a reference to visible windows, but the
/// Rust wrapper would otherwise drop immediately.
pub fn widgets_keep_alive<W: IsA<gtk::Window>>(window: W) {
    thread_local! {
        static WINDOWS: RefCell<Vec<gtk::Window>> = const { RefCell::new(Vec::new()) };
    }
    let window = window.upcast::<gtk::Window>();
    window.connect_close_request(|_| gtk::glib::Propagation::Proceed);
    WINDOWS.with(|windows| {
        let mut windows = windows.borrow_mut();
        windows.push(window.clone());
        let len = windows.len();
        if len > 24 {
            windows.drain(0..len - 24);
        }
    });
}

/// Register the application stylesheet additions once, at startup.
pub fn install_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(
        r#"
        .monospace {
            font-family: "Source Code Pro", "JetBrains Mono", "Fira Code", monospace;
        }
        .commit-graph {
            background: transparent;
        }
        .file-row {
            padding: 3px 6px;
        }
        .status-chip {
            border-radius: 9999px;
            padding: 0 6px;
            font-size: 0.75em;
            font-weight: bold;
        }
        .status-chip.added { background-color: alpha(@success_color, 0.25); color: @success_color; }
        .status-chip.modified { background-color: alpha(@accent_color, 0.25); color: @accent_color; }
        .status-chip.deleted { background-color: alpha(@error_color, 0.25); color: @error_color; }
        .status-chip.renamed,
        .status-chip.copied { background-color: alpha(@warning_color, 0.28); color: @warning_color; }
        .status-chip.conflicted { background-color: alpha(@error_color, 0.35); color: @error_color; }
        .status-chip.untracked { background-color: alpha(@window_fg_color, 0.12); }
        .ref-badge {
            border-radius: 6px;
            padding: 0 5px;
            font-size: 0.78em;
            font-weight: bold;
        }
        .ref-badge.head { background-color: alpha(@accent_bg_color, 0.85); color: @accent_fg_color; }
        .ref-badge.branch { background-color: alpha(@accent_color, 0.22); color: @accent_color; }
        .ref-badge.remote { background-color: alpha(@warning_color, 0.22); color: @warning_color; }
        .ref-badge.tag { background-color: alpha(@success_color, 0.22); color: @success_color; }
        .commit-summary { font-weight: 500; }
        .op-banner {
            background-color: alpha(@warning_color, 0.18);
            border-radius: 10px;
            padding: 8px 12px;
        }
        .view-switcher-bar {
            background-color: alpha(@window_fg_color, 0.03);
            padding: 2px 0;
        }
        .search-bar {
            background-color: alpha(@window_fg_color, 0.05);
            border-radius: 10px;
            padding: 2px 4px;
        }
        "#,
    );
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
