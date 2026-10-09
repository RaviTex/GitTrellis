//! Application entry point: GTK application setup, logging and window
//! management.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::core::{logging, Settings, Settings as AppSettings};
use crate::log_info;
use crate::ui::window::MainWindow;

pub const APP_ID: &str = "io.github.gittrellis.GitTrellis";

thread_local! {
    static MAIN_WINDOW: RefCell<Option<Rc<MainWindow>>> = const { RefCell::new(None) };
}

/// Run the application. Returns the process exit code.
pub fn run() -> glib::ExitCode {
    let settings = Settings::load();
    logging::init(level_from(&settings.log_level));
    log_info!(
        "GitTrellis {} starting (config: {})",
        env!("CARGO_PKG_VERSION"),
        Settings::config_file().display()
    );

    // A distinct application id can be requested for development builds so
    // they can run alongside an installed copy.
    let app_id = std::env::var("GITTRELLIS_APP_ID").unwrap_or_else(|_| APP_ID.to_string());
    let app = adw::Application::builder()
        .application_id(&app_id)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    app.connect_startup(|_| {
        crate::ui::install_css();
    });

    app.connect_activate(|app| {
        ensure_window(app);
    });

    app.connect_open(|app, files, _hint| {
        let window = ensure_window(app);
        for file in files {
            if let Some(path) = file.path() {
                window.open_path(path, true);
            }
        }
    });

    app.run()
}

fn ensure_window(app: &adw::Application) -> Rc<MainWindow> {
    if let Some(window) = MAIN_WINDOW.with(|w| w.borrow().clone()) {
        window.window().present();
        return window;
    }
    let settings = AppSettings::load();
    let window = MainWindow::new(app, settings);
    MAIN_WINDOW.with(|w| *w.borrow_mut() = Some(Rc::clone(&window)));
    window
}

fn level_from(name: &str) -> logging::Level {
    match name {
        "error" => logging::Level::Error,
        "warn" => logging::Level::Warn,
        "debug" => logging::Level::Debug,
        _ => logging::Level::Info,
    }
}
