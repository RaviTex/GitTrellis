//! Stash management.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

use crate::git::StashEntry;

use super::repo_page::UiRequest;
use super::util;

type Sender = Rc<dyn Fn(UiRequest)>;

pub struct StashesView {
    root: gtk::Box,
    list: gtk::ListBox,
    count: gtk::Label,
    request: RefCell<Option<Sender>>,
}

impl StashesView {
    pub fn new() -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);

        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        toolbar.set_margin_top(6);
        toolbar.set_margin_bottom(6);
        toolbar.set_margin_start(8);
        toolbar.set_margin_end(8);
        let hint = gtk::Label::builder()
            .label("Stashes are saved states of your working directory")
            .xalign(0.0)
            .hexpand(true)
            .build();
        hint.add_css_class("dim-label");
        let create = gtk::Button::builder()
            .label("Stash Changes")
            .icon_name("list-add-symbolic")
            .build();
        toolbar.append(&hint);
        toolbar.append(&create);

        let (header, count) = util::list_header("Stashes");
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
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        root.append(&scroll);

        let view = Rc::new_cyclic(|weak: &std::rc::Weak<Self>| {
            let w = weak.clone();
            create.connect_clicked(move |_| {
                if let Some(v) = w.upgrade() {
                    v.send(UiRequest::CreateStash);
                }
            });
            Self {
                root: root.clone(),
                list: list.clone(),
                count: count.clone(),
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

    pub fn render(&self, stashes: &[StashEntry]) {
        let sender = match self.request.borrow().clone() {
            Some(s) => s,
            None => return,
        };
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        for stash in stashes {
            self.list.append(&build_stash_row(stash, &sender));
        }
        self.count.set_label(&stashes.len().to_string());
        if stashes.is_empty() {
            let empty = gtk::Label::builder()
                .label("No stashes. Changes can be stashed from the toolbar.")
                .margin_top(16)
                .build();
            empty.add_css_class("dim-label");
            self.list.append(&empty);
        }
    }
}

fn build_stash_row(stash: &StashEntry, sender: &Sender) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    let widget = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    widget.set_margin_top(4);
    widget.set_margin_bottom(4);
    widget.set_margin_start(6);
    widget.set_margin_end(6);

    let icon = gtk::Image::from_icon_name("view-paged-symbolic");
    icon.set_pixel_size(14);
    icon.add_css_class("accent");

    let text = gtk::Box::new(gtk::Orientation::Vertical, 1);
    text.set_hexpand(true);
    let title = gtk::Label::builder()
        .label(&format!("stash@{{{}}}", stash.index))
        .xalign(0.0)
        .build();
    title.add_css_class("heading");
    let detail = if stash.message.is_empty() {
        stash.oid.chars().take(8).collect::<String>()
    } else {
        stash.message.clone()
    };
    let subtitle = gtk::Label::builder()
        .label(&detail)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    subtitle.add_css_class("dim-label");
    text.append(&title);
    text.append(&subtitle);

    let when = gtk::Label::new(Some(&util::relative_time(stash.time.unwrap_or(0))));
    when.add_css_class("dim-label");

    let menu = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .valign(gtk::Align::Center)
        .build();
    menu.add_css_class("flat");
    menu.set_popover(Some(&stash_popover(stash, sender.clone())));

    widget.append(&icon);
    widget.append(&text);
    widget.append(&when);
    widget.append(&menu);
    row.set_child(Some(&widget));
    row.set_tooltip_text(Some(&stash.message));
    row
}

fn stash_popover(stash: &StashEntry, sender: Sender) -> gtk::Popover {
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
    add("View Changes", UiRequest::ShowStash(stash.index), false);
    add("Apply (keep stash)", UiRequest::ApplyStash(stash.index), false);
    add("Apply and Restore Index", UiRequest::ApplyStashIndex(stash.index), false);
    add("Pop (apply and drop)", UiRequest::PopStash(stash.index), false);
    add(
        "Create Branch from Stash…",
        UiRequest::BranchFromStash(stash.index),
        false,
    );
    add("Drop Stash…", UiRequest::DropStash(stash.index), true);
    popover.set_child(Some(&box_));
    popover
}
