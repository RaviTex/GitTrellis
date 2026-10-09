//! Remote management and network operations.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

use crate::git::Remote;

use super::repo_page::UiRequest;
use super::util;

type Sender = Rc<dyn Fn(UiRequest)>;

pub struct RemotesView {
    root: gtk::Box,
    list: gtk::ListBox,
    count: gtk::Label,
    progress_label: gtk::Label,
    progress_bar: gtk::ProgressBar,
    progress_box: gtk::Box,
    cancel_button: gtk::Button,
    request: RefCell<Option<Sender>>,
}

impl RemotesView {
    pub fn new() -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);

        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        toolbar.set_margin_top(6);
        toolbar.set_margin_bottom(6);
        toolbar.set_margin_start(8);
        toolbar.set_margin_end(8);
        let fetch_all = gtk::Button::builder()
            .label("Fetch All")
            .icon_name("view-refresh-symbolic")
            .build();
        let push_tags = gtk::Button::builder()
            .label("Push Tags")
            .icon_name("tag-symbolic")
            .build();
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        let add = gtk::Button::builder()
            .label("Add Remote")
            .icon_name("list-add-symbolic")
            .build();
        toolbar.append(&fetch_all);
        toolbar.append(&push_tags);
        toolbar.append(&spacer);
        toolbar.append(&add);

        let progress_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        progress_box.set_margin_start(8);
        progress_box.set_margin_end(8);
        progress_box.set_margin_top(4);
        progress_box.set_margin_bottom(4);
        let progress_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let progress_label = gtk::Label::builder()
            .label("")
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        progress_label.add_css_class("monospace");
        progress_label.add_css_class("dim-label");
        let cancel_button = gtk::Button::with_label("Cancel");
        cancel_button.add_css_class("flat");
        progress_row.append(&progress_label);
        progress_row.append(&cancel_button);
        let progress_bar = gtk::ProgressBar::new();
        progress_box.append(&progress_row);
        progress_box.append(&progress_bar);
        progress_box.set_visible(false);

        let (header, count) = util::list_header("Remotes");
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::None);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
        content.set_margin_start(8);
        content.set_margin_end(8);
        content.set_margin_bottom(8);
        content.append(&header);
        content.append(&list);
        let scroll = gtk::ScrolledWindow::builder()
            .child(&content)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();

        root.append(&toolbar);
        root.append(&progress_box);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        root.append(&scroll);

        let view = Rc::new_cyclic(|weak: &std::rc::Weak<Self>| {
            {
                let w = weak.clone();
                fetch_all.connect_clicked(move |_| {
                    if let Some(v) = w.upgrade() {
                        v.send(UiRequest::FetchAll);
                    }
                });
            }
            {
                let w = weak.clone();
                push_tags.connect_clicked(move |_| {
                    if let Some(v) = w.upgrade() {
                        v.send(UiRequest::PushAllTags);
                    }
                });
            }
            {
                let w = weak.clone();
                add.connect_clicked(move |_| {
                    if let Some(v) = w.upgrade() {
                        v.send(UiRequest::AddRemote);
                    }
                });
            }
            Self {
                root: root.clone(),
                list: list.clone(),
                count: count.clone(),
                progress_label: progress_label.clone(),
                progress_bar: progress_bar.clone(),
                progress_box: progress_box.clone(),
                cancel_button: cancel_button.clone(),
                request: RefCell::new(None),
            }
        });
        view
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn connect_request(&self, request: Sender) {
        *self.request.borrow_mut() = Some(request);
    }

    fn send(&self, req: UiRequest) {
        if let Some(r) = self.request.borrow().as_ref() {
            r(req);
        }
    }

    pub fn render(&self, remotes: &[Remote]) {
        let sender = match self.request.borrow().clone() {
            Some(s) => s,
            None => return,
        };
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        for remote in remotes {
            self.list.append(&build_remote_row(remote, &sender));
        }
        self.count.set_label(&remotes.len().to_string());
        if remotes.is_empty() {
            let empty = gtk::Label::builder()
                .label("No remotes configured. Add one to fetch, pull and push.")
                .margin_top(16)
                .build();
            empty.add_css_class("dim-label");
            self.list.append(&empty);
        }
    }

    /// Show progress of a network operation.
    pub fn set_progress(&self, label: Option<&str>, fraction: Option<f64>) {
        match label {
            Some(text) => {
                self.progress_box.set_visible(true);
                self.progress_label.set_label(text);
                if let Some(f) = fraction {
                    self.progress_bar.set_fraction(f);
                    self.progress_bar.set_show_text(true);
                } else {
                    self.progress_bar.pulse();
                    self.progress_bar.set_show_text(false);
                }
            }
            None => {
                self.progress_box.set_visible(false);
                self.progress_label.set_label("");
            }
        }
    }

    pub fn cancel_button(&self) -> &gtk::Button {
        &self.cancel_button
    }
}

