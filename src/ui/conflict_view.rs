//! Conflict resolution UI.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;

use crate::git::{ConflictChoice, ConflictRegion, OperationState};

use super::repo_page::UiRequest;
use super::util;

type Sender = Rc<dyn Fn(UiRequest)>;

pub struct ConflictView {
    root: gtk::Box,
    file_list: gtk::ListBox,
    regions_box: gtk::Box,
    banner: gtk::Box,
    banner_label: gtk::Label,
    path_label: gtk::Label,
    skip_button: gtk::Button,
    toolbar: gtk::Box,
    placeholder: gtk::Stack,
    request: RefCell<Option<Sender>>,
    current_path: RefCell<Option<PathBuf>>,
    conflict_paths: RefCell<Vec<PathBuf>>,
}

impl ConflictView {
    pub fn new() -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);

        // Banner for an in-progress operation
        let banner = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        banner.add_css_class("op-banner");
        banner.set_margin_top(6);
        banner.set_margin_bottom(6);
        banner.set_margin_start(8);
        banner.set_margin_end(8);
        let banner_icon = gtk::Image::from_icon_name("dialog-warning-symbolic");
        banner_icon.set_pixel_size(16);
        let banner_label = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .wrap(true)
            .build();
        let continue_button = gtk::Button::with_label("Continue");
        continue_button.add_css_class("suggested-action");
        let skip_button = gtk::Button::with_label("Skip");
        let abort_button = gtk::Button::with_label("Abort");
        abort_button.add_css_class("destructive-action");
        banner.append(&banner_icon);
        banner.append(&banner_label);
        banner.append(&continue_button);
        banner.append(&skip_button);
        banner.append(&abort_button);
        banner.set_visible(false);

        let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
        paned.set_wide_handle(true);
        paned.set_position(300);

        // Left: conflicted files
        let left = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let (list_header, _) = util::list_header("Conflicted Files");
        list_header.set_margin_start(8);
        list_header.set_margin_end(8);
        let file_list = gtk::ListBox::new();
        file_list.add_css_class("boxed-list");
        file_list.set_selection_mode(gtk::SelectionMode::Single);
        file_list.set_margin_start(8);
        file_list.set_margin_end(8);
        file_list.set_margin_bottom(8);
        let left_scroll = gtk::ScrolledWindow::builder()
            .child(&file_list)
            .vexpand(true)
            .build();
        left.append(&list_header);
        left.append(&left_scroll);
        paned.set_start_child(Some(&left));

        // Right: regions
        let right = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        toolbar.set_margin_top(6);
        toolbar.set_margin_bottom(6);
        toolbar.set_margin_start(8);
        toolbar.set_margin_end(8);
        let path_label = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .build();
        path_label.add_css_class("heading");
        let accept_all_ours = gtk::Button::with_label("All Current");
        let accept_all_theirs = gtk::Button::with_label("All Incoming");
        let mark_resolved = gtk::Button::with_label("Mark Resolved");
        mark_resolved.add_css_class("suggested-action");
        let open_editor = util::icon_button("document-open-symbolic", "Open file in external editor");
        toolbar.append(&path_label);
        toolbar.append(&accept_all_ours);
        toolbar.append(&accept_all_theirs);
        toolbar.append(&mark_resolved);
        toolbar.append(&open_editor);

        let regions_box = gtk::Box::new(gtk::Orientation::Vertical, 10);
        regions_box.set_margin_start(8);
        regions_box.set_margin_end(8);
        regions_box.set_margin_bottom(12);
        let regions_scroll = gtk::ScrolledWindow::builder()
            .child(&regions_box)
            .vexpand(true)
            .build();

        let placeholder_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        placeholder_box.set_valign(gtk::Align::Center);
        placeholder_box.set_halign(gtk::Align::Center);
        let ph_icon = gtk::Image::from_icon_name("emblem-ok-symbolic");
        ph_icon.set_pixel_size(48);
        ph_icon.add_css_class("success");
        let ph_title = gtk::Label::new(Some("No conflicts"));
        ph_title.add_css_class("heading");
        let ph_sub = gtk::Label::new(Some("The working tree has no unresolved conflicts."));
        ph_sub.add_css_class("dim-label");
        placeholder_box.append(&ph_icon);
        placeholder_box.append(&ph_title);
        placeholder_box.append(&ph_sub);

        let placeholder = gtk::Stack::builder().vexpand(true).build();
        placeholder.add_named(&placeholder_box, Some("empty"));
        let resolver = gtk::Box::new(gtk::Orientation::Vertical, 0);
        resolver.append(&toolbar);
        resolver.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        resolver.append(&regions_scroll);
        placeholder.add_named(&resolver, Some("resolver"));

        right.append(&placeholder);
        paned.set_end_child(Some(&right));
        paned.set_resize_start_child(false);
        paned.set_resize_end_child(true);

        root.append(&banner);
        root.append(&paned);

        let view = Rc::new_cyclic(|weak: &std::rc::Weak<Self>| {
            {
                let w = weak.clone();
                continue_button.connect_clicked(move |_| {
                    if let Some(v) = w.upgrade() {
                        v.send(UiRequest::ContinueOperation);
                    }
                });
            }
            {
                let w = weak.clone();
                skip_button.connect_clicked(move |_| {
                    if let Some(v) = w.upgrade() {
                        v.send(UiRequest::SkipOperation);
                    }
                });
            }
            {
                let w = weak.clone();
                abort_button.connect_clicked(move |_| {
                    if let Some(v) = w.upgrade() {
                        v.send(UiRequest::AbortOperation);
                    }
                });
            }
            {
                let w = weak.clone();
                accept_all_ours.connect_clicked(move |_| {
                    if let Some(v) = w.upgrade() {
                        if let Some(path) = v.current_path.borrow().clone() {
                            v.send(UiRequest::AcceptAllConflicts {
                                path,
                                choice: ConflictChoice::Ours,
                            });
                        }
                    }
                });
            }
            {
                let w = weak.clone();
                accept_all_theirs.connect_clicked(move |_| {
                    if let Some(v) = w.upgrade() {
                        if let Some(path) = v.current_path.borrow().clone() {
                            v.send(UiRequest::AcceptAllConflicts {
                                path,
                                choice: ConflictChoice::Theirs,
                            });
                        }
                    }
                });
            }
            {
                let w = weak.clone();
                mark_resolved.connect_clicked(move |_| {
                    if let Some(v) = w.upgrade() {
                        if let Some(path) = v.current_path.borrow().clone() {
                            v.send(UiRequest::MarkResolved(path));
                        }
                    }
                });
            }
            {
                let w = weak.clone();
                open_editor.connect_clicked(move |_| {
                    if let Some(v) = w.upgrade() {
                        if let Some(path) = v.current_path.borrow().clone() {
                            v.send(UiRequest::OpenFile(path));
                        }
                    }
                });
            }
            let w = weak.clone();
            file_list.connect_row_selected(move |_, row| {
                let Some(row) = row else { return };
                let Some(v) = w.upgrade() else { return };
                let index = row.index() as usize;
                let paths = v.conflict_paths.borrow();
                let Some(path) = paths.get(index).cloned() else {
                    return;
                };
                drop(paths);
                v.send(UiRequest::SelectConflict(path));
            });
            Self {
                root: root.clone(),
                file_list: file_list.clone(),
                regions_box: regions_box.clone(),
                banner: banner.clone(),
                banner_label: banner_label.clone(),
                path_label: path_label.clone(),
                skip_button: skip_button.clone(),
                toolbar: toolbar.clone(),
                placeholder: placeholder.clone(),
                request: RefCell::new(None),
                current_path: RefCell::new(None),
                conflict_paths: RefCell::new(Vec::new()),
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

    pub fn set_operation(&self, operation: &OperationState) {
        if operation.is_none() {
            self.banner.set_visible(false);
            return;
        }
        self.banner.set_visible(true);
        self.banner_label.set_label(&format!(
            "{} — resolve all conflicts, then continue. Aborting restores the previous state.",
            util::operation_label(operation)
        ));
        let can_skip = matches!(operation, OperationState::Rebase { .. } | OperationState::CherryPick { .. } | OperationState::Revert { .. });
        self.skip_button.set_visible(can_skip);
    }

    pub fn render_list(&self, paths: &[PathBuf]) {
        while let Some(child) = self.file_list.first_child() {
            self.file_list.remove(&child);
        }
        *self.conflict_paths.borrow_mut() = paths.to_vec();
        for path in paths {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            row.set_margin_top(4);
            row.set_margin_bottom(4);
            row.set_margin_start(6);
            row.set_margin_end(6);
            let icon = gtk::Image::from_icon_name("dialog-warning-symbolic");
            icon.set_pixel_size(14);
            icon.add_css_class("error");
            let label = util::path_label(&path.display().to_string());
            row.append(&icon);
            row.append(&label);
            self.file_list.append(&row);
        }
        if paths.is_empty() {
            self.placeholder.set_visible_child_name("empty");
            self.toolbar.set_sensitive(false);
        } else {
            self.toolbar.set_sensitive(true);
            if self.file_list.row_at_index(0).is_some() {
                self.file_list.select_row(self.file_list.row_at_index(0).as_ref());
            }
        }
    }

    /// Show the conflict regions of one file.
    pub fn show_file(&self, path: &std::path::Path, regions: &[ConflictRegion]) {
        *self.current_path.borrow_mut() = Some(path.to_path_buf());
        self.path_label.set_label(&path.display().to_string());
        while let Some(child) = self.regions_box.first_child() {
            self.regions_box.remove(&child);
        }
        self.placeholder.set_visible_child_name("resolver");

        if regions.is_empty() {
            let notice = gtk::Label::builder()
                .label(
                    "This file has no textual conflict markers (binary file or add/delete conflict).\nUse \"Keep Current\" or \"Keep Incoming\" from the buttons above the list, or edit the file manually and mark it resolved.",
                )
                .xalign(0.0)
                .wrap(true)
                .build();
            notice.add_css_class("dim-label");
            self.regions_box.append(&notice);
            // Provide side-choosing buttons for this file.
            let side_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let keep_current = gtk::Button::with_label("Keep Current Version");
            let keep_incoming = gtk::Button::with_label("Keep Incoming Version");
            let sender = self.request.borrow().clone();
            if let Some(sender) = sender {
                let p = path.to_path_buf();
                let s = sender.clone();
                keep_current.connect_clicked(move |_| {
                    s(UiRequest::ResolveConflictSide {
                        path: p.clone(),
                        ours: true,
                    });
                });
                let p = path.to_path_buf();
                keep_incoming.connect_clicked(move |_| {
                    sender(UiRequest::ResolveConflictSide {
                        path: p.clone(),
                        ours: false,
                    });
                });
            }
            side_box.append(&keep_current);
            side_box.append(&keep_incoming);
            self.regions_box.append(&side_box);
            return;
        }

        for (index, region) in regions.iter().enumerate() {
            self.regions_box
                .append(&region_widget(path, index, region, regions.len(), self.request.borrow().clone()));
        }
    }

    pub fn current_path(&self) -> Option<PathBuf> {
        self.current_path.borrow().clone()
    }
}

fn region_widget(
    path: &std::path::Path,
    index: usize,
    region: &ConflictRegion,
    total: usize,
    sender: Option<Sender>,
) -> gtk::Widget {
    let frame = gtk::Box::new(gtk::Orientation::Vertical, 6);
    frame.add_css_class("card");
    frame.set_margin_top(2);

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    header.set_margin_top(8);
    header.set_margin_start(10);
    header.set_margin_end(10);
    let title = gtk::Label::builder()
        .label(format!("Conflict {} of {}", index + 1, total))
        .xalign(0.0)
        .hexpand(true)
        .build();
    title.add_css_class("heading");
    let accept_ours = gtk::Button::with_label("Accept Current");
    let accept_theirs = gtk::Button::with_label("Accept Incoming");
    let accept_both = gtk::Button::with_label("Accept Both");
    accept_ours.add_css_class("flat");
    accept_theirs.add_css_class("flat");
    accept_both.add_css_class("flat");
    header.append(&title);
    header.append(&accept_ours);
    header.append(&accept_theirs);
    header.append(&accept_both);

    let columns = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    columns.set_margin_start(10);
    columns.set_margin_end(10);
    columns.set_margin_bottom(10);
    columns.set_homogeneous(true);
    columns.append(&side_box(
        &format!("Current — {}", display_label(&region.ours_label)),
        &region.ours,
        "error",
    ));
    columns.append(&side_box(
        &format!("Incoming — {}", display_label(&region.theirs_label)),
        &region.theirs,
        "success",
    ));

    frame.append(&header);
    frame.append(&columns);

    let wire = |button: &gtk::Button, choice: ConflictChoice| {
        let sender = sender.clone();
        let path = path.to_path_buf();
        button.connect_clicked(move |_| {
            if let Some(s) = sender.as_ref() {
                s(UiRequest::AcceptConflict {
                    path: path.clone(),
                    region: index,
                    choice,
                });
            }
        });
    };
    wire(&accept_ours, ConflictChoice::Ours);
    wire(&accept_theirs, ConflictChoice::Theirs);
    wire(&accept_both, ConflictChoice::Both);

    frame.upcast()
}

fn side_box(title: &str, lines: &[String], css: &str) -> gtk::Box {
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let label = gtk::Label::builder()
        .label(title)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    label.add_css_class("dim-label");
    label.add_css_class("heading");
    let content = gtk::Label::builder()
        .label(if lines.is_empty() {
            "(no lines)".to_string()
        } else {
            lines.join("\n")
        })
        .xalign(0.0)
        .yalign(0.0)
        .wrap(true)
        .selectable(true)
        .build();
    content.add_css_class("monospace");
    content.add_css_class(css);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&content)
        .min_content_height(80)
        .max_content_height(220)
        .has_frame(true)
        .build();
    box_.append(&label);
    box_.append(&scroll);
    box_
}

fn display_label(label: &str) -> String {
    if label.is_empty() {
        "?".to_string()
    } else {
        label.to_string()
    }
}
