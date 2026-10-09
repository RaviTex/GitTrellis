//! Commit panel: message entry, amend, sign-off, GPG signing and commit.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;

use crate::git::CommitOptions;
use crate::log_debug;

use super::repo_page::UiRequest;

pub struct CommitPanel {
    root: gtk::Box,
    message_view: gtk::TextView,
    summary_label: gtk::Label,
    amend_check: gtk::CheckButton,
    signoff_check: gtk::CheckButton,
    gpg_check: gtk::CheckButton,
    commit_button: gtk::Button,
    history_button: gtk::MenuButton,
    request: RefCell<Option<Rc<dyn Fn(UiRequest)>>>,
    staged_count: Cell<usize>,
    recent_messages: RefCell<Vec<String>>,
}

impl CommitPanel {
    pub fn new() -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
        root.set_margin_top(8);
        root.set_margin_bottom(8);
        root.set_margin_start(10);
        root.set_margin_end(10);

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let title = gtk::Label::builder()
            .label("Commit message")
            .xalign(0.0)
            .hexpand(true)
            .build();
        title.add_css_class("heading");
        let history_button = gtk::MenuButton::builder()
            .icon_name("document-open-recent-symbolic")
            .tooltip_text("Recent messages")
            .build();
        history_button.add_css_class("flat");
        header.append(&title);
        header.append(&history_button);

        let message_view = gtk::TextView::builder()
            .wrap_mode(gtk::WrapMode::WordChar)
            .accepts_tab(false)
            .height_request(88)
            .build();
        let message_scroll = gtk::ScrolledWindow::builder()
            .child(&message_view)
            .min_content_height(88)
            .max_content_height(200)
            .has_frame(true)
            .build();

        let summary_label = gtk::Label::builder()
            .label("Nothing staged")
            .xalign(0.0)
            .hexpand(true)
            .build();
        summary_label.add_css_class("dim-label");

        let checks = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let amend_check = gtk::CheckButton::with_label("Amend");
        amend_check.set_tooltip_text(Some("Replace the previous commit with this one"));
        let signoff_check = gtk::CheckButton::with_label("Sign-off");
        signoff_check.set_tooltip_text(Some("Append a Signed-off-by trailer"));
        let gpg_check = gtk::CheckButton::with_label("Sign");
        gpg_check.set_tooltip_text(Some("GPG/SSH sign this commit"));
        checks.append(&amend_check);
        checks.append(&signoff_check);
        checks.append(&gpg_check);

        let commit_button = gtk::Button::with_label("Commit");
        commit_button.add_css_class("suggested-action");
        commit_button.set_sensitive(false);
        commit_button.set_tooltip_text(Some("Commit staged changes (Ctrl+Enter)"));

        let button_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        button_row.append(&summary_label);
        button_row.append(&commit_button);

        root.append(&header);
        root.append(&message_scroll);
        root.append(&checks);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        root.append(&button_row);

        let panel = Rc::new_cyclic(|weak: &std::rc::Weak<Self>| {
            let click_panel = weak.clone();
            commit_button.connect_clicked(move |_| {
                if let Some(panel) = click_panel.upgrade() {
                    panel.submit();
                }
            });
            let change_panel = weak.clone();
            message_view.buffer().connect_changed(move |_| {
                if let Some(panel) = change_panel.upgrade() {
                    panel.refresh_button();
                }
            });
            let amend_weak = weak.clone();
            amend_check.connect_toggled(move |_| {
                if let Some(panel) = amend_weak.upgrade() {
                    panel.refresh_button();
                }
            });
            Self {
                root: root.clone(),
                message_view: message_view.clone(),
                summary_label: summary_label.clone(),
                amend_check: amend_check.clone(),
                signoff_check: signoff_check.clone(),
                gpg_check: gpg_check.clone(),
                commit_button: commit_button.clone(),
                history_button: history_button.clone(),
                request: RefCell::new(None),
                staged_count: Cell::new(0),
                recent_messages: RefCell::new(Vec::new()),
            }
        });
        panel
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn connect_request(&self, request: Rc<dyn Fn(UiRequest)>) {
        *self.request.borrow_mut() = Some(request);
    }

