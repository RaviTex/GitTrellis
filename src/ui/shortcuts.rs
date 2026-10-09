//! Keyboard shortcuts: definitions, application of user overrides, the
//! reference dialog and the editor used from Settings.

use std::rc::Rc;

use adw::prelude::*;

use crate::core::Settings;

/// One configurable command.
#[derive(Debug, Clone, Copy)]
pub struct ShortcutDef {
    /// Action name registered on the window (`win.<action>`).
    pub action: &'static str,
    pub label: &'static str,
    pub section: &'static str,
    /// Default accelerators; an empty slice means unbound by default.
    pub defaults: &'static [&'static str],
}

pub const DEFS: &[ShortcutDef] = &[
    ShortcutDef {
        action: "open-repo",
        label: "Open repository",
        section: "Repository",
        defaults: &["<Control>o"],
    },
    ShortcutDef {
        action: "clone",
        label: "Clone repository",
        section: "Repository",
        defaults: &["<Control><Shift>o"],
    },
    ShortcutDef {
        action: "init",
        label: "Initialize repository",
        section: "Repository",
        defaults: &[],
    },
    ShortcutDef {
        action: "close-repo",
        label: "Close repository",
        section: "Repository",
        defaults: &["<Control>w"],
    },
    ShortcutDef {
        action: "refresh",
        label: "Refresh",
        section: "Repository",
        defaults: &["<Control>r"],
    },
    ShortcutDef {
        action: "search",
        label: "Search",
        section: "Repository",
        defaults: &["<Control><Shift>f"],
    },
    ShortcutDef {
        action: "settings",
        label: "Settings",
        section: "Repository",
        defaults: &["<Control>comma"],
    },
    ShortcutDef {
        action: "shortcuts",
        label: "Keyboard shortcuts",
        section: "Repository",
        defaults: &["<Control>question", "F1"],
    },
    ShortcutDef {
        action: "diagnostics",
        label: "Diagnostics",
        section: "Repository",
        defaults: &[],
    },
    ShortcutDef {
        action: "stage-selected",
        label: "Stage selected files",
        section: "Changes",
        defaults: &["<Control>s"],
    },
    ShortcutDef {
        action: "stage-all",
        label: "Stage all changes",
        section: "Changes",
        defaults: &["<Control><Shift>s"],
    },
    ShortcutDef {
        action: "commit",
        label: "Commit",
        section: "Changes",
        defaults: &["<Control>Return"],
    },
    ShortcutDef {
        action: "push",
        label: "Push",
        section: "Remotes",
        defaults: &["<Control>p"],
    },
    ShortcutDef {
        action: "pull",
        label: "Pull",
        section: "Remotes",
        defaults: &["<Control><Shift>p"],
    },
    ShortcutDef {
        action: "fetch",
        label: "Fetch",
        section: "Remotes",
        defaults: &["<Control>f"],
    },
    ShortcutDef {
        action: "section-changes",
        label: "Go to Changes",
        section: "Navigation",
        defaults: &["<Control>1"],
    },
    ShortcutDef {
        action: "section-history",
        label: "Go to History",
        section: "Navigation",
        defaults: &["<Control>2"],
    },
    ShortcutDef {
        action: "section-branches",
        label: "Go to Branches",
        section: "Navigation",
        defaults: &["<Control>3"],
    },
    ShortcutDef {
        action: "section-tags",
        label: "Go to Tags",
        section: "Navigation",
        defaults: &["<Control>4"],
    },
    ShortcutDef {
        action: "section-stashes",
        label: "Go to Stashes",
        section: "Navigation",
        defaults: &["<Control>5"],
    },
    ShortcutDef {
        action: "section-remotes",
        label: "Go to Remotes",
        section: "Navigation",
        defaults: &["<Control>6"],
    },
    ShortcutDef {
        action: "toggle-sidebar",
        label: "Toggle sidebar",
        section: "Navigation",
        defaults: &["F9"],
    },
];

/// The effective accelerators for one action, honouring user overrides.
/// An override of the empty string disables the shortcut.
pub fn effective_accels(settings: &Settings, def: &ShortcutDef) -> Vec<String> {
    match settings.shortcut_overrides.get(def.action) {
        Some(custom) if custom.trim().is_empty() => Vec::new(),
        Some(custom) => vec![custom.clone()],
        None => def.defaults.iter().map(|s| s.to_string()).collect(),
    }
}

pub fn is_customized(settings: &Settings, def: &ShortcutDef) -> bool {
    settings.shortcut_overrides.contains_key(def.action)
}

/// Apply the effective accelerators to the application's window actions.
pub fn apply_accels(app: &adw::Application, settings: &Settings) {
    for def in DEFS {
        let accels = effective_accels(settings, def);
        let refs: Vec<&str> = accels.iter().map(|s| s.as_str()).collect();
        app.set_accels_for_action(&format!("win.{}", def.action), &refs);
    }
}

