//! Askpass helper.
//!
//! When `ssh` or `git` needs a passphrase, password or username and no
//! terminal is available, OpenSSH and Git run the program named by
//! `SSH_ASKPASS` / `GIT_ASKPASS` with the prompt as the first argument and
//! read the answer from its stdout. Stratum points both variables at its own
//! helper binary; this module implements that mode and shows a native dialog.
//!
//! Security notes:
//! * The secret is only ever written to stdout for the waiting process. It is
//!   never logged, stored or echoed.
//! * The process always exits explicitly after answering or cancelling, so the
//!   waiting `ssh`/`git` process can never be left hanging.
//!
//! This helper intentionally uses plain GTK, not libadwaita: it does not call
//! `adw::init()` (there is no `AdwApplication` here) and only needs a small
//! dialog.

use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gtk::prelude::*;

/// Test hook: when set to `1`, the answer comes from `STRATUM_ASKPASS_ANSWER`
/// without showing a dialog. Used by automated tests.
pub const ASKPASS_TEST_MODE_ENV: &str = "STRATUM_ASKPASS_TEST_MODE";
pub const ASKPASS_ANSWER_ENV: &str = "STRATUM_ASKPASS_ANSWER";
/// Test hook: when set (with test mode) the real dialog is driven
/// automatically after a short delay. Value `cancel` cancels the dialog.
pub const ASKPASS_AUTOANSWER_ENV: &str = "STRATUM_ASKPASS_AUTOANSWER";

/// Remember the timestamp of the last cancelled prompt so the UI can explain
/// the resulting failure as "cancelled" instead of "authentication failed".
pub fn mark_cancelled() {
    let dir = crate::core::Settings::state_dir();
    let _ = std::fs::create_dir_all(&dir);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let _ = std::fs::write(dir.join("askpass-cancelled"), now.to_string());
}

