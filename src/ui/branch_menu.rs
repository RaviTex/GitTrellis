//! Searchable branch switcher in the repository header.

use std::cell::RefCell;

use adw::prelude::*;

use crate::git::Branch;

use super::repo_page::{Sender, UiRequest};

pub(super) struct BranchMenu {
    popover: gtk::Popover,
    search: gtk::SearchEntry,
    locals: gtk::Box,
    remotes: gtk::Box,
    branches: RefCell<Vec<Branch>>,
    sender: Sender,
}

impl BranchMenu {
    pub(super) fn new(button: &gtk::MenuButton, sender: Sender) -> Self {
        let popover = gtk::Popover::new();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
        content.set_margin_start(6);
        content.set_margin_end(6);
        content.set_margin_top(6);
        content.set_margin_bottom(6);

        let search = gtk::SearchEntry::builder()
            .placeholder_text("Filter branches…")
            .build();
        search.update_property(&[gtk::accessible::Property::Label("Filter branches")]);
        content.append(&search);

        let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let locals = add_section(&list, "Local branches");
        let remotes = add_section(&list, "Remote branches");
        let scroll = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .max_content_height(360)
            .min_content_width(300)
            .propagate_natural_height(true)
            .build();
        content.append(&scroll);
        content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        for (label, request) in [
            ("New Branch…", UiRequest::CreateBranch),
            ("Pull", UiRequest::QuickPull),
            ("Push", UiRequest::QuickPush),
            ("Pull with options…", UiRequest::PullOptions),
        ] {
            let button = gtk::Button::with_label(label);
            button.set_has_frame(false);
            button.set_halign(gtk::Align::Fill);
            connect_action(&button, &popover, &sender, request);
            content.append(&button);
        }

        let local_filter = locals.clone();
        let remote_filter = remotes.clone();
        search.connect_search_changed(move |entry| {
            let query = entry.text().to_lowercase();
            filter_rows(&local_filter, &query);
            filter_rows(&remote_filter, &query);
        });

        filter_rows(&locals, "");
        filter_rows(&remotes, "");
        popover.set_child(Some(&content));
        button.set_popover(Some(&popover));
        Self {
            popover,
            search,
            locals,
            remotes,
            branches: RefCell::new(Vec::new()),
            sender,
        }
    }

    pub(super) fn render(&self, branches: &[Branch]) {
        if self.branches.borrow().as_slice() == branches {
            return;
        }
        *self.branches.borrow_mut() = branches.to_vec();

        for list in [&self.locals, &self.remotes] {
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
        }
        let mut branches: Vec<_> = branches.iter().filter(|b| !b.is_remote_head()).collect();
        branches.sort_by(|a, b| b.is_head.cmp(&a.is_head).then_with(|| a.name.cmp(&b.name)));
        for branch in branches {
            let list = if branch.is_remote {
                &self.remotes
            } else {
                &self.locals
            };
            list.append(&branch_row(branch, &self.popover, &self.sender));
        }

        let query = self.search.text().to_lowercase();
        filter_rows(&self.locals, &query);
        filter_rows(&self.remotes, &query);
    }

    pub(super) fn open(&self) {
        self.popover.popup();
        self.search.grab_focus();
    }
}

fn add_section(parent: &gtk::Box, title: &str) -> gtk::Box {
    let section = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let heading = gtk::Label::builder().label(title).xalign(0.0).build();
    heading.add_css_class("heading");
    heading.set_margin_start(6);
    section.append(&heading);
    let rows = gtk::Box::new(gtk::Orientation::Vertical, 2);
    section.append(&rows);
    parent.append(&section);
    rows
}

fn filter_rows(list: &gtk::Box, query: &str) {
    let mut any_visible = false;
    let mut child = list.first_child();
    while let Some(row) = child {
        child = row.next_sibling();
        let visible = query.is_empty() || row.widget_name().to_lowercase().contains(query);
        row.set_visible(visible);
        any_visible |= visible;
    }
    if let Some(section) = list.parent() {
        section.set_visible(any_visible);
    }
}

fn connect_action(
    button: &gtk::Button,
    popover: &gtk::Popover,
    sender: &Sender,
    request: UiRequest,
) {
    let popover = popover.downgrade();
    let sender = sender.clone();
    button.connect_clicked(move |_| {
        if let Some(popover) = popover.upgrade() {
            popover.popdown();
        }
        sender(request.clone());
    });
}

