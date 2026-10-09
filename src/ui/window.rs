//! Main application window: repository sidebar, welcome page and repo pages.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::core::Settings;
use crate::git::Repository;

use super::{clone_dialog, diagnostics_dialog, repo_page::RepoPage, settings_dialog, shortcuts, util};

struct RepoSession {
    path: PathBuf,
    page: Rc<RepoPage>,
}

pub struct MainWindow {
    window: adw::ApplicationWindow,
    app: adw::Application,
    split: adw::OverlaySplitView,
    open_list: gtk::ListBox,
    recent_list: gtk::ListBox,
    content_stack: gtk::Stack,
    sessions: RefCell<Vec<RepoSession>>,
    settings: RefCell<Settings>,
    rebuilding: Cell<bool>,
    opening: RefCell<std::collections::HashSet<PathBuf>>,
}

impl MainWindow {
    pub fn new(app: &adw::Application, settings: Settings) -> Rc<Self> {
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Stratum")
            .default_width(settings.window_width.max(900))
            .default_height(settings.window_height.max(600))
            .width_request(720)
            .height_request(480)
            .build();

        let split = adw::OverlaySplitView::builder()
            .min_sidebar_width(260.0)
            .max_sidebar_width(300.0)
            .show_sidebar(true)
            .build();

        // ---------------- sidebar ----------------
        let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sidebar.set_size_request(272, -1);
        let sidebar_header = adw::HeaderBar::new();
        sidebar_header.set_show_title(false);
        let sidebar_title = gtk::Label::builder().label("Stratum").build();
        sidebar_title.add_css_class("heading");
        let init_button = util::icon_button("document-new-symbolic", "Initialize a repository here");
        let open_button = util::icon_button("folder-open-symbolic", "Open repository (Ctrl+O)");
        let clone_button = util::icon_button("cloud-download-symbolic", "Clone repository (Ctrl+Shift+O)");
        // Order: application name, then small action icons.
        sidebar_header.pack_start(&sidebar_title);
        sidebar_header.pack_start(&init_button);
        sidebar_header.pack_start(&open_button);
        sidebar_header.pack_start(&clone_button);

        let (open_header, _) = util::list_header("Open Repositories");
        let open_list = gtk::ListBox::new();
        open_list.add_css_class("navigation-sidebar");
        open_list.set_selection_mode(gtk::SelectionMode::Single);

        let (recent_header, _) = util::list_header("Recent");
        let recent_list = gtk::ListBox::new();
        recent_list.add_css_class("navigation-sidebar");
        recent_list.set_selection_mode(gtk::SelectionMode::None);

        let sidebar_content = gtk::Box::new(gtk::Orientation::Vertical, 2);
        sidebar_content.set_margin_start(6);
        sidebar_content.set_margin_end(6);
        sidebar_content.append(&open_header);
        sidebar_content.append(&open_list);
        sidebar_content.append(&recent_header);
        sidebar_content.append(&recent_list);
        let sidebar_scroll = gtk::ScrolledWindow::builder()
            .child(&sidebar_content)
            .vexpand(true)
            .build();

        sidebar.append(&sidebar_header);
        sidebar.append(&sidebar_scroll);
        split.set_sidebar(Some(&sidebar));

        // ---------------- content ----------------
        let content_stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .vexpand(true)
            .hexpand(true)
            .build();

        let welcome = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let welcome_header = adw::HeaderBar::new();
        welcome_header.set_show_title(false);
        let welcome_box = gtk::Box::new(gtk::Orientation::Vertical, 12);
        welcome_box.set_valign(gtk::Align::Center);
        welcome_box.set_halign(gtk::Align::Center);
        let welcome_icon = gtk::Image::from_icon_name("git-symbolic");
        welcome_icon.set_pixel_size(96);
        welcome_icon.add_css_class("dim-label");
        let welcome_title = gtk::Label::builder().label("Stratum").build();
        welcome_title.add_css_class("title-1");
        let welcome_sub = gtk::Label::builder()
            .label("A native Git client for Linux")
            .build();
        welcome_sub.add_css_class("dim-label");
        let welcome_buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        welcome_buttons.set_halign(gtk::Align::Center);
        let w_open = gtk::Button::with_label("Open Repository");
        w_open.add_css_class("suggested-action");
        let w_clone = gtk::Button::with_label("Clone Repository");
        let w_init = gtk::Button::with_label("Initialize Repository");
        welcome_buttons.append(&w_open);
        welcome_buttons.append(&w_clone);
        welcome_buttons.append(&w_init);
        let hint = gtk::Label::builder()
            .label("Tip: drag a repository folder onto the window, or press Ctrl+O.")
            .build();
        hint.add_css_class("dim-label");
        welcome_box.append(&welcome_icon);
        welcome_box.append(&welcome_title);
        welcome_box.append(&welcome_sub);
        welcome_box.append(&welcome_buttons);
        welcome_box.append(&hint);
        let welcome_scroll = gtk::ScrolledWindow::builder()
            .child(&welcome_box)
            .vexpand(true)
            .build();
        welcome.append(&welcome_header);
        welcome.append(&welcome_scroll);
        content_stack.add_named(&welcome, Some("welcome"));

        split.set_content(Some(&content_stack));
        window.set_content(Some(&split));

        let sidebar_toggle = util::icon_button("sidebar-show-symbolic", "Toggle sidebar (F9)");
        {
            let split = split.clone();
            sidebar_toggle.connect_clicked(move |_| {
                split.set_show_sidebar(!split.shows_sidebar());
            });
        }
        welcome_header.pack_start(&sidebar_toggle);

        let window_rc = Rc::new(Self {
            window: window.clone(),
            app: app.clone(),
            split: split.clone(),
            open_list: open_list.clone(),
            recent_list: recent_list.clone(),
            content_stack: content_stack.clone(),
            sessions: RefCell::new(Vec::new()),
            settings: RefCell::new(settings.clone()),
            rebuilding: Cell::new(false),
            opening: RefCell::new(std::collections::HashSet::new()),
        });

        // Wire buttons
        {
            let weak = Rc::downgrade(&window_rc);
            open_button.connect_clicked(move |_| {
                if let Some(w) = weak.upgrade() {
                    w.open_repository_dialog();
                }
            });
        }
        {
            let weak = Rc::downgrade(&window_rc);
            clone_button.connect_clicked(move |_| {
                if let Some(w) = weak.upgrade() {
                    w.clone_dialog();
                }
            });
        }
        {
            let weak = Rc::downgrade(&window_rc);
            init_button.connect_clicked(move |_| {
                if let Some(w) = weak.upgrade() {
                    w.init_dialog();
                }
            });
        }
        {
            let weak = Rc::downgrade(&window_rc);
            w_open.connect_clicked(move |_| {
                if let Some(w) = weak.upgrade() {
                    w.open_repository_dialog();
                }
            });
        }
        {
            let weak = Rc::downgrade(&window_rc);
            w_clone.connect_clicked(move |_| {
                if let Some(w) = weak.upgrade() {
                    w.clone_dialog();
                }
            });
        }
        {
            let weak = Rc::downgrade(&window_rc);
            w_init.connect_clicked(move |_| {
                if let Some(w) = weak.upgrade() {
                    w.init_dialog();
                }
            });
        }
        {
            let weak = Rc::downgrade(&window_rc);
            open_list.connect_row_selected(move |_, row| {
                let Some(row) = row else { return };
                let Some(w) = weak.upgrade() else { return };
                if w.rebuilding.get() {
                    return;
                }
                w.activate_row(row);
            });
        }
        {
            let weak = Rc::downgrade(&window_rc);
            recent_list.connect_row_activated(move |_, row| {
                let Some(w) = weak.upgrade() else { return };
                if let Some(path) = row.widget_name().strip_prefix("path:").map(PathBuf::from) {
                    w.open_path(path, true);
                }
            });
        }

        // Drag and drop a folder onto the window.
        {
            let weak = Rc::downgrade(&window_rc);
            let drop = gtk::DropTarget::new(
                gtk::gdk::FileList::static_type(),
                gtk::gdk::DragAction::COPY,
            );
            drop.connect_drop(move |_, value, _, _| {
                let Ok(list) = value.get::<gtk::gdk::FileList>() else {
                    return false;
                };
                let mut handled = false;
                if let Some(w) = weak.upgrade() {
                    for file in list.files() {
                        if let Some(path) = file.path() {
                            w.open_path(path, !handled);
                            handled = true;
                        }
                    }
                }
                handled
            });
            window.add_controller(drop);
        }

        window_rc.install_actions(app);
        window_rc.rebuild_sidebar();
        window_rc.start_auto_refresh();
        if settings.window_maximized {
            window.maximize();
        }
        window.present();

        // Restore previously open repositories.
        let weak = Rc::downgrade(&window_rc);
        glib::idle_add_local_once(move || {
            if let Some(w) = weak.upgrade() {
                let paths: Vec<PathBuf> = w
                    .settings
                    .borrow()
                    .open_repos
                    .iter()
                    .map(PathBuf::from)
                    .filter(|p| p.exists())
                    .collect();
                for path in paths {
                    w.open_path(path, false);
                }
                if w.sessions.borrow().is_empty() {
                    w.content_stack.set_visible_child_name("welcome");
                }
            }
        });

        window_rc
    }

