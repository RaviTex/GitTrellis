//! Tag management.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

use crate::git::Tag;

use super::repo_page::UiRequest;
use super::util;

type Sender = Rc<dyn Fn(UiRequest)>;

pub struct TagsView {
    root: gtk::Box,
    list: gtk::ListBox,
    filter: gtk::SearchEntry,
    count: gtk::Label,
    request: RefCell<Option<Sender>>,
}

impl TagsView {
    pub fn new() -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);

        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        toolbar.set_margin_top(6);
        toolbar.set_margin_bottom(6);
        toolbar.set_margin_start(8);
        toolbar.set_margin_end(8);
        let filter = gtk::SearchEntry::builder()
            .placeholder_text("Filter tags…")
            .hexpand(true)
            .build();
        let create = gtk::Button::builder()
            .label("New Tag")
            .icon_name("list-add-symbolic")
            .build();
        toolbar.append(&filter);
        toolbar.append(&create);

        let (header, count) = util::list_header("Tags");
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
            {
                let w = weak.clone();
                create.connect_clicked(move |_| {
                    if let Some(v) = w.upgrade() {
                        v.send(UiRequest::CreateTag);
                    }
                });
            }
            {
                let w = weak.clone();
                filter.connect_search_changed(move |_| {
                    if let Some(v) = w.upgrade() {
                        v.apply_filter();
                    }
                });
            }
            Self {
                root: root.clone(),
                list: list.clone(),
                filter: filter.clone(),
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

    pub fn focus_filter(&self) {
        self.filter.grab_focus();
    }

    pub fn render(&self, tags: &[Tag]) {
        let sender = match self.request.borrow().clone() {
            Some(s) => s,
            None => return,
        };
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        for tag in tags {
            self.list.append(&build_tag_row(tag, &sender));
        }
        self.count.set_label(&tags.len().to_string());
        self.apply_filter();
    }

    fn apply_filter(&self) {
        let query = self.filter.text().to_lowercase();
        let mut child = self.list.first_child();
        while let Some(row) = child {
            child = row.next_sibling();
            let matches = query.is_empty()
                || row
                    .downcast_ref::<gtk::ListBoxRow>()
                    .and_then(|r| r.tooltip_text())
                    .map(|t| t.to_lowercase().contains(&query))
                    .unwrap_or(true);
            row.set_visible(matches);
        }
    }
}

fn build_tag_row(tag: &Tag, sender: &Sender) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    let widget = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    widget.set_margin_top(4);
    widget.set_margin_bottom(4);
    widget.set_margin_start(6);
    widget.set_margin_end(6);

    let icon = gtk::Image::from_icon_name("tag-symbolic");
    icon.set_pixel_size(14);
    icon.add_css_class("success");

    let text = gtk::Box::new(gtk::Orientation::Vertical, 1);
    text.set_hexpand(true);
    let name = gtk::Label::builder()
        .label(&tag.name)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    name.add_css_class("heading");
    let subtitle_text = if tag.is_annotated {
        format!("annotated · {}", tag.subject)
    } else {
        format!("lightweight · {}", util::short_oid(&tag.target))
    };
    let subtitle = gtk::Label::builder()
        .label(&subtitle_text)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    subtitle.add_css_class("dim-label");
    text.append(&name);
    text.append(&subtitle);

    let menu = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .valign(gtk::Align::Center)
        .build();
    menu.add_css_class("flat");
    menu.set_popover(Some(&tag_popover(tag, sender.clone())));

    widget.append(&icon);
    widget.append(&text);
    widget.append(&menu);
    row.set_child(Some(&widget));
    row.set_tooltip_text(Some(&tag.name));
    row
}

fn tag_popover(tag: &Tag, sender: Sender) -> gtk::Popover {
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
    add("Checkout (detached)", UiRequest::CheckoutTag(tag.name.clone()), false);
    add("Show in History", UiRequest::ShowTagInHistory(tag.name.clone()), false);
    add("Push Tag…", UiRequest::PushTag(tag.name.clone()), false);
    add("Copy Tag Name", UiRequest::CopyText(tag.name.clone()), false);
    add("Delete Tag…", UiRequest::DeleteTag(tag.name.clone()), true);
    add(
        "Delete Remote Tag…",
        UiRequest::DeleteRemoteTag(tag.name.clone()),
        true,
    );
    popover.set_child(Some(&box_));
    popover
}