/// Human readable form of an accelerator string.
pub fn display_accel(accel: &str) -> String {
    accel
        .replace("<Control>", "Ctrl+")
        .replace("<Primary>", "Ctrl+")
        .replace("<Shift>", "Shift+")
        .replace("<Alt>", "Alt+")
        .replace("<Super>", "Super+")
        .replace("<Meta>", "Meta+")
        .replace("Return", "Enter")
        .replace("question", "?")
        .replace("comma", ",")
        .replace("BackSpace", "Backspace")
}

fn effective_display(settings: &Settings, def: &ShortcutDef) -> String {
    let accels = effective_accels(settings, def);
    if accels.is_empty() {
        "Not assigned".to_string()
    } else {
        accels
            .iter()
            .map(|a| display_accel(a))
            .collect::<Vec<_>>()
            .join(" · ")
    }
}

/// What the user chose in the capture dialog.
#[derive(Debug, Clone)]
pub enum ShortcutChoice {
    Custom(String),
    /// Remove the override and go back to the default.
    Reset,
    /// Explicitly disable the shortcut.
    Disabled,
}

/// Apply a captured choice to the settings.
pub fn apply_choice(settings: &mut Settings, action: &str, choice: &ShortcutChoice) {
    match choice {
        ShortcutChoice::Custom(accel) => {
            settings
                .shortcut_overrides
                .insert(action.to_string(), accel.clone());
        }
        ShortcutChoice::Reset => {
            settings.shortcut_overrides.remove(action);
        }
        ShortcutChoice::Disabled => {
            settings
                .shortcut_overrides
                .insert(action.to_string(), String::new());
        }
    }
}

/// Dialog that records a single key combination.
pub fn capture_dialog(
    parent: &impl IsA<gtk::Window>,
    def: &ShortcutDef,
    on_result: impl Fn(ShortcutChoice) + 'static,
) {
    let window = adw::Window::builder()
        .title("Set shortcut")
        .modal(true)
        .transient_for(parent)
        .default_width(380)
        .build();
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    toolbar.add_top_bar(&header);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_start(24);
    content.set_margin_end(24);
    content.set_margin_top(24);
    content.set_margin_bottom(24);

    let title = gtk::Label::builder()
        .label(format!("Shortcut for “{}”", def.label))
        .xalign(0.0)
        .wrap(true)
        .build();
    title.add_css_class("heading");
    let capture = gtk::Label::builder()
        .label("Press a key combination…")
        .margin_top(12)
        .margin_bottom(12)
        .build();
    capture.add_css_class("title-2");
    let hint = gtk::Label::builder()
        .label("Esc cancels · Backspace removes the shortcut")
        .xalign(0.5)
        .build();
    hint.add_css_class("dim-label");

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::Center);
    let reset = gtk::Button::with_label("Use Default");
    let cancel = gtk::Button::with_label("Cancel");
    buttons.append(&reset);
    buttons.append(&cancel);

    content.append(&title);
    content.append(&capture);
    content.append(&hint);
    content.append(&buttons);
    toolbar.set_content(Some(&content));
    window.set_content(Some(&toolbar));

    let on_result = Rc::new(on_result);
    {
        let on_result = Rc::clone(&on_result);
        let window = window.clone();
        reset.connect_clicked(move |_| {
            on_result(ShortcutChoice::Reset);
            window.close();
        });
    }
    {
        let window = window.clone();
        cancel.connect_clicked(move |_| {
            window.close();
        });
    }
    {
        let on_result = Rc::clone(&on_result);
        let window_for_keys = window.clone();
        let capture_label = capture.clone();
        let controller = gtk::EventControllerKey::new();
        controller.set_propagation_phase(gtk::PropagationPhase::Capture);
        controller.connect_key_pressed(move |_, key, _, state| {
            use gtk::gdk::Key;
            if key == Key::Escape {
                window_for_keys.close();
                return gtk::glib::Propagation::Stop;
            }
            if key == Key::BackSpace || key == Key::Delete {
                on_result(ShortcutChoice::Disabled);
                window_for_keys.close();
                return gtk::glib::Propagation::Stop;
            }
            // Ignore modifier-only presses.
            if matches!(
                key,
                Key::Shift_L
                    | Key::Shift_R
                    | Key::Control_L
                    | Key::Control_R
                    | Key::Alt_L
                    | Key::Alt_R
                    | Key::Super_L
                    | Key::Super_R
                    | Key::Meta_L
                    | Key::Meta_R
                    | Key::Caps_Lock
                    | Key::Num_Lock
            ) {
                return gtk::glib::Propagation::Stop;
            }
            let accel = gtk::accelerator_name(key, state).to_string();
            capture_label.set_label(&display_accel(&accel));
            on_result(ShortcutChoice::Custom(accel));
            window_for_keys.close();
            gtk::glib::Propagation::Stop
        });
        window.add_controller(controller);
    }

    window.present();
}