    fn install_actions(self: &Rc<Self>, _app: &adw::Application) {
        let actions: Vec<(&str, Vec<&str>, Rc<dyn Fn(&Rc<Self>)>)> = vec![
            (
                "open-repo",
                vec!["<Control>o"],
                Rc::new(|w: &Rc<Self>| w.open_repository_dialog()),
            ),
            (
                "clone",
                vec!["<Control><Shift>o"],
                Rc::new(|w: &Rc<Self>| w.clone_dialog()),
            ),
            (
                "init",
                vec![],
                Rc::new(|w: &Rc<Self>| w.init_dialog()),
            ),
            (
                "refresh",
                vec!["<Control>r"],
                Rc::new(|w: &Rc<Self>| {
                    if let Some(page) = w.active_page() {
                        page.refresh_all();
                    }
                }),
            ),
            (
                "stage-selected",
                vec!["<Control>s"],
                Rc::new(|w: &Rc<Self>| {
                    if let Some(page) = w.active_page() {
                        page.stage_selected();
                    }
                }),
            ),
            (
                "stage-all",
                vec!["<Control><Shift>s"],
                Rc::new(|w: &Rc<Self>| {
                    if let Some(page) = w.active_page() {
                        page.stage_all();
                    }
                }),
            ),
            (
                "commit",
                vec!["<Control>Return"],
                Rc::new(|w: &Rc<Self>| {
                    if let Some(page) = w.active_page() {
                        page.commit_now();
                    }
                }),
            ),
            (
                "push",
                vec!["<Control>p"],
                Rc::new(|w: &Rc<Self>| {
                    if let Some(page) = w.active_page() {
                        page.quick_push();
                    }
                }),
            ),
            (
                "pull",
                vec!["<Control><Shift>p"],
                Rc::new(|w: &Rc<Self>| {
                    if let Some(page) = w.active_page() {
                        page.quick_pull();
                    }
                }),
            ),
            (
                "fetch",
                vec!["<Control>f"],
                Rc::new(|w: &Rc<Self>| {
                    if let Some(page) = w.active_page() {
                        page.quick_fetch();
                    }
                }),
            ),
            (
                "search",
                vec!["<Control><Shift>f"],
                Rc::new(|w: &Rc<Self>| {
                    if let Some(page) = w.active_page() {
                        page.focus_search();
                    }
                }),
            ),
            (
                "settings",
                vec!["<Control>comma"],
                Rc::new(|w: &Rc<Self>| w.open_settings()),
            ),
            (
                "shortcuts",
                vec!["<Control>question", "F1"],
                Rc::new(|w: &Rc<Self>| w.show_shortcuts()),
            ),
            (
                "diagnostics",
                vec![],
                Rc::new(|w: &Rc<Self>| w.show_diagnostics()),
            ),
            (
                "about",
                vec![],
                Rc::new(|w: &Rc<Self>| w.show_about()),
            ),
            (
                "section-changes",
                vec!["<Control>1"],
                Rc::new(|w: &Rc<Self>| {
                    if let Some(page) = w.active_page() {
                        page.show_section("changes");
                    }
                }),
            ),
            (
                "section-history",
                vec!["<Control>2"],
                Rc::new(|w: &Rc<Self>| {
                    if let Some(page) = w.active_page() {
                        page.show_section("history");
                    }
                }),
            ),
            (
                "section-branches",
                vec!["<Control>3"],
                Rc::new(|w: &Rc<Self>| {
                    if let Some(page) = w.active_page() {
                        page.show_section("branches");
                    }
                }),
            ),
            (
                "section-tags",
                vec!["<Control>4"],
                Rc::new(|w: &Rc<Self>| {
                    if let Some(page) = w.active_page() {
                        page.show_section("tags");
                    }
                }),
            ),
            (
                "section-stashes",
                vec!["<Control>5"],
                Rc::new(|w: &Rc<Self>| {
                    if let Some(page) = w.active_page() {
                        page.show_section("stashes");
                    }
                }),
            ),
            (
                "section-remotes",
                vec!["<Control>6"],
                Rc::new(|w: &Rc<Self>| {
                    if let Some(page) = w.active_page() {
                        page.show_section("remotes");
                    }
                }),
            ),
            (
                "toggle-sidebar",
                vec!["F9"],
                Rc::new(|w: &Rc<Self>| {
                    w.split.set_show_sidebar(!w.split.shows_sidebar());
                }),
            ),
            (
                "close-repo",
                vec!["<Control>w"],
                Rc::new(|w: &Rc<Self>| {
                    if let Some(path) = w.active_path() {
                        let weak = Rc::downgrade(w);
                        let path_clone = path.clone();
                        let window = w.window.clone();
                        super::confirm::confirm(
                            &window,
                            "Close repository?",
                            &format!("{} will be removed from the window.", path.display()),
                            "Close",
                            false,
                            move || {
                                if let Some(w) = weak.upgrade() {
                                    w.close_repo(&path_clone);
                                }
                            },
                        );
                    }
                }),
            ),
        ];

        for (name, _defaults, callback) in actions {
            let action = gio::SimpleAction::new(name, None);
            let callback = Rc::clone(&callback);
            action.connect_activate({
                let weak = Rc::downgrade(self);
                move |_, _| {
                    if let Some(w) = weak.upgrade() {
                        callback(&w);
                    }
                }
            });
            self.window.add_action(&action);
        }
        // `shortcuts::DEFS` is authoritative for accelerators; user overrides
        // are applied immediately after installation.
        self.apply_accels_for(&self.settings.borrow().clone());

        // Save state when the window closes.
        {
            let weak = Rc::downgrade(self);
            self.window.connect_close_request(move |_| {
                if let Some(w) = weak.upgrade() {
                    w.save_state();
                }
                glib::Propagation::Proceed
            });
        }
    }

