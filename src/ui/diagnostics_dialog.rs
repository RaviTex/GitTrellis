//! Diagnostics: in-app log viewer and environment information.

use adw::prelude::*;

use crate::core::logging;

pub fn show(parent: &impl IsA<gtk::Window>) {
    let window = adw::Window::builder()
        .title("Diagnostics")
        .default_width(780)
        .default_height(520)
        .transient_for(parent)
        .build();

    let toolbar_view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();

    let copy_button = gtk::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text("Copy all log lines")
        .build();
    let clear_button = gtk::Button::builder()
        .icon_name("user-trash-symbolic")
        .tooltip_text("Clear the in-memory log")
        .build();
    let folder_button = gtk::Button::builder()
        .icon_name("folder-open-symbolic")
        .tooltip_text("Open the log folder")
        .build();
    header.pack_end(&copy_button);
    header.pack_end(&clear_button);
    header.pack_end(&folder_button);
    toolbar_view.add_top_bar(&header);

    let info = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .margin_top(8)
        .margin_bottom(4)
        .margin_start(12)
        .margin_end(12)
        .build();
    info.add_css_class("dim-label");
    let log_path = logging::log_file_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "not available".into());
    info.set_label(&format!(
        "Stratum {} · Git {} · log file: {}",
        env!("CARGO_PKG_VERSION"),
        crate::git::Git::system()
            .version()
            .unwrap_or_else(|_| "not found".into()),
        log_path
    ));

    let view = gtk::TextView::builder()
        .editable(false)
        .monospace(true)
        .wrap_mode(gtk::WrapMode::WordChar)
        .build();
    view.buffer().set_text(&logging::export_text());
    let scroll = gtk::ScrolledWindow::builder()
        .child(&view)
        .vexpand(true)
        .has_frame(true)
        .margin_start(12)
        .margin_end(12)
        .margin_bottom(12)
        .build();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 4);
    content.append(&info);
    content.append(&scroll);
    toolbar_view.set_content(Some(&content));
    window.set_content(Some(&toolbar_view));

    copy_button.connect_clicked(move |button| {
        button.clipboard().set_text(&logging::export_text());
    });
    {
        let view = view.clone();
        clear_button.connect_clicked(move |_| {
            logging::clear();
            view.buffer().set_text("");
        });
    }
    folder_button.connect_clicked(move |_| {
        let dir = crate::core::Settings::state_dir().join("logs");
        let _ = std::fs::create_dir_all(&dir);
        if let Err(e) = crate::ui::util::open_path(&dir) {
            crate::log_warn!("could not open log folder: {e}");
        }
    });

    crate::ui::widgets_keep_alive(window.clone());
    window.present();
}