fn build_remote_row(remote: &Remote, sender: &Sender) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    let widget = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    widget.set_margin_top(4);
    widget.set_margin_bottom(4);
    widget.set_margin_start(6);
    widget.set_margin_end(6);

    let icon = gtk::Image::from_icon_name("network-server-symbolic");
    icon.set_pixel_size(14);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 1);
    text.set_hexpand(true);
    let name = gtk::Label::builder()
        .label(&remote.name)
        .xalign(0.0)
        .build();
    name.add_css_class("heading");
    let url = gtk::Label::builder()
        .label(&remote.fetch_url)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .build();
    url.add_css_class("dim-label");
    url.add_css_class("monospace");
    text.append(&name);
    text.append(&url);

    let fetch = gtk::Button::builder()
        .label("Fetch")
        .valign(gtk::Align::Center)
        .build();
    fetch.add_css_class("flat");
    {
        let sender = sender.clone();
        let name = remote.name.clone();
        fetch.connect_clicked(move |_| {
            sender(UiRequest::FetchRemote(name.clone()));
        });
    }
    let push = gtk::Button::builder()
        .label("Push")
        .valign(gtk::Align::Center)
        .build();
    push.add_css_class("flat");
    {
        let sender = sender.clone();
        let name = remote.name.clone();
        push.connect_clicked(move |_| {
            sender(UiRequest::PushTo(name.clone()));
        });
    }

    let menu = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .valign(gtk::Align::Center)
        .build();
    menu.add_css_class("flat");
    menu.set_popover(Some(&remote_popover(remote, sender.clone())));

    widget.append(&icon);
    widget.append(&text);
    widget.append(&fetch);
    widget.append(&push);
    widget.append(&menu);
    row.set_child(Some(&widget));
    row.set_tooltip_text(Some(&remote.fetch_url));
    row
}

fn remote_popover(remote: &Remote, sender: Sender) -> gtk::Popover {
    let popover = gtk::Popover::new();
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 2);
    box_.set_margin_start(6);
    box_.set_margin_end(6);
    box_.set_margin_top(6);
    box_.set_margin_bottom(6);
    let add = |label: &str, req: UiRequest, destructive: bool| {
        let button = gtk::Button::builder()
            .label(label)
            .has_frame(false)
            .halign(gtk::Align::Fill)
            .build();
        if destructive {
            button.add_css_class("destructive-action");
        }
        let sender = sender.clone();
        let pop = popover.clone();
        button.connect_clicked(move |_| {
            sender(req.clone());
            pop.popdown();
        });
        box_.append(&button);
    };
    add("Fetch", UiRequest::FetchRemote(remote.name.clone()), false);
    add("Pull…", UiRequest::PullFrom(remote.name.clone()), false);
    add("Push…", UiRequest::PushTo(remote.name.clone()), false);
    add("Push All Tags", UiRequest::PushAllTagsTo(remote.name.clone()), false);
    add("Copy URL", UiRequest::CopyText(remote.fetch_url.clone()), false);
    add("Edit Remote…", UiRequest::EditRemote(remote.name.clone()), false);
    add(
        "Rename Remote…",
        UiRequest::RenameRemote(remote.name.clone()),
        false,
    );
    add(
        "Remove Remote…",
        UiRequest::RemoveRemote(remote.name.clone()),
        true,
    );
    popover.set_child(Some(&box_));
    popover
}