    fn rebuild_sidebar(self: &Rc<Self>) {
        self.rebuilding.set(true);
        while let Some(child) = self.open_list.first_child() {
            self.open_list.remove(&child);
        }
        let active = self.active_path();
        let mut active_row = None;
        let sessions: Vec<(PathBuf, String)> = self
            .sessions
            .borrow()
            .iter()
            .map(|s| {
                (
                    s.path.clone(),
                    s.path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default(),
                )
            })
            .collect();
        for (path, name) in sessions {
            let row = gtk::ListBoxRow::new();
            let box_ = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            box_.set_margin_top(6);
            box_.set_margin_bottom(6);
            box_.set_margin_start(8);
            box_.set_margin_end(8);
            let icon = gtk::Image::from_icon_name("git-symbolic");
            icon.set_pixel_size(16);
            let text = gtk::Box::new(gtk::Orientation::Vertical, 1);
            text.set_hexpand(true);
            let name_label = gtk::Label::builder()
                .label(&name)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build();
            let path_label = gtk::Label::builder()
                .label(
                    path.parent()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_default(),
                )
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::Middle)
                .build();
            path_label.add_css_class("dim-label");
            text.append(&name_label);
            text.append(&path_label);
            let close = gtk::Button::builder()
                .icon_name("window-close-symbolic")
                .tooltip_text("Close repository")
                .build();
            close.add_css_class("flat");
            close.add_css_class("circular");
            {
                let weak = Rc::downgrade(self);
                let path = path.clone();
                close.connect_clicked(move |_| {
                    if let Some(w) = weak.upgrade() {
                        w.close_repo(&path);
                    }
                });
            }
            box_.append(&icon);
            box_.append(&text);
            box_.append(&close);
            row.set_child(Some(&box_));
            row.set_tooltip_text(Some(&path.display().to_string()));
            row.set_widget_name(&format!("path:{}", path.display()));
            self.open_list.append(&row);
            if active.as_ref() == Some(&path) {
                active_row = Some(row);
            }
        }
        if let Some(row) = active_row {
            self.open_list.select_row(Some(&row));
        }
        self.rebuilding.set(false);