    pub fn set_recent_messages(&self, messages: &[String]) {
        *self.recent_messages.borrow_mut() = messages.to_vec();
        let popover = gtk::Popover::new();
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.set_margin_start(4);
        list.set_margin_end(4);
        list.set_margin_top(4);
        list.set_margin_bottom(4);
        if messages.is_empty() {
            let empty = gtk::Label::builder()
                .label("No recent messages")
                .margin_start(8).margin_end(8).margin_top(8).margin_bottom(8)
                .build();
            empty.add_css_class("dim-label");
            list.append(&empty);
        }
        for message in messages.iter().take(15) {
            let btn = gtk::Button::builder()
                .label(message)
                .has_frame(false)
                .halign(gtk::Align::Fill)
                .build();
            if let Some(label) = btn.child().and_then(|c| c.downcast::<gtk::Label>().ok()) {
                label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                label.set_xalign(0.0);
                label.set_max_width_chars(60);
            }
            let text = message.clone();
            let view = self.message_view.clone();
            let pop = popover.clone();
            btn.connect_clicked(move |_| {
                view.buffer().set_text(&text);
                pop.popdown();
            });
            list.append(&btn);
        }
        let scroller = gtk::ScrolledWindow::builder()
            .child(&list)
            .max_content_height(320)
            .propagate_natural_height(true)
            .build();
        popover.set_child(Some(&scroller));
        self.history_button.set_popover(Some(&popover));
    }

    pub fn message(&self) -> String {
        let buffer = self.message_view.buffer();
        buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .to_string()
    }

    pub fn clear_message(&self) {
        self.message_view.buffer().set_text("");
    }

    pub fn set_message(&self, text: &str) {
        self.message_view.buffer().set_text(text);
    }

    pub fn amend(&self) -> bool {
        self.amend_check.is_active()
    }

    pub fn set_amend(&self, amend: bool) {
        self.amend_check.set_active(amend);
    }

    pub fn confirm_amend(&self, previous_message: &str) {
        self.set_amend(true);
        if self.message().trim().is_empty() {
            self.set_message(previous_message);
        }
    }

    pub fn set_signoff(&self, value: bool) {
        self.signoff_check.set_active(value);
    }

    pub fn signoff(&self) -> bool {
        self.signoff_check.is_active()
    }

    pub fn set_gpg(&self, value: bool) {
        self.gpg_check.set_active(value);
    }

    pub fn gpg(&self) -> bool {
        self.gpg_check.is_active()
    }

    pub fn update_state(&self, staged_count: usize) {
        self.staged_count.set(staged_count);
        self.refresh_button();
    }

    fn refresh_button(&self) {
        let staged = self.staged_count.get();
        let amend = self.amend_check.is_active();
        let has_message = !self.message().trim().is_empty();
        let can = (staged > 0 || amend) && has_message;
        self.commit_button.set_sensitive(can);
        if amend {
            self.summary_label.set_label("Amending previous commit");
        } else if staged == 0 {
            self.summary_label.set_label("Nothing staged");
        } else {
            self.summary_label
                .set_label(&format!("{staged} file(s) staged"));
        }
    }

    pub fn commit_options(&self) -> CommitOptions {
        let gpg = self.gpg_check.is_active();
        CommitOptions {
            message: self.message(),
            amend: self.amend_check.is_active(),
            sign_off: self.signoff_check.is_active(),
            gpg_sign: if gpg { Some(None) } else { None },
            allow_empty: false,
            author: None,
        }
    }

    /// Emit the commit request; returns false when there is nothing to do.
    pub fn submit(&self) -> bool {
        let staged = self.staged_count.get();
        let amend = self.amend_check.is_active();
        if self.message().trim().is_empty() {
            return false;
        }
        if staged == 0 && !amend {
            return false;
        }
        let opts = self.commit_options();
        log_debug!("commit requested (amend={})", opts.amend);
        if let Some(r) = self.request.borrow().as_ref() {
            r(UiRequest::Commit(Box::new(opts)));
        }
        true
    }

    pub fn focus_message(&self) {
        self.message_view.grab_focus();
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.root.set_sensitive(enabled);
    }
}
