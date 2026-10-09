//! Confirmation dialogs for destructive and sensitive operations.

use adw::prelude::*;

/// Show a question with an optional destructive confirm button.
pub fn confirm<F>(
    parent: &impl IsA<gtk::Window>,
    heading: &str,
    body: &str,
    ok_label: &str,
    destructive: bool,
    on_confirm: F,
) where
    F: Fn() + 'static,
{
    let dialog = adw::MessageDialog::new(Some(parent), Some(heading), Some(body));
    dialog.set_body_use_markup(false);
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("ok", ok_label);
    if destructive {
        dialog.set_response_appearance("ok", adw::ResponseAppearance::Destructive);
    } else {
        dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    }
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    let on_confirm = std::rc::Rc::new(on_confirm);
    dialog.connect_response(None, move |_, response| {
        if response == "ok" {
            on_confirm();
        }
    });
    dialog.present();
}

/// A destructive confirmation that additionally requires the user to tick a
/// checkbox. Used for operations that can lose data.
pub fn confirm_dangerous<F>(
    parent: &impl IsA<gtk::Window>,
    heading: &str,
    body: &str,
    explanation: &str,
    ok_label: &str,
    on_confirm: F,
) where
    F: Fn() + 'static,
{
    let dialog = adw::MessageDialog::new(Some(parent), Some(heading), Some(body));
    dialog.set_body_use_markup(false);

    let explain = gtk::Label::builder()
        .label(explanation)
        .xalign(0.0)
        .wrap(true)
        .build();
    explain.add_css_class("dim-label");
    let check = gtk::CheckButton::builder()
        .label("I understand this cannot be undone")
        .margin_top(6)
        .build();
    let extra = gtk::Box::new(gtk::Orientation::Vertical, 8);
    extra.append(&explain);
    extra.append(&check);
    dialog.set_extra_child(Some(&extra));

    dialog.add_response("cancel", "Cancel");
    dialog.add_response("ok", ok_label);
    dialog.set_response_appearance("ok", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    let on_confirm = std::rc::Rc::new(on_confirm);
    let dialog_weak = dialog.downgrade();
    let check_for_response = check.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "ok" {
            return;
        }
        if !check_for_response.is_active() {
            // Require the acknowledgement before proceeding.
            check_for_response.add_css_class("error");
            if let Some(dialog) = dialog_weak.upgrade() {
                dialog.present();
            }
            return;
        }
        on_confirm();
    });
    dialog.present();
}

/// Simple text prompt dialog.
pub fn prompt<F>(parent: &impl IsA<gtk::Window>, heading: &str, placeholder: &str, ok_label: &str, on_ok: F)
where
    F: Fn(String) + 'static,
{
    let dialog = adw::MessageDialog::new(Some(parent), Some(heading), None);
    let entry = gtk::Entry::builder()
        .placeholder_text(placeholder)
        .activates_default(true)
        .build();
    dialog.set_extra_child(Some(&entry));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("ok", ok_label);
    dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("ok"));
    dialog.set_close_response("cancel");
    let on_ok = std::rc::Rc::new(on_ok);
    dialog.connect_response(None, move |d, response| {
        if response == "ok" {
            let text = entry.text().to_string();
            on_ok(text);
        }
        d.close();
    });
    dialog.present();
}
