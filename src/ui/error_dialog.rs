//! User facing error reporting: human readable message, recovery hint and
//! copyable technical details.

use adw::prelude::*;

use crate::git::GitError;

pub fn show(parent: &impl IsA<gtk::Window>, error: &GitError) {
    crate::log_error!("{error}");
    let details = error.technical_details();
    let has_details = !details.trim().is_empty();

    // If the user cancelled the passphrase dialog, the raw Git error looks
    // like an authentication failure; explain what actually happened.
    if matches!(error, GitError::AuthFailed { .. }) && crate::ui::askpass::cancelled_recently() {
        let dialog = adw::MessageDialog::new(
            Some(parent),
            Some("Authentication cancelled"),
            Some(
                "The authentication prompt was cancelled, so the operation did not run.\n\nStart it again when you are ready and enter your passphrase in the dialog.",
            ),
        );
        dialog.add_response("close", "Close");
        dialog.set_default_response(Some("close"));
        dialog.set_close_response("close");
        dialog.present();
        return;
    }

    let dialog = adw::MessageDialog::new(
        Some(parent),
        Some(&user_heading(error)),
        Some(&error.user_message()),
    );
    dialog.set_body_use_markup(false);

    if let Some(recovery) = error.recovery_action() {
        let recovery_label = gtk::Label::builder()
            .label(format!("Suggested action: {recovery}"))
            .xalign(0.0)
            .wrap(true)
            .build();
        recovery_label.add_css_class("dim-label");
        dialog.set_extra_child(Some(&recovery_label));
    } else if has_details {
        dialog.set_extra_child(Some(&details_expander(&details)));
    }

    dialog.add_response("close", "Close");
    dialog.set_default_response(Some("close"));
    dialog.set_close_response("close");

    if has_details {
        dialog.add_response("copy", "Copy details");
        dialog.set_response_appearance("copy", adw::ResponseAppearance::Suggested);
        let details_clone = details.clone();
        dialog.connect_response(Some("copy"), move |d, _| {
            d.clipboard().set_text(&details_clone);
            d.close();
        });
    }

    dialog.present();
}

fn details_expander(details: &str) -> gtk::Widget {
    let view = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .wrap_mode(gtk::WrapMode::WordChar)
        .build();
    view.buffer().set_text(details);
    view.add_css_class("dim-label");
    let scroller = gtk::ScrolledWindow::builder()
        .child(&view)
        .min_content_height(120)
        .max_content_height(260)
        .build();
    let expander = gtk::Expander::builder()
        .label("Technical details")
        .child(&scroller)
        .build();
    expander.upcast()
}

fn user_heading(error: &GitError) -> String {
    match error {
        GitError::GitNotFound => "Git is not installed".into(),
        GitError::NotARepository { .. } => "Not a Git repository".into(),
        GitError::AuthFailed { .. } => "Authentication failed".into(),
        GitError::CorruptRepository { .. } => "Repository problem".into(),
        GitError::Cancelled => "Operation cancelled".into(),
        GitError::InvalidState { .. } => "Action not possible".into(),
        GitError::CommandFailed { .. } => "Git reported an error".into(),
        GitError::Parse { .. } => "Unexpected Git output".into(),
        GitError::Io(_) => "I/O error".into(),
    }
}

/// Log and show an error, returning control to the caller.
pub fn show_logged(parent: &impl IsA<gtk::Window>, context: &str, error: &GitError) {
    crate::log_error!("{context}: {}", error.technical_details());
    show(parent, error);
}

/// Show an informational message.
pub fn info(parent: &impl IsA<gtk::Window>, heading: &str, body: &str) {
    let dialog = adw::MessageDialog::new(Some(parent), Some(heading), Some(body));
    dialog.add_response("close", "Close");
    dialog.set_default_response(Some("close"));
    dialog.set_close_response("close");
    dialog.present();
}

