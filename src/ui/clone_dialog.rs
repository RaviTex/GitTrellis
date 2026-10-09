//! Clone repository dialog with progress and cancellation.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;

use crate::core::Settings;
use crate::git::{GitError, Repository};

use super::tasks;

pub fn show(parent: &impl IsA<gtk::Window>, settings: Settings, on_cloned: Rc<dyn Fn(PathBuf)>) {
    let window = adw::Window::builder()
        .title("Clone Repository")
        .default_width(620)
        .default_height(460)
        .transient_for(parent)
        .build();

    let toolbar_view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    toolbar_view.add_top_bar(&header);

    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::SlideLeft)
        .build();

    // ---------------- form ----------------
    let form = gtk::Box::new(gtk::Orientation::Vertical, 10);
    form.set_margin_start(16);
    form.set_margin_end(16);
    form.set_margin_top(16);
    form.set_margin_bottom(16);
    form.set_valign(gtk::Align::Start);

    let url_entry = adw::EntryRow::builder()
        .title("Repository URL")
        .text(&settings.last_clone_url)
        .build();
    let dest_value: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    let dest_button = gtk::Button::builder()
        .label("Choose…")
        .valign(gtk::Align::Center)
        .build();
    let dest_row = adw::ActionRow::builder()
        .title("Destination")
        .subtitle(&settings.clone_default_dir)
        .build();
    dest_row.add_suffix(&dest_button);
    let name_entry = adw::EntryRow::builder()
        .title("Directory name")
        .build();
    let branch_entry = adw::EntryRow::builder()
        .title("Branch (optional)")
        .build();
    let shallow_row = adw::SwitchRow::builder()
        .title("Shallow clone")
        .subtitle("Fetch only the latest revision (faster for large repositories)")
        .build();
    let depth_row = adw::SpinRow::builder()
        .title("Depth")
        .adjustment(&gtk::Adjustment::new(1.0, 1.0, 1000.0, 1.0, 10.0, 0.0))
        .sensitive(false)
        .build();
    {
        let depth_row = depth_row.clone();
        shallow_row
            .bind_property("active", &depth_row, "sensitive")
            .sync_create()
            .build();
    }

    form.append(&url_entry);
    form.append(&dest_row);
    form.append(&name_entry);
    form.append(&branch_entry);

    let advanced = gtk::Expander::builder().label("Advanced").build();
    let advanced_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
    advanced_box.append(&shallow_row);
    advanced_box.append(&depth_row);
    advanced.set_child(Some(&advanced_box));
    form.append(&advanced);

    // Derive the directory name from the URL.
    {
        let name_entry = name_entry.clone();
        let url_for_change = url_entry.clone();
        url_entry.connect_changed(move |_| {
            if !name_entry.text().is_empty() {
                return;
            }
            let url = url_for_change.text().to_string();
            if url.trim().is_empty() {
                return;
            }
            let guess = guess_name(&url);
            if !guess.is_empty() {
                name_entry.set_text(&guess);
            }
        });
    }

    let dest_clone = Rc::clone(&dest_value);
    let dest_row_clone = dest_row.clone();
    let dest_button_clone = dest_button.clone();
    let default_dir = settings.clone_default_dir.clone();
    dest_button.connect_clicked(move |_| {
        let dialog = gtk::FileDialog::builder()
            .title("Choose destination folder")
            .initial_folder(&gio::File::for_path(&default_dir))
            .build();
        let dest_row = dest_row_clone.clone();
        let dest_value = Rc::clone(&dest_clone);
        let parent = dest_button_clone.root().and_downcast::<gtk::Window>();
        dialog.select_folder(parent.as_ref(), gio::Cancellable::NONE, move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    *dest_value.borrow_mut() = path.to_string_lossy().to_string();
                    dest_row.set_subtitle(&path.to_string_lossy());
                }
            }
        });
    });

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    buttons.set_margin_top(6);
    let clone_button = gtk::Button::with_label("Clone");
    clone_button.add_css_class("suggested-action");
    buttons.append(&clone_button);
    form.append(&buttons);

    stack.add_named(&form, Some("form"));

    // ---------------- progress ----------------
    let progress = gtk::Box::new(gtk::Orientation::Vertical, 10);
    progress.set_margin_start(16);
    progress.set_margin_end(16);
    progress.set_margin_top(16);
    progress.set_margin_bottom(16);
    let status = gtk::Label::builder()
        .label("Preparing clone…")
        .xalign(0.0)
        .build();
    status.add_css_class("heading");
    let bar = gtk::ProgressBar::builder().show_text(false).build();
    let cancel = gtk::Button::with_label("Cancel");
    cancel.add_css_class("flat");
    let log_view = gtk::TextView::builder()
        .editable(false)
        .monospace(true)
        .wrap_mode(gtk::WrapMode::WordChar)
        .build();
    let log_scroll = gtk::ScrolledWindow::builder()
        .child(&log_view)
        .vexpand(true)
        .has_frame(true)
        .build();
    let progress_buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    progress_buttons.set_halign(gtk::Align::End);
    progress_buttons.append(&cancel);
    progress.append(&status);
    progress.append(&bar);
    progress.append(&log_scroll);
    progress.append(&progress_buttons);
    stack.add_named(&progress, Some("progress"));
    stack.set_visible_child_name("form");

    toolbar_view.set_content(Some(&stack));
    window.set_content(Some(&toolbar_view));

    // ---------------- clone action ----------------
    let active_op: Rc<RefCell<Option<tasks::StreamingOp>>> = Rc::new(RefCell::new(None));
    {
        let window = window.clone();
        let stack = stack.clone();
        let status = status.clone();
        let bar = bar.clone();
        let log_view = log_view.clone();
        let url_entry = url_entry.clone();
        let name_entry = name_entry.clone();
        let branch_entry = branch_entry.clone();
        let dest_value = Rc::clone(&dest_value);
        let dest_row = dest_row.clone();
        let shallow_row = shallow_row.clone();
        let depth_row = depth_row.clone();
        let default_dir = settings.clone_default_dir.clone();
        let active_op = Rc::clone(&active_op);
        let on_cloned = Rc::clone(&on_cloned);
        clone_button.connect_clicked(move |_| {
            let stack = stack.clone();
            let status = status.clone();
            let log_view = log_view.clone();
            let on_cloned = Rc::clone(&on_cloned);
            let url = url_entry.text().trim().to_string();
            if url.is_empty() {
                status.set_label("Please enter a repository URL.");
                return;
            }
            let name = name_entry.text().trim().to_string();
            let base = if dest_value.borrow().is_empty() {
                default_dir.clone()
            } else {
                dest_value.borrow().clone()
            };
            let dest = PathBuf::from(base).join(if name.is_empty() {
                guess_name(&url)
            } else {
                name
            });
            dest_row.set_subtitle(&dest.to_string_lossy());
            let branch = branch_entry.text().trim().to_string();
            let branch = if branch.is_empty() { None } else { Some(branch) };
            let depth = if shallow_row.is_active() {
                Some(depth_row.value() as u32)
            } else {
                None
            };

            stack.set_visible_child_name("progress");
            status.set_label("Cloning…");
            bar.pulse();
            log_view.buffer().set_text("");
            let dest_clone = dest.clone();
            let log_view_ui = log_view.clone();
            let status_ui = status.clone();
            let window_ui = window.clone();
            let op = tasks::run_streaming(
                move |cancel, progress| {
                    Repository::clone(&url, &dest_clone, branch.as_deref(), depth, Some(cancel), progress)
                        .map(|_| ())
                },
                move |line| {
                    status_ui.set_label(&line);
                    let mut end = log_view_ui.buffer().end_iter();
                    log_view_ui
                        .buffer()
                        .insert(&mut end, &format!("{line}\n"));
                    let mut end = log_view_ui.buffer().end_iter();
                    log_view_ui.scroll_to_iter(&mut end, 0.0, false, 0.0, 0.0);
                },
                move |result| match result {
                    Ok(()) => {
                        window_ui.close();
                        on_cloned(dest.clone());
                    }
                    Err(GitError::Cancelled) => {
                        status.set_label("Clone cancelled.");
                        let _ = std::fs::remove_dir_all(&dest);
                        stack.set_visible_child_name("form");
                    }
                    Err(e) => {
                        status.set_label("Clone failed");
                        let mut end = log_view.buffer().end_iter();
                        log_view
                            .buffer()
                            .insert(&mut end, &format!("\n{}\n", e.technical_details()));
                    }
                },
            );
            *active_op.borrow_mut() = Some(op);
        });
    }
    {
        let active_op = Rc::clone(&active_op);
        cancel.connect_clicked(move |_| {
            if let Some(op) = active_op.borrow().as_ref() {
                op.cancel();
            }
        });
    }

    crate::ui::widgets_keep_alive(window.clone());
    window.present();
}