/// True when the askpass dialog was cancelled within the last 15 seconds.
pub fn cancelled_recently() -> bool {
    let path = crate::core::Settings::state_dir().join("askpass-cancelled");
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(when) = text.trim().parse::<u64>() else {
        return false;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    now.saturating_sub(when) <= 15
}

/// Extract the key path from prompts such as
/// `Enter passphrase for key '/home/user/.ssh/id_ed25519':`.
fn key_path_from_prompt(prompt: &str) -> Option<String> {
    let start = prompt.find('\'')? + 1;
    let rest = &prompt[start..];
    let end = rest.find('\'')?;
    let path = &rest[..end];
    if path.is_empty() {
        None
    } else {
        Some(path.to_string())
    }
}

/// Load the key into the user's SSH agent so the passphrase is only needed
/// once per session. The passphrase is passed through the environment of a
/// short-lived helper process, never written to disk.
fn spawn_ssh_add(key: &str, passphrase: &str) {
    let Ok(helper) = std::env::current_exe() else {
        return;
    };
    let result = std::process::Command::new("ssh-add")
        .arg(key)
        .env("SSH_ASKPASS", &helper)
        .env("SSH_ASKPASS_REQUIRE", "force")
        .env(ASKPASS_TEST_MODE_ENV, "1")
        .env(ASKPASS_ANSWER_ENV, passphrase)
        .env_remove(ASKPASS_AUTOANSWER_ENV)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if let Err(e) = result {
        crate::log_debug!("ssh-add not available: {e}");
    }
}

pub fn run() -> gtk::glib::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let prompt = args
        .iter()
        .skip(1)
        .find(|a| *a != "--askpass")
        .cloned()
        .unwrap_or_else(|| "Authentication required".to_string());

    // Automated tests: answer from the environment, no display needed.
    if std::env::var(ASKPASS_TEST_MODE_ENV).as_deref() == Ok("1")
        && std::env::var(ASKPASS_AUTOANSWER_ENV).is_err()
    {
        return match std::env::var(ASKPASS_ANSWER_ENV) {
            Ok(answer) => {
                emit(&answer);
                gtk::glib::ExitCode::SUCCESS
            }
            Err(_) => gtk::glib::ExitCode::FAILURE,
        };
    }

    if gtk::init().is_err() {
        eprintln!("stratum-askpass: no display available");
        return gtk::glib::ExitCode::FAILURE;
    }

    let answer: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let window = gtk::Window::builder()
        .title("Authentication required")
        .resizable(false)
        .default_width(460)
        .build();

    // Plain GTK header bar: gives the dialog a title and close button without
    // requiring libadwaita initialization.
    let header = gtk::HeaderBar::new();
    window.set_titlebar(Some(&header));

    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_start(18);
    content.set_margin_end(18);
    content.set_margin_top(18);
    content.set_margin_bottom(18);

    let header_box = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let icon = gtk::Image::from_icon_name("dialog-password-symbolic");
    icon.set_pixel_size(32);
    icon.add_css_class("dim-label");
    let text_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    text_box.set_hexpand(true);
    let title = gtk::Label::builder()
        .label("Authentication required")
        .xalign(0.0)
        .build();
    title.add_css_class("heading");
    let prompt_label = gtk::Label::builder()
        .label(&prompt)
        .xalign(0.0)
        .wrap(true)
        .selectable(true)
        .build();
    prompt_label.add_css_class("dim-label");
    text_box.append(&title);
    text_box.append(&prompt_label);
    header_box.append(&icon);
    header_box.append(&text_box);

    let entry = gtk::PasswordEntry::builder()
        .show_peek_icon(true)
        .activates_default(true)
        .hexpand(true)
        .build();

    let is_passphrase = prompt.to_lowercase().contains("passphrase");
    let hint = gtk::Label::builder()
        .label(
            "Tip: add the key to your SSH agent once per session with `ssh-add` to avoid being asked again.",
        )
        .xalign(0.0)
        .wrap(true)
        .build();
    hint.add_css_class("dim-label");
    hint.set_visible(is_passphrase);

    {
        let entry = entry.clone();
        entry.connect_changed(move |e| {
            e.remove_css_class("error");
        });
    }

    let key_path = if is_passphrase {
        key_path_from_prompt(&prompt)
    } else {
        None
    };
    let agent_check = gtk::CheckButton::with_label("Add the key to SSH agent for this session");
    agent_check.set_tooltip_text(Some(
        "Loads the key with ssh-add so you are not asked again in this session. The passphrase is not stored.",
    ));
    agent_check.set_active(true);
    agent_check.set_visible(key_path.is_some());

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancel");
    let ok = gtk::Button::with_label("Continue");
    ok.add_css_class("suggested-action");
    ok.set_receives_default(true);
    buttons.append(&cancel);
    buttons.append(&ok);

    content.append(&header_box);
    content.append(&entry);
    content.append(&hint);
    content.append(&agent_check);
    content.append(&buttons);
    window.set_child(Some(&content));

    // Private main loop for this helper process.
    let main_loop = gtk::glib::MainLoop::new(None, false);

    {
        let answer = Rc::clone(&answer);
        let entry = entry.clone();
        let main_loop = main_loop.clone();
        ok.connect_clicked(move |_| {
            let value = entry.text().to_string();
            if value.is_empty() {
                // Do not silently ignore the click: mark the field and
                // return focus so the user can see what is missing.
                entry.add_css_class("error");
                entry.grab_focus();
                return;
            }
            entry.remove_css_class("error");
            *answer.borrow_mut() = Some(value);
            main_loop.quit();
        });
    }
    {
        let main_loop = main_loop.clone();
        cancel.connect_clicked(move |_| {
            main_loop.quit();
        });
    }
    {
        let main_loop = main_loop.clone();
        window.connect_close_request(move |_| {
            main_loop.quit();
            gtk::glib::Propagation::Proceed
        });
    }
    // Escape cancels.
    {
        let main_loop = main_loop.clone();
        let escape = gtk::EventControllerKey::new();
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                main_loop.quit();
                return gtk::glib::Propagation::Stop;
            }
            gtk::glib::Propagation::Proceed
        });
        window.add_controller(escape);
    }

    // Test hook: drive the real dialog automatically.
    if std::env::var(ASKPASS_TEST_MODE_ENV).as_deref() == Ok("1") {
        if let Ok(auto) = std::env::var(ASKPASS_AUTOANSWER_ENV) {
            let entry = entry.clone();
            let ok = ok.clone();
            let cancel = cancel.clone();
            gtk::glib::timeout_add_local_once(Duration::from_millis(300), move || {
                if auto == "cancel" || auto.is_empty() {
                    cancel.emit_clicked();
                } else {
                    entry.set_text(&auto);
                    ok.emit_clicked();
                }
            });
        }
    }

    // The field must be ready to type into immediately; otherwise the user's
    // keystrokes go nowhere and the dialog appears to do nothing.
    gtk::prelude::GtkWindowExt::set_default_widget(&window, Some(&ok));
    gtk::prelude::GtkWindowExt::set_focus(&window, Some(&entry));
    window.present();
    entry.grab_focus();
    main_loop.run();

    let result = answer.borrow().clone();
    match result {
        Some(value) => {
            if agent_check.is_active() {
                if let Some(key) = key_path {
                    spawn_ssh_add(&key, &value);
                }
            }
            emit(&value);
            // Exit explicitly: no GTK teardown can keep ssh waiting.
            std::process::exit(0);
        }
        None => {
            mark_cancelled();
            std::process::exit(1);
        }
    }
}

fn emit(value: &str) {
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(value.as_bytes());
    let _ = stdout.write_all(b"\n");
    let _ = stdout.flush();
}