fn branch_row(branch: &Branch, popover: &gtk::Popover, sender: &Sender) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    row.set_widget_name(&branch.name);
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let icon = gtk::Image::from_icon_name(if branch.is_head {
        "emblem-ok-symbolic"
    } else if branch.is_remote {
        "network-workgroup-symbolic"
    } else {
        "git-branch-symbolic"
    });
    let text = gtk::Box::new(gtk::Orientation::Vertical, 1);
    text.set_hexpand(true);
    let name = gtk::Label::builder()
        .label(&branch.name)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    if branch.is_head {
        name.add_css_class("heading");
    }
    text.append(&name);

    if let Some(upstream) = &branch.upstream {
        let tracking = if branch.gone {
            format!("{upstream} · upstream is gone")
        } else {
            let mut relation = upstream.to_string();
            if branch.ahead > 0 {
                relation.push_str(&format!(" ↑{}", branch.ahead));
            }
            if branch.behind > 0 {
                relation.push_str(&format!(" ↓{}", branch.behind));
            }
            relation
        };
        let label = gtk::Label::builder()
            .label(&tracking)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        label.add_css_class("dim-label");
        text.append(&label);
    }

    content.append(&icon);
    content.append(&text);
    let button = gtk::Button::builder()
        .child(&content)
        .has_frame(false)
        .tooltip_text(&branch.name)
        .hexpand(true)
        .build();
    if let Some(remote) = &branch.remote {
        let prefix = format!("{remote}/");
        let name = branch.name.strip_prefix(&prefix).unwrap_or(&branch.name);
        connect_action(
            &button,
            popover,
            sender,
            UiRequest::CheckoutRemoteBranch {
                remote: remote.clone(),
                branch: name.to_string(),
            },
        );
    } else {
        connect_action(
            &button,
            popover,
            sender,
            UiRequest::CheckoutBranch(branch.name.clone()),
        );
    }

    let actions = branch_popover(branch, sender.clone(), popover);
    let menu = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .tooltip_text(format!("Actions for {}", branch.name))
        .valign(gtk::Align::Center)
        .popover(&actions)
        .build();
    menu.add_css_class("flat");
    row.append(&button);
    row.append(&menu);
    row
}

fn branch_popover(branch: &Branch, sender: Sender, parent: &gtk::Popover) -> gtk::Popover {
    let popover = gtk::Popover::new();
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 2);
    box_.set_margin_start(6);
    box_.set_margin_end(6);
    box_.set_margin_top(6);
    box_.set_margin_bottom(6);
    let parent = parent.downgrade();
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
        let pop = popover.downgrade();
        let parent = parent.clone();
        button.connect_clicked(move |_| {
            if let Some(pop) = pop.upgrade() {
                pop.popdown();
            }
            if let Some(parent) = parent.upgrade() {
                parent.popdown();
            }
            sender(req.clone());
        });
        box_.append(&button);
    };

    if branch.is_remote {
        let remote = branch.remote.clone().unwrap_or_default();
        let short = branch
            .name
            .strip_prefix(&format!("{remote}/"))
            .unwrap_or(&branch.name)
            .to_string();
        add(
            "Checkout as Local Branch",
            UiRequest::CheckoutRemoteBranch {
                remote: remote.clone(),
                branch: short.clone(),
            },
            false,
        );
        add(
            "Merge into Current Branch",
            UiRequest::MergeBranch(branch.full_ref.clone()),
            false,
        );
        add(
            "Rebase Current onto This",
            UiRequest::RebaseOnto(branch.full_ref.clone()),
            false,
        );
        add(
            "Copy Branch Name",
            UiRequest::CopyText(branch.name.clone()),
            false,
        );
        add(
            "Delete Remote Branch…",
            UiRequest::DeleteRemoteBranch {
                remote,
                branch: short,
            },
            true,
        );
    } else {
        if !branch.is_head {
            add(
                "Checkout",
                UiRequest::CheckoutBranch(branch.name.clone()),
                false,
            );
        }
        add(
            "Merge into Current",
            UiRequest::MergeBranch(branch.full_ref.clone()),
            false,
        );
        add(
            "Rebase Current onto This",
            UiRequest::RebaseOnto(branch.full_ref.clone()),
            false,
        );
        add("Push…", UiRequest::PushBranch(branch.name.clone()), false);
        if !branch.is_head {
            add(
                "Set as Upstream of Current",
                UiRequest::SetUpstream(branch.full_ref.clone()),
                false,
            );
        }
        add(
            "Compare with Current",
            UiRequest::CompareWithBranch(branch.full_ref.clone()),
            false,
        );
        add(
            "Rename…",
            UiRequest::RenameBranch(branch.name.clone()),
            false,
        );
        add(
            "Copy Branch Name",
            UiRequest::CopyText(branch.name.clone()),
            false,
        );
        if !branch.is_head {
            add(
                "Delete Branch…",
                UiRequest::DeleteBranch(branch.name.clone()),
                true,
            );
        }
    }
    popover.set_child(Some(&box_));
    popover
}
