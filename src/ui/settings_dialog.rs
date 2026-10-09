//! Settings: application preferences and Git configuration.

use std::rc::Rc;

use adw::prelude::*;

use crate::core::{DiffViewMode, Settings};
use crate::git::{ConfigScope, Repository};

/// Show the preferences window. `repo` enables the repository-scope Git
/// configuration controls; `on_saved` is invoked after application settings
/// change so the UI can apply them.
pub fn show(
    parent: &impl IsA<gtk::Window>,
    settings: Settings,
    repo: Option<Repository>,
    focus_page: Option<&str>,
    on_saved: Rc<dyn Fn(Settings)>,
) -> adw::PreferencesWindow {
    let window = adw::PreferencesWindow::builder()
        .title("Settings")
        .transient_for(parent)
        .modal(false)
        .search_enabled(true)
        .build();

    let saved = Rc::new(std::cell::RefCell::new(settings.clone()));
    let apply = {
        let saved = Rc::clone(&saved);
        let on_saved = Rc::clone(&on_saved);
        move |settings: Settings| {
            *saved.borrow_mut() = settings.clone();
            let _ = settings.save();
            on_saved(settings);
        }
    };

    // ---------------- Application page ----------------
    let app_page = adw::PreferencesPage::builder()
        .title("Application")
        .icon_name("preferences-system-symbolic")
        .build();
    app_page.set_name(Some("application"));

    let appearance = adw::PreferencesGroup::builder().title("Appearance").build();
    let diff_modes = gtk::StringList::new(&["Unified", "Side by side"]);
    let diff_row = adw::ComboRow::builder()
        .title("Diff layout")
        .model(&diff_modes)
        .selected(match settings.diff_view_mode {
            DiffViewMode::Unified => 0,
            DiffViewMode::SideBySide => 1,
        })
        .build();
    {
        let apply = apply.clone();
        let saved = Rc::clone(&saved);
        diff_row.connect_selected_notify(move |row| {
            let mut settings = saved.borrow().clone();
            settings.diff_view_mode = if row.selected() == 1 {
                DiffViewMode::SideBySide
            } else {
                DiffViewMode::Unified
            };
            apply(settings);
        });
    }
    appearance.add(&diff_row);

    let context_row = adw::SpinRow::builder()
        .title("Diff context lines")
        .subtitle("Lines of context shown around each change in patches")
        .adjustment(&gtk::Adjustment::new(
            settings.diff_context_lines as f64,
            0.0,
            20.0,
            1.0,
            4.0,
            0.0,
        ))
        .build();
    {
        let apply = apply.clone();
        let saved = Rc::clone(&saved);
        context_row.connect_value_notify(move |row| {
            let saved = Rc::clone(&saved);
            let mut settings = saved.borrow().clone();
            settings.diff_context_lines = row.value() as u32;
            apply(settings);
        });
    }
    appearance.add(&context_row);

    let ignored_row = adw::SwitchRow::builder()
        .title("Show ignored files")
        .subtitle("List files matched by .gitignore in the Changes view")
        .active(settings.show_ignored_files)
        .build();
    {
        let apply = apply.clone();
        let saved = Rc::clone(&saved);
        ignored_row.connect_active_notify(move |row| {
            let saved = Rc::clone(&saved);
            let mut settings = saved.borrow().clone();
            settings.show_ignored_files = row.is_active();
            apply(settings);
        });
    }
    appearance.add(&ignored_row);

    let behaviour = adw::PreferencesGroup::builder().title("Fetching").build();
    let prune_row = adw::SwitchRow::builder()
        .title("Prune on fetch")
        .subtitle("Remove remote-tracking branches whose remote branch was deleted")
        .active(settings.fetch_prune)
        .build();
    {
        let apply = apply.clone();
        let saved = Rc::clone(&saved);
        prune_row.connect_active_notify(move |row| {
            let saved = Rc::clone(&saved);
            let mut settings = saved.borrow().clone();
            settings.fetch_prune = row.is_active();
            apply(settings);
        });
    }
    behaviour.add(&prune_row);

    let commit_group = adw::PreferencesGroup::builder().title("Commits").build();
    let signoff_row = adw::SwitchRow::builder()
        .title("Sign off commits by default")
        .active(settings.commit_sign_off)
        .build();
    {
        let apply = apply.clone();
        let saved = Rc::clone(&saved);
        signoff_row.connect_active_notify(move |row| {
            let saved = Rc::clone(&saved);
            let mut settings = saved.borrow().clone();
            settings.commit_sign_off = row.is_active();
            apply(settings);
        });
    }
    let gpg_row = adw::SwitchRow::builder()
        .title("Sign commits with GPG/SSH by default")
        .active(settings.commit_gpg_sign)
        .build();
    {
        let apply = apply.clone();
        let saved = Rc::clone(&saved);
        gpg_row.connect_active_notify(move |row| {
            let saved = Rc::clone(&saved);
            let mut settings = saved.borrow().clone();
            settings.commit_gpg_sign = row.is_active();
            apply(settings);
        });
    }
    commit_group.add(&signoff_row);
    commit_group.add(&gpg_row);

    let log_group = adw::PreferencesGroup::builder()
        .title("Diagnostics")
        .description("Log files are written to the application state directory.")
        .build();
    let level_model = gtk::StringList::new(&["Error", "Warnings", "Info", "Debug"]);
    let level_row = adw::ComboRow::builder()
        .title("Log level")
        .model(&level_model)
        .selected(match settings.log_level.as_str() {
            "error" => 0,
            "warn" => 1,
            "debug" => 2,
            _ => 3,
        })
        .build();
    {
        let apply = apply.clone();
        let saved = Rc::clone(&saved);
        level_row.connect_selected_notify(move |row| {
            let saved = Rc::clone(&saved);
            let mut settings = saved.borrow().clone();
            settings.log_level = match row.selected() {
                0 => "error",
                1 => "warn",
                2 => "debug",
                _ => "info",
            }
            .to_string();
            crate::core::logging::set_level(level_from(&settings.log_level));
            apply(settings);
        });
    }
    log_group.add(&level_row);

    app_page.add(&appearance);
    app_page.add(&behaviour);
    app_page.add(&commit_group);
    app_page.add(&log_group);
    window.add(&app_page);

    // ---------------- Shortcuts page ----------------
    let apply_rc: Rc<dyn Fn(Settings)> = {
        let apply = apply.clone();
        Rc::new(move |settings: Settings| apply(settings))
    };
    let shortcuts_page =
        super::shortcuts::build_settings_page(Rc::clone(&saved), Rc::clone(&apply_rc));
    window.add(&shortcuts_page);
    let focus_page = focus_page.map(str::to_string);

    // ---------------- Git configuration page ----------------
    if let Some(repo) = repo {
        let git_page = adw::PreferencesPage::builder()
            .title("Git")
            .icon_name("git-symbolic")
            .build();
        git_page.set_name(Some("git"));

        let scope_group = adw::PreferencesGroup::builder()
            .title("Configuration scope")
            .description("Repository settings only affect this repository.")
            .build();
        let scope_model = gtk::StringList::new(&["Global (all repositories)", "This repository"]);
        let scope_row = adw::ComboRow::builder()
            .title("Scope")
            .model(&scope_model)
            .selected(0)
            .build();
        scope_group.add(&scope_row);

        let identity = adw::PreferencesGroup::builder()
            .title("Identity")
            .description("Used as the author of new commits.")
            .build();
        let name_entry = adw::EntryRow::builder().title("User name").build();
        let email_entry = adw::EntryRow::builder().title("Email").build();
        identity.add(&name_entry);
        identity.add(&email_entry);

        let defaults = adw::PreferencesGroup::builder().title("Defaults").build();
        let branch_entry = adw::EntryRow::builder().title("Default branch").build();
        let editor_entry = adw::EntryRow::builder().title("Editor").build();
        let credential_entry = adw::EntryRow::builder().title("Credential helper").build();
        defaults.add(&branch_entry);
        defaults.add(&editor_entry);
        defaults.add(&credential_entry);

        let signing = adw::PreferencesGroup::builder()
            .title("Signing")
            .description("Commit and tag signing.")
            .build();
        let signing_key_entry = adw::EntryRow::builder().title("Signing key").build();
        let gpg_format_model = gtk::StringList::new(&["openpgp", "ssh", "x509"]);
        let gpg_format_row = adw::ComboRow::builder()
            .title("GPG format")
            .model(&gpg_format_model)
            .selected(0)
            .build();
        signing.add(&signing_key_entry);
        signing.add(&gpg_format_row);

        let load_values = {
            let repo = repo.clone();
            let name_entry = name_entry.clone();
            let email_entry = email_entry.clone();
            let branch_entry = branch_entry.clone();
            let editor_entry = editor_entry.clone();
            let credential_entry = credential_entry.clone();
            let signing_key_entry = signing_key_entry.clone();
            let gpg_format_row = gpg_format_row.clone();
            move |scope: ConfigScope| {
                let get = |key: &str| {
                    repo.config_get_scoped(key, scope)
                        .ok()
                        .flatten()
                        .unwrap_or_default()
                };
                name_entry.set_text(&get("user.name"));
                email_entry.set_text(&get("user.email"));
                branch_entry.set_text(&get("init.defaultBranch"));
                editor_entry.set_text(&get("core.editor"));
                credential_entry.set_text(&get("credential.helper"));
                signing_key_entry.set_text(&get("user.signingkey"));
                let format = get("gpg.format");
                gpg_format_row.set_selected(match format.as_str() {
                    "ssh" => 1,
                    "x509" => 2,
                    _ => 0,
                });
            }
        };
        load_values(ConfigScope::Global);

        // Reload values when the scope changes.
        {
            let load_values = load_values.clone();
            let repo = repo.clone();
            let scope_row_clone = scope_row.clone();
            scope_row.connect_selected_notify(move |_| {
                let _ = &repo;
                let _ = &scope_row_clone;
                load_values(ConfigScope::Global);
            });
        }

        let save_row = adw::ActionRow::builder()
            .title("Save Git configuration")
            .subtitle("Writes the values above to the selected scope")
            .build();
        let save_button = gtk::Button::builder()
            .label("Save")
            .valign(gtk::Align::Center)
            .build();
        save_button.add_css_class("suggested-action");
        save_row.add_suffix(&save_button);
        {
            let repo = repo.clone();
            let name_entry = name_entry.clone();
            let email_entry = email_entry.clone();
            let branch_entry = branch_entry.clone();
            let editor_entry = editor_entry.clone();
            let credential_entry = credential_entry.clone();
            let signing_key_entry = signing_key_entry.clone();
            let gpg_format_row = gpg_format_row.clone();
            let scope_row = scope_row.clone();
            let save_row = save_row.clone();
            save_button.connect_clicked(move |_| {
                let scope = if scope_row.selected() == 1 {
                    ConfigScope::Local
                } else {
                    ConfigScope::Global
                };
                let values = [
                    ("user.name", name_entry.text().to_string()),
                    ("user.email", email_entry.text().to_string()),
                    ("init.defaultBranch", branch_entry.text().to_string()),
                    ("core.editor", editor_entry.text().to_string()),
                    ("credential.helper", credential_entry.text().to_string()),
                    ("user.signingkey", signing_key_entry.text().to_string()),
                    (
                        "gpg.format",
                        match gpg_format_row.selected() {
                            1 => "ssh".to_string(),
                            2 => "x509".to_string(),
                            _ => "openpgp".to_string(),
                        },
                    ),
                ];
                let mut errors = Vec::new();
                for (key, value) in values {
                    let result = if value.trim().is_empty() {
                        repo.config_unset(key, scope)
                    } else {
                        repo.config_set(key, value.trim(), scope)
                    };
                    if let Err(e) = result {
                        errors.push(format!("{key}: {}", e.user_message()));
                    }
                }
                if errors.is_empty() {
                    save_row.set_subtitle("Saved");
                    crate::log_info!("git configuration saved ({scope:?})");
                } else {
                    save_row.set_subtitle("Some values could not be saved");
                    crate::log_error!("failed to save config: {}", errors.join("; "));
                }
            });
        }

        git_page.add(&scope_group);
        git_page.add(&identity);
        git_page.add(&defaults);
        git_page.add(&signing);
        git_page.add(&adw::PreferencesGroup::new());
        let save_group = adw::PreferencesGroup::new();
        save_group.add(&save_row);
        git_page.add(&save_group);
        window.add(&git_page);
    }

    super::util::close_on_escape(&window);
    crate::ui::widgets_keep_alive(window.clone());
    window.present();
    // Selecting the page must happen after the window is presented.
    if let Some(page) = focus_page {
        window.set_visible_page_name(&page);
    }
    window
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_window_focuses_requested_page() {
        // Needs a display; skip silently on headless machines.
        if gtk::init().is_err() {
            eprintln!("skipping settings window test: no display");
            return;
        }
        let parent = gtk::Window::new();
        let settings = Settings::default();
        let window = show(
            &parent,
            settings,
            None,
            Some("shortcuts"),
            Rc::new(|_| {}),
        );
        let context = gtk::glib::MainContext::default();
        for _ in 0..20 {
            while context.pending() {
                context.iteration(false);
            }
        }
        assert_eq!(
            window.visible_page_name().as_deref(),
            Some("shortcuts"),
            "settings should open on the Shortcuts page"
        );
        window.close();
    }
}

fn level_from(name: &str) -> crate::core::logging::Level {
    match name {
        "error" => crate::core::logging::Level::Error,
        "warn" => crate::core::logging::Level::Warn,
        "debug" => crate::core::logging::Level::Debug,
        _ => crate::core::logging::Level::Info,
    }
}
