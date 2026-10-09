//! Branch management: locals, remotes, checkout, merge, rebase, delete...

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

use crate::git::Branch;

use super::repo_page::UiRequest;
use super::util;

type Sender = Rc<dyn Fn(UiRequest)>;

pub struct BranchesView {
    root: gtk::Box,
    local_list: gtk::ListBox,
    remote_list: gtk::ListBox,
    local_count: gtk::Label,
    remote_count: gtk::Label,
    filter: gtk::SearchEntry,
    request: RefCell<Option<Sender>>,
}

impl BranchesView {
    pub fn new() -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);

        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        toolbar.set_margin_top(6);
        toolbar.set_margin_bottom(6);
        toolbar.set_margin_start(8);
        toolbar.set_margin_end(8);
        let filter = gtk::SearchEntry::builder()
            .placeholder_text("Filter branches…")
            .hexpand(true)
            .build();
        let create = gtk::Button::builder()
            .label("New Branch")
            .icon_name("list-add-symbolic")
            .build();
        let compare = util::icon_button("git-compare-symbolic", "Compare two branches");
        toolbar.append(&filter);
        toolbar.append(&compare);
        toolbar.append(&create);

        let (local_header, local_count) = util::list_header("Local Branches");
        let (remote_header, remote_count) = util::list_header("Remote Branches");
        local_header.set_margin_top(6);
        remote_header.set_margin_top(12);
        let local_list = list_box();
        let remote_list = list_box();

        let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
        content.set_margin_start(8);
        content.set_margin_end(8);
        content.set_margin_bottom(8);
        content.append(&local_header);
        content.append(&local_list);
        content.append(&remote_header);
        content.append(&remote_list);
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
                        v.send(UiRequest::CreateBranch);
                    }
                });
            }
            {
                let w = weak.clone();
                compare.connect_clicked(move |_| {
                    if let Some(v) = w.upgrade() {
                        v.send(UiRequest::CompareBranches);
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
                local_list: local_list.clone(),
                remote_list: remote_list.clone(),
                local_count: local_count.clone(),
                remote_count: remote_count.clone(),
                filter: filter.clone(),
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

    pub fn render(&self, branches: &[Branch]) {
        let sender = match self.request.borrow().clone() {
            Some(s) => s,
            None => return,
        };
        for list in [&self.local_list, &self.remote_list] {
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
        }
        let mut locals = 0usize;
        let mut remotes = 0usize;
        for branch in branches {
            if branch.is_remote {
                if branch.name.ends_with("/HEAD") {
                    continue;
                }
                self.remote_list.append(&build_branch_row(branch, &sender));
                remotes += 1;
            } else {
                self.local_list.append(&build_branch_row(branch, &sender));
                locals += 1;
            }
        }
        self.local_count.set_label(&locals.to_string());
        self.remote_count.set_label(&remotes.to_string());
        self.apply_filter();
    }

    fn apply_filter(&self) {
        let query = self.filter.text().to_lowercase();
        for list in [&self.local_list, &self.remote_list] {
            let mut child = list.first_child();
            while let Some(row) = child {
                child = row.next_sibling();
                let matches = if query.is_empty() {
                    true
                } else {
                    row.downcast_ref::<gtk::ListBoxRow>()
                        .and_then(|r| r.tooltip_text())
                        .map(|t| t.to_lowercase().contains(&query))
                        .unwrap_or(true)
                };
                row.set_visible(matches);
            }
        }
    }
}

fn list_box() -> gtk::ListBox {
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    list
}

fn build_branch_row(branch: &Branch, sender: &Sender) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    let widget = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    widget.set_margin_top(4);
    widget.set_margin_bottom(4);
    widget.set_margin_start(6);
    widget.set_margin_end(6);

    let icon = gtk::Image::from_icon_name(if branch.is_remote {
        "network-workgroup-symbolic"
    } else if branch.is_head {
        "emblem-ok-symbolic"
    } else {
        "git-branch-symbolic"
    });
    icon.set_pixel_size(14);
    if branch.is_head {
        icon.add_css_class("accent");
    }

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
    let subtitle_text = if branch.gone {
        "upstream is gone".to_string()
    } else if branch.tip_summary.is_empty() {
        String::new()
    } else {
        branch.tip_summary.clone()
    };
    let subtitle = gtk::Label::builder()
        .label(&subtitle_text)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    subtitle.add_css_class("dim-label");
    text.append(&name);
    if !subtitle_text.is_empty() {
        text.append(&subtitle);
    }

    let tracking = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    if let Some(upstream) = &branch.upstream {
        let up = gtk::Label::new(Some(upstream));
        up.add_css_class("dim-label");
        tracking.append(&up);
    }
    if branch.ahead > 0 {
        let ahead = gtk::Label::new(Some(&format!("↑{}", branch.ahead)));
        ahead.add_css_class("success");
        tracking.append(&ahead);
    }
    if branch.behind > 0 {
        let behind = gtk::Label::new(Some(&format!("↓{}", branch.behind)));
        behind.add_css_class("warning");
        tracking.append(&behind);
    }

    let menu = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .valign(gtk::Align::Center)
        .build();
    menu.add_css_class("flat");
    menu.set_popover(Some(&branch_popover(branch, sender.clone())));

    widget.append(&icon);
    widget.append(&text);
    widget.append(&tracking);
    widget.append(&menu);
    row.set_child(Some(&widget));
    row.set_tooltip_text(Some(&branch.name));

    // Double click to checkout
    let gesture = gtk::GestureClick::new();
    gesture.set_button(1);
    let sender = sender.clone();
    let name = branch.name.clone();
    let is_remote = branch.is_remote;
    let remote = branch.remote.clone();
    gesture.connect_pressed(move |_, n_press, _, _| {
        if n_press == 2 {
            if is_remote {
                if let Some(remote) = &remote {
                    let short = name
                        .strip_prefix(&format!("{remote}/"))
                        .unwrap_or(&name)
                        .to_string();
                    sender(UiRequest::CheckoutRemoteBranch {
                        remote: remote.clone(),
                        branch: short,
                    });
                }
            } else {
                sender(UiRequest::CheckoutBranch(name.clone()));
            }
        }
    });
    widget.add_controller(gesture);

    row
}

fn branch_popover(branch: &Branch, sender: Sender) -> gtk::Popover {
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
            UiRequest::MergeBranch(branch.name.clone()),
            false,
        );
        add(
            "Rebase Current onto This",
            UiRequest::RebaseOnto(branch.name.clone()),
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
        add("Merge into Current", UiRequest::MergeBranch(branch.name.clone()), false);
        add(
            "Rebase Current onto This",
            UiRequest::RebaseOnto(branch.name.clone()),
            false,
        );
        add("Push…", UiRequest::PushBranch(branch.name.clone()), false);
        if !branch.is_head {
            add(
                "Set as Upstream of Current",
                UiRequest::SetUpstream(branch.name.clone()),
                false,
            );
        }
        add(
            "Compare with Current",
            UiRequest::CompareWithBranch(branch.name.clone()),
            false,
        );
        add("Rename…", UiRequest::RenameBranch(branch.name.clone()), false);
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