fn guess_name(url: &str) -> String {
    let trimmed = url.trim_end_matches('/').trim_end_matches(".git");
    let last = trimmed
        .rsplit(['/', ':'])
        .next()
        .unwrap_or(trimmed)
        .to_string();
    last
}

/// Initialize a repository from a chosen directory.
pub fn show_init(parent: &impl IsA<gtk::Window>, on_initialized: Rc<dyn Fn(PathBuf)>) {
    let window = adw::Window::builder()
        .title("Initialize Repository")
        .default_width(560)
        .default_height(320)
        .transient_for(parent)
        .build();
    let toolbar_view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    toolbar_view.add_top_bar(&header);

    let form = gtk::Box::new(gtk::Orientation::Vertical, 10);
    form.set_margin_start(16);
    form.set_margin_end(16);
    form.set_margin_top(16);
    form.set_margin_bottom(16);
    form.set_valign(gtk::Align::Start);
    let dir_value: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    let choose = gtk::Button::builder()
        .label("Choose…")
        .valign(gtk::Align::Center)
        .build();
    let dir_row = adw::ActionRow::builder()
        .title("Directory")
        .subtitle("No directory selected")
        .build();
    dir_row.add_suffix(&choose);
    let branch_entry = adw::EntryRow::builder()
        .title("Initial branch name")
        .text("main")
        .build();
    let create = gtk::Button::with_label("Initialize");
    create.add_css_class("suggested-action");
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    buttons.append(&create);
    form.append(&dir_row);
    form.append(&branch_entry);
    form.append(&buttons);
    toolbar_view.set_content(Some(&form));
    window.set_content(Some(&toolbar_view));

    {
        let dir_value = Rc::clone(&dir_value);
        let dir_row = dir_row.clone();
        choose.connect_clicked(move |button| {
            let dialog = gtk::FileDialog::builder()
                .title("Choose a directory to initialize")
                .build();
            let parent = button.root().and_downcast::<gtk::Window>();
            let dir_row = dir_row.clone();
            let dir_value = Rc::clone(&dir_value);
            dialog.select_folder(parent.as_ref(), gio::Cancellable::NONE, move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        *dir_value.borrow_mut() = path.to_string_lossy().to_string();
                        dir_row.set_subtitle(&path.to_string_lossy());
                    }
                }
            });
        });
    }
    {
        let window = window.clone();
        let dir_value = Rc::clone(&dir_value);
        create.connect_clicked(move |_| {
            let dir = PathBuf::from(dir_value.borrow().clone());
            if dir.as_os_str().is_empty() {
                return;
            }
            let branch = branch_entry.text().trim().to_string();
            let branch = if branch.is_empty() { None } else { Some(branch) };
            match Repository::init(&dir, branch.as_deref()) {
                Ok(_) => {
                    window.close();
                    on_initialized(dir.clone());
                }
                Err(e) => {
                    crate::log_error!("init failed: {}", e.technical_details());
                    if let Some(parent) = window.transient_for() {
                        super::error_dialog::show(&parent, &e);
                    }
                }
            }
        });
    }

    crate::ui::widgets_keep_alive(window.clone());
    window.present();
}
