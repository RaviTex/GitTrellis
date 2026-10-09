//! Shared UI helpers: formatting, colors, clipboard, file launching.

use std::path::Path;

use adw::prelude::*;
use gtk::{gdk, glib};

use crate::git::{ChangeKind, ConflictKind, OperationState};

/// A small palette used for graph lanes and status chips.
pub const LANE_COLORS: [&str; 10] = [
    "#3584e4", "#33d17a", "#e5a50a", "#e66100", "#c061cb", "#e01b24", "#986a44", "#62a0ea",
    "#26a269", "#f5c211",
];

pub fn lane_color(lane: usize) -> gdk::RGBA {
    let c = LANE_COLORS[lane % LANE_COLORS.len()];
    c.parse().unwrap_or(gdk::RGBA::BLUE)
}

#[derive(Debug, Clone, Copy)]
pub struct DiffColors {
    pub add_bg: gdk::RGBA,
    pub add_line_bg: gdk::RGBA,
    pub del_bg: gdk::RGBA,
    pub del_line_bg: gdk::RGBA,
    pub word_bg: gdk::RGBA,
    pub hunk_bg: gdk::RGBA,
    pub hunk_fg: gdk::RGBA,
    pub ctx_fg: gdk::RGBA,
    pub dim_fg: gdk::RGBA,
}

#[allow(deprecated)]
fn lookup(widget: &impl IsA<gtk::Widget>, name: &str, fallback: (f32, f32, f32)) -> gdk::RGBA {
    widget
        .as_ref()
        .style_context()
        .lookup_color(name)
        .unwrap_or_else(|| gdk::RGBA::new(fallback.0, fallback.1, fallback.2, 1.0))
}

pub fn diff_colors(widget: &impl IsA<gtk::Widget>) -> DiffColors {
    let success = lookup(widget, "success_color", (0.2, 0.82, 0.48));
    let error = lookup(widget, "error_color", (0.9, 0.3, 0.24));
    let accent = lookup(widget, "accent_color", (0.2, 0.52, 0.9));
    let fg = lookup(widget, "window_fg_color", (0.9, 0.9, 0.9));
    let dim = gdk::RGBA::new(fg.red(), fg.green(), fg.blue(), 0.55);
    let mut add_bg = success;
    add_bg.set_alpha(0.16);
    let mut add_line_bg = success;
    add_line_bg.set_alpha(0.30);
    let mut del_bg = error;
    del_bg.set_alpha(0.16);
    let mut del_line_bg = error;
    del_line_bg.set_alpha(0.30);
    let mut word_bg = accent;
    word_bg.set_alpha(0.45);
    let mut hunk_bg = accent;
    hunk_bg.set_alpha(0.14);
    DiffColors {
        add_bg,
        add_line_bg,
        del_bg,
        del_line_bg,
        word_bg,
        hunk_bg,
        hunk_fg: accent,
        ctx_fg: fg,
        dim_fg: dim,
    }
}

pub fn rgba_to_css(c: &gdk::RGBA) -> String {
    format!(
        "rgba({},{},{},{})",
        (c.red() * 255.0).round() as u8,
        (c.green() * 255.0).round() as u8,
        (c.blue() * 255.0).round() as u8,
        c.alpha()
    )
}

/// Icon + css class + tooltip for a change kind.
pub fn change_icon(kind: ChangeKind) -> (&'static str, &'static str, &'static str) {
    match kind {
        ChangeKind::Added => ("list-add-symbolic", "success", "Added"),
        ChangeKind::Modified => ("document-edit-symbolic", "accent", "Modified"),
        ChangeKind::Deleted => ("user-trash-symbolic", "error", "Deleted"),
        ChangeKind::Renamed => ("edit-copy-symbolic", "warning", "Renamed"),
        ChangeKind::Copied => ("edit-copy-symbolic", "warning", "Copied"),
        ChangeKind::TypeChanged => ("emblem-synchronizing-symbolic", "warning", "Type changed"),
        ChangeKind::Untracked => ("text-x-generic-symbolic", "dim-label", "Untracked"),
        ChangeKind::Ignored => ("action-unavailable-symbolic", "dim-label", "Ignored"),
        ChangeKind::Unmerged => ("dialog-warning-symbolic", "error", "Conflicted"),
        ChangeKind::Unmodified => ("object-select-symbolic", "dim-label", "Unchanged"),
    }
}

pub fn conflict_label(kind: ConflictKind) -> &'static str {
    kind.description()
}