        // Recent repositories
        while let Some(child) = self.recent_list.first_child() {
            self.recent_list.remove(&child);
        }
        let settings = self.settings.borrow();
        let open: Vec<String> = self
            .sessions
            .borrow()
            .iter()
            .map(|s| s.path.to_string_lossy().to_string())
            .collect();
        let mut shown = 0;
        for recent in settings.recent_repos.iter() {
            if open.contains(&recent.path) {
                continue;
            }
            if shown >= 8 {
                break;
            }
            shown += 1;
            let row = gtk::ListBoxRow::new();
            let box_ = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            box_.set_margin_top(4);
            box_.set_margin_bottom(4);
            box_.set_margin_start(8);
            box_.set_margin_end(8);
            let icon = gtk::Image::from_icon_name("document-open-recent-symbolic");
            icon.set_pixel_size(14);
            icon.add_css_class("dim-label");
            let label = gtk::Label::builder()
                .label(&recent.name)
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build();
            let remove = gtk::Button::builder()
                .icon_name("window-close-symbolic")
                .tooltip_text("Remove from recent list")
                .build();
            remove.add_css_class("flat");
            remove.add_css_class("circular");
            {
                let weak = Rc::downgrade(self);
                let path = recent.path.clone();
                remove.connect_clicked(move |_| {
                    if let Some(w) = weak.upgrade() {
                        w.settings.borrow_mut().remove_recent(&path);
                        let _ = w.settings.borrow().save();
                        let w2 = Rc::clone(&w);
                        w2.rebuild_sidebar();
                    }
                });
            }
            box_.append(&icon);
            box_.append(&label);
            box_.append(&remove);
            row.set_child(Some(&box_));
            row.set_widget_name(&format!("path:{}", recent.path));
            self.recent_list.append(&row);
        }
    }

    fn activate_row(self: &Rc<Self>, row: &gtk::ListBoxRow) {
        if let Some(path) = row.widget_name().strip_prefix("path:").map(PathBuf::from) {
            self.show_repo(&path);
        }
    }

    pub fn active_path(&self) -> Option<PathBuf> {
        let sessions = self.sessions.borrow();
        let active_name = self.content_stack.visible_child_name()?;
        let active = active_name.strip_prefix("repo:")?;
        sessions
            .iter()
            .find(|s| s.path.display().to_string() == active)
            .map(|s| s.path.clone())
    }

    pub fn active_page(&self) -> Option<Rc<RepoPage>> {
        let path = self.active_path()?;
        let sessions = self.sessions.borrow();
        sessions
            .iter()
            .find(|s| s.path == path)
            .map(|s| Rc::clone(&s.page))
    }

    fn show_repo(self: &Rc<Self>, path: &Path) {
        let name = format!("repo:{}", path.display());
        if self.content_stack.child_by_name(&name).is_none() {
            if let Some(session) = self.sessions.borrow().iter().find(|s| s.path == path) {
                self.content_stack
                    .add_named(session.page.widget(), Some(&name));
            }
        }
        self.content_stack.set_visible_child_name(&name);
        if let Some(page) = self.active_page() {
            self.window
                .set_title(Some(&format!("{} — Stratum", page.title())));
            page.refresh_all();
        }
        {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let mut settings = self.settings.borrow_mut();
            settings.add_open_repo(path);
            settings.touch_recent(path, &name);
            let _ = settings.save();
        }
        self.rebuild_sidebar();
    }

    pub fn open_path(self: &Rc<Self>, path: PathBuf, activate: bool) {
        let existing = self
            .sessions
            .borrow()
            .iter()
            .find(|s| s.path == path)
            .map(|s| s.path.clone());
        if let Some(existing) = existing {
            if activate {
                self.show_repo(&existing);
            }
            return;
        }
        if !self.opening.borrow_mut().insert(path.clone()) {
            return;
        }
        let weak = Rc::downgrade(self);
        let path_clone = path.clone();
        super::tasks::run_blocking(
            move || Repository::open(&path_clone),
            move |result| {
                let Some(window) = weak.upgrade() else { return };
                window.opening.borrow_mut().remove(&path);
                match result {
                    Ok(repo) => {
                        let settings = window.settings.borrow().clone();
                        let page = RepoPage::new(repo, settings);
                        let toggle =
                            util::icon_button("sidebar-show-symbolic", "Toggle sidebar (F9)");
                        let split = window.split.clone();
                        toggle.connect_clicked(move |_| {
                            split.set_show_sidebar(!split.shows_sidebar());
                        });
                        page.add_header_start(&toggle);
                        window.sessions.borrow_mut().push(RepoSession {
                            path: path.clone(),
                            page,
                        });
                        if activate {
                            window.show_repo(&path);
                        } else {
                            window.rebuild_sidebar();
                        }
                    }
                    Err(e) => {
                        crate::log_warn!(
                            "could not open {}: {}",
                            path.display(),
                            e.technical_details()
                        );
                        super::error_dialog::show(&window.window, &e);
                    }
                }
            },
        );
    }

    pub fn close_repo(self: &Rc<Self>, path: &Path) {
        let index = self.sessions.borrow().iter().position(|s| s.path == path);
        let Some(index) = index else { return };
        let session = self.sessions.borrow_mut().remove(index);
        let name = format!("repo:{}", path.display());
        if let Some(child) = self.content_stack.child_by_name(&name) {
            self.content_stack.remove(&child);
        }
        drop(session);
        {
            let mut settings = self.settings.borrow_mut();
            settings.remove_open_repo(&path.to_string_lossy());
            let _ = settings.save();
        }
        if self.sessions.borrow().is_empty() {
            self.content_stack.set_visible_child_name("welcome");
            self.window.set_title(Some("Stratum"));
        } else {
            let first = self.sessions.borrow()[0].path.clone();
            self.show_repo(&first);
        }
        self.rebuild_sidebar();
    }

    pub fn open_repository_dialog(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::builder()
            .title("Open Git repository")
            .build();
        let weak = Rc::downgrade(self);
        dialog.select_folder(Some(&self.window), gio::Cancellable::NONE, move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    if let Some(window) = weak.upgrade() {
                        window.open_path(path, true);
                    }
                }
            }
        });
    }

    pub fn clone_dialog(self: &Rc<Self>) {
        let settings = self.settings.borrow().clone();
        let weak = Rc::downgrade(self);
        clone_dialog::show(
            &self.window,
            settings,
            Rc::new(move |path| {
                if let Some(window) = weak.upgrade() {
                    window.open_path(path.clone(), true);
                }
            }),
        );
    }

    pub fn init_dialog(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        clone_dialog::show_init(
            &self.window,
            Rc::new(move |path| {
                if let Some(window) = weak.upgrade() {
                    window.open_path(path.clone(), true);
                }
            }),
        );
    }

    pub fn open_settings(self: &Rc<Self>) {
        let settings = self.settings.borrow().clone();
        let weak = Rc::downgrade(self);
        let repo = self.active_page().map(|p| p.repository().clone());
        settings_dialog::show(
            &self.window,
            settings,
            repo,
            Rc::new(move |settings| {
                if let Some(window) = weak.upgrade() {
                    if let Some(page) = window.active_page() {
                        page.apply_settings(settings.clone());
                    }
                    window.apply_accels_for(&settings);
                    *window.settings.borrow_mut() = settings;
                }
            }),
        );
    }

    pub fn show_shortcuts(&self) {
        let settings = self.settings.borrow().clone();
        shortcuts::show(&self.window, &settings);
    }

    pub fn show_diagnostics(&self) {
        diagnostics_dialog::show(&self.window);
    }

    pub fn show_about(&self) {
        let about = adw::AboutWindow::builder()
            .application_name("Stratum")
            .application_icon("io.github.stratum_git.Stratum")
            .version(env!("CARGO_PKG_VERSION"))
            .developer_name("Stratum contributors")
            .license_type(gtk::License::Gpl30)
            .comments("A native Git GUI client for Linux.")
            .build();
        about.present();
    }

    pub fn window(&self) -> &adw::ApplicationWindow {
        &self.window
    }

    /// (Re-)apply keyboard accelerators from the current settings.
    pub fn apply_accels_for(&self, settings: &crate::core::Settings) {
        let app = self.app.clone();
        crate::ui::shortcuts::apply_accels(&app, settings);
    }

    /// Auto-refresh: every few seconds, refresh the active repository's
    /// status while the window is active and no operation is running.
    fn start_auto_refresh(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let mut tick: u32 = 0;
        glib::timeout_add_seconds_local(5, move || {
            let Some(w) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            tick = tick.wrapping_add(1);
            if !w.window.is_active() {
                return glib::ControlFlow::Continue;
            }
            if let Some(page) = w.active_page() {
                if !page.is_busy() {
                    if tick % 6 == 0 {
                        page.refresh_all();
                    } else {
                        page.refresh_status_views();
                    }
                }
            }
            glib::ControlFlow::Continue
        });
    }

    pub fn save_state(&self) {
        let mut settings = self.settings.borrow_mut();
        settings.window_width = self.window.default_width().max(600);
        settings.window_height = self.window.default_height().max(400);
        settings.window_maximized = self.window.is_maximized();
        let _ = settings.save();
    }
}