/// Preferences page that lists and edits every shortcut.
pub fn build_settings_page(
    settings: Rc<std::cell::RefCell<Settings>>,
    on_changed: Rc<dyn Fn(Settings)>,
) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("Shortcuts")
        .icon_name("preferences-desktop-keyboard-shortcuts-symbolic")
        .build();
    page.set_name(Some("shortcuts"));

    let mut groups: Vec<(String, adw::PreferencesGroup)> = Vec::new();
    for def in DEFS {
        let section = def.section.to_string();
        if !groups.iter().any(|(name, _)| name == &section) {
            let group = adw::PreferencesGroup::builder().title(&section).build();
            groups.push((section.clone(), group));
        }
    }

    let mut rows: Vec<(String, adw::ActionRow)> = Vec::new();
    for def in DEFS {
        let row = adw::ActionRow::builder()
            .title(def.label)
            .subtitle(effective_display(&settings.borrow(), def))
            .build();
        let button = gtk::Button::with_label("Change…");
        button.add_css_class("flat");
        button.set_valign(gtk::Align::Center);
        row.add_suffix(&button);
        row.set_activatable_widget(Some(&button));
        {
            let settings = Rc::clone(&settings);
            let on_changed = Rc::clone(&on_changed);
            let row_clone = row.clone();
            button.connect_clicked(move |button| {
                let Some(parent) = button.root().and_downcast::<gtk::Window>() else {
                    return;
                };
                let settings = Rc::clone(&settings);
                let on_changed = Rc::clone(&on_changed);
                let row = row_clone.clone();
                capture_dialog(&parent, def, move |choice| {
                    let mut current = settings.borrow().clone();
                    apply_choice(&mut current, def.action, &choice);
                    row.set_subtitle(&effective_display(&current, def));
                    on_changed(current);
                });
            });
        }
        for (name, group) in groups.iter_mut() {
            if name == def.section {
                group.add(&row);
            }
        }
        rows.push((def.action.to_string(), row));
    }

    // "Reset all" group
    let reset_group = adw::PreferencesGroup::new();
    let reset_row = adw::ActionRow::builder()
        .title("Reset all shortcuts")
        .subtitle("Restore the default key bindings")
        .build();
    let reset_button = gtk::Button::builder()
        .label("Reset")
        .valign(gtk::Align::Center)
        .build();
    reset_row.add_suffix(&reset_button);
    reset_group.add(&reset_row);
    {
        let settings = Rc::clone(&settings);
        let on_changed = Rc::clone(&on_changed);
        let rows = rows.clone();
        reset_button.connect_clicked(move |_| {
            let mut current = settings.borrow().clone();
            current.shortcut_overrides.clear();
            for (action, row) in &rows {
                if let Some(def) = DEFS.iter().find(|d| d.action == action) {
                    row.set_subtitle(&effective_display(&current, def));
                }
            }
            on_changed(current);
        });
    }

    for (_, group) in &groups {
        page.add(group);
    }
    page.add(&reset_group);
    page
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_overrides() {
        let mut settings = Settings::default();
        let def = DEFS.iter().find(|d| d.action == "stage-all").unwrap();
        assert_eq!(effective_accels(&settings, def), vec!["<Control><Shift>s"]);

        settings
            .shortcut_overrides
            .insert("stage-all".into(), "<Control>g".into());
        assert_eq!(effective_accels(&settings, def), vec!["<Control>g"]);

        settings
            .shortcut_overrides
            .insert("stage-all".into(), String::new());
        assert!(effective_accels(&settings, def).is_empty());

        settings.shortcut_overrides.remove("stage-all");
        assert_eq!(effective_accels(&settings, def), vec!["<Control><Shift>s"]);
    }

    #[test]
    fn choice_application() {
        let mut settings = Settings::default();
        apply_choice(&mut settings, "fetch", &ShortcutChoice::Custom("<Control>g".into()));
        assert_eq!(settings.shortcut_overrides.get("fetch").unwrap(), "<Control>g");
        apply_choice(&mut settings, "fetch", &ShortcutChoice::Disabled);
        assert_eq!(settings.shortcut_overrides.get("fetch").unwrap(), "");
        apply_choice(&mut settings, "fetch", &ShortcutChoice::Reset);
        assert!(!settings.shortcut_overrides.contains_key("fetch"));
    }

    #[test]
    fn accel_display() {
        assert_eq!(display_accel("<Control><Shift>f"), "Ctrl+Shift+f");
        assert_eq!(display_accel("<Control>Return"), "Ctrl+Enter");
    }

    #[test]
    fn all_actions_unique() {
        let mut seen = std::collections::HashSet::new();
        for def in DEFS {
            assert!(seen.insert(def.action), "duplicate action {}", def.action);
        }
    }
}