pub fn relative_time(timestamp: i64) -> String {
    if timestamp <= 0 {
        return String::new();
    }
    let now = chrono::Utc::now().timestamp();
    let delta = now - timestamp;
    if delta < 60 {
        "just now".into()
    } else if delta < 3600 {
        format!("{} min ago", delta / 60)
    } else if delta < 86400 {
        format!("{} h ago", delta / 3600)
    } else if delta < 86400 * 30 {
        format!("{} d ago", delta / 86400)
    } else {
        chrono::DateTime::from_timestamp(timestamp, 0)
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_default()
    }
}

pub fn absolute_time(timestamp: i64) -> String {
    chrono::DateTime::from_timestamp(timestamp, 0)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_default()
}

pub fn operation_label(op: &OperationState) -> String {
    match op {
        OperationState::None => String::new(),
        OperationState::Merge { .. } => "Merging — resolve conflicts and commit".into(),
        OperationState::Rebase {
            head_name,
            onto,
            done,
            total,
            ..
        } => {
            let progress = match (done, total) {
                (Some(d), Some(t)) => format!(" ({d}/{t})"),
                _ => String::new(),
            };
            format!(
                "Rebasing {}{} onto {}",
                head_name.as_deref().unwrap_or("branch"),
                progress,
                onto.as_deref().unwrap_or("upstream")
            )
        }
        OperationState::CherryPick { sha } => {
            format!("Cherry-picking {}", sha.as_deref().unwrap_or("commit"))
        }
        OperationState::Revert { sha } => {
            format!("Reverting {}", sha.as_deref().unwrap_or("commit"))
        }
        OperationState::Bisect => "Bisecting — mark good/bad commits".into(),
    }
}

pub fn short_oid(oid: &str) -> &str {
    if oid.len() >= 8 {
        &oid[..8]
    } else {
        oid
    }
}

pub fn copy_text(widget: &impl IsA<gtk::Widget>, text: &str) {
    widget.as_ref().clipboard().set_text(text);
}

pub fn open_uri(uri: &str) -> Result<(), glib::Error> {
    gio::AppInfo::launch_default_for_uri(uri, None::<&gio::AppLaunchContext>)
}

/// Open a file with the user's default application.
pub fn open_path(path: &Path) -> Result<(), glib::Error> {
    let uri = gio::File::for_path(path).uri();
    open_uri(&uri)
}

/// Show the containing folder of a file in the file manager.
pub fn open_containing_folder(path: &Path) -> Result<(), glib::Error> {
    let parent = path.parent().unwrap_or(path);
    let uri = gio::File::for_path(parent).uri();
    open_uri(&uri)
}

/// A dimmed, middle-ellipsized path label.
pub fn path_label(path: &str) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(path)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .xalign(0.0)
        .hexpand(true)
        .build();
    label.add_css_class("monospace");
    label
}

pub fn separator() -> gtk::Separator {
    gtk::Separator::new(gtk::Orientation::Horizontal)
}

/// Build a compact action button with an icon and tooltip.
pub fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .valign(gtk::Align::Center)
        .build();
    button.add_css_class("flat");
    button
}

pub fn menu_button(icon: &str, tooltip: &str) -> gtk::MenuButton {
    let button = gtk::MenuButton::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .valign(gtk::Align::Center)
        .build();
    button.add_css_class("flat");
    button
}

/// A header row for grouped file lists such as "Staged Changes".
pub fn list_header(title: &str) -> (gtk::Box, gtk::Label) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.set_margin_top(6);
    row.set_margin_bottom(2);
    row.set_margin_start(6);
    row.set_margin_end(6);
    let label = gtk::Label::builder()
        .label(title)
        .xalign(0.0)
        .hexpand(true)
        .build();
    label.add_css_class("heading");
    let count = gtk::Label::builder().label("").xalign(1.0).build();
    count.add_css_class("dim-label");
    row.append(&label);
    row.append(&count);
    (row, count)
}

/// Make a window closable with Escape.
pub fn close_on_escape(window: &impl IsA<gtk::Window>) {
    let window = window.clone().upcast::<gtk::Window>();
    let controller = gtk::EventControllerKey::new();
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    let win = window.clone();
    controller.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            win.close();
            gtk::glib::Propagation::Stop
        } else {
            gtk::glib::Propagation::Proceed
        }
    });
    window.add_controller(controller);
}

pub fn toast(overlay: &adw::ToastOverlay, message: &str) {
    let toast = adw::Toast::new(message);
    toast.set_timeout(3);
    overlay.add_toast(toast);
}

/// Show a non-fatal error using an alert dialog.
pub fn error_dialog(parent: &gtk::Window, error: &crate::git::GitError) {
    super::error_dialog::show(parent, error);
}
