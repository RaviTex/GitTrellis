//! Working tree changes: conflicts, staged, unstaged and untracked files,
//! with a diff pane and the commit panel.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;

use crate::git::{ChangeKind, FileDiff, RepoStatus, StatusEntry};

use super::commit_panel::CommitPanel;
use super::diff_view::{DiffAction, DiffMode, DiffView};
use super::repo_page::UiRequest;
use super::util;

pub struct FileRow {
    pub widget: gtk::Box,
    pub path: PathBuf,
    pub orig_path: Option<PathBuf>,
    pub staged: bool,
    pub untracked: bool,
    pub ignored: bool,
    pub conflicted: bool,
    pub has_unstaged: bool,
}

type Sender = Rc<dyn Fn(UiRequest)>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum RowKind {
    Staged,
    Unstaged,
    Untracked,
    Conflict,
    Ignored,
}

pub struct ChangesView {
    root: gtk::Paned,
    conflicts_section: gtk::Box,
    conflicts_list: gtk::ListBox,
    conflicts_count: gtk::Label,
    staged_section: gtk::Box,
    staged_list: gtk::ListBox,
    staged_count: gtk::Label,
    unstaged_section: gtk::Box,
    unstaged_list: gtk::ListBox,
    unstaged_count: gtk::Label,
    untracked_section: gtk::Box,
    untracked_list: gtk::ListBox,
    untracked_count: gtk::Label,
    ignored_section: gtk::Box,
    ignored_list: gtk::ListBox,
    stage_selected_button: gtk::Button,
    unstage_selected_button: gtk::Button,
    diff_view: Rc<DiffView>,
    commit_panel: Rc<CommitPanel>,
    request: Rc<RefCell<Option<Sender>>>,
    selected_path: RefCell<Option<PathBuf>>,
    show_ignored: RefCell<bool>,
    rows: RefCell<Vec<Rc<FileRow>>>,
    rendering: std::cell::Cell<bool>,
}

impl ChangesView {
    pub fn new() -> Rc<Self> {
        let root = gtk::Paned::new(gtk::Orientation::Horizontal);
        root.set_wide_handle(true);
        root.set_resize_start_child(true);
        root.set_resize_end_child(true);

        let left = gtk::Box::new(gtk::Orientation::Vertical, 0);
        left.set_size_request(360, -1);

        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        toolbar.set_margin_top(4);
        toolbar.set_margin_bottom(4);
        toolbar.set_margin_start(6);
        toolbar.set_margin_end(6);
        let stage_selected_button =
            util::icon_button("list-add-symbolic", "Stage selected files (Ctrl+S)");
        let unstage_selected_button =
            util::icon_button("list-remove-symbolic", "Unstage selected files");
        let stage_all_button =
            util::icon_button("go-up-symbolic", "Stage all changes (Ctrl+Shift+S)");
        let unstage_all_button = util::icon_button("go-down-symbolic", "Unstage all changes");
        let refresh_button = util::icon_button("view-refresh-symbolic", "Refresh (Ctrl+R)");
        let overflow = util::menu_button("view-more-symbolic", "More actions");

        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        toolbar.append(&stage_selected_button);
        toolbar.append(&unstage_selected_button);
        toolbar.append(&spacer);
        toolbar.append(&stage_all_button);
        toolbar.append(&unstage_all_button);
        toolbar.append(&refresh_button);
        toolbar.append(&overflow);

        let lists_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        lists_box.set_margin_start(6);
        lists_box.set_margin_end(6);
        lists_box.set_margin_bottom(6);

        let (conflicts_section, conflicts_list, conflicts_count) = section("Conflicts");
        let (staged_section, staged_list, staged_count) = section("Staged Changes");
        let (unstaged_section, unstaged_list, unstaged_count) = section("Changes");
        let (untracked_section, untracked_list, untracked_count) = section("Untracked Files");
        let (ignored_section, ignored_list, _) = section("Ignored Files");
        ignored_section.set_visible(false);

        lists_box.append(&conflicts_section);
        lists_box.append(&staged_section);
        lists_box.append(&unstaged_section);
        lists_box.append(&untracked_section);
        lists_box.append(&ignored_section);

        let lists_scroll = gtk::ScrolledWindow::builder()
            .child(&lists_box)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();

        let commit_panel = CommitPanel::new();

        left.append(&toolbar);
        left.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        left.append(&lists_scroll);
        left.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        left.append(commit_panel.widget());

        let right = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let diff_view = DiffView::new();
        right.append(diff_view.widget());

        root.set_start_child(Some(&left));
        root.set_end_child(Some(&right));
        root.set_position(420);

        let list_refs: [(&gtk::ListBox, &str); 5] = [
            (&conflicts_list, "conflicts"),
            (&staged_list, "staged"),
            (&unstaged_list, "unstaged"),
            (&untracked_list, "untracked"),
            (&ignored_list, "ignored"),
        ];
        for (list, _) in list_refs {
            list.set_selection_mode(gtk::SelectionMode::Multiple);
            list.set_activate_on_single_click(true);
        }

        let view = Rc::new_cyclic(|weak: &std::rc::Weak<Self>| {
            {
                let w = weak.clone();
                stage_selected_button.connect_clicked(move |_| {
                    if let Some(v) = w.upgrade() {
                        v.stage_selected();
                    }
                });
            }
            {
                let w = weak.clone();
                unstage_selected_button.connect_clicked(move |_| {
                    if let Some(v) = w.upgrade() {
                        v.unstage_selected();
                    }
                });
            }
            {
                let w = weak.clone();
                stage_all_button.connect_clicked(move |_| {
                    if let Some(v) = w.upgrade() {
                        v.send(UiRequest::StageAll);
                    }
                });
            }
            {
                let w = weak.clone();
                unstage_all_button.connect_clicked(move |_| {
                    if let Some(v) = w.upgrade() {
                        v.send(UiRequest::UnstageAll);
                    }
                });
            }
            {
                let w = weak.clone();
                refresh_button.connect_clicked(move |_| {
                    if let Some(v) = w.upgrade() {
                        v.send(UiRequest::RefreshAll);
                    }
                });
            }
            let shared: Rc<RefCell<Option<Sender>>> = Rc::new(RefCell::new(None));
            overflow.set_popover(Some(&overflow_popover(shared.clone())));

            // Selection -> preview the diff
            for list in [
                &conflicts_list,
                &staged_list,
                &unstaged_list,
                &untracked_list,
                &ignored_list,
            ] {
                let w = weak.clone();
                list.connect_row_selected(move |list, row| {
                    let Some(v) = w.upgrade() else { return };
                    if v.rendering.get() {
                        return;
                    }
                    match row {
                        Some(row) => {
                            let Some(child) = row.child() else { return };
                            let rows = v.rows.borrow();
                            let found = rows.iter().find(|r| r.widget.eq(&child)).cloned();
                            drop(rows);
                            if let Some(found) = found {
                                v.selected_path.replace(Some(found.path.clone()));
                                v.send(UiRequest::SelectDiff {
                                    path: found.path.clone(),
                                    staged: found.staged,
                                    rev: None,
                                });
                            }
                        }
                        None => {
                            // A row was deselected. When nothing is selected
                            // anymore the preview is cleared as well, so a
                            // second click really deselects.
                            if !v.any_selection() {
                                v.selected_path.borrow_mut().take();
                                v.diff_view.clear();
                                v.diff_view.show_placeholder(
                                    "Select a file",
                                    "Its diff will appear here.",
                                );
                            } else {
                                let _ = list;
                            }
                        }
                    }
                });
            }

            Self {
                root: root.clone(),
                conflicts_section: conflicts_section.clone(),
                conflicts_list: conflicts_list.clone(),
                conflicts_count: conflicts_count.clone(),
                staged_section: staged_section.clone(),
                staged_list: staged_list.clone(),
                staged_count: staged_count.clone(),
                unstaged_section: unstaged_section.clone(),
                unstaged_list: unstaged_list.clone(),
                unstaged_count: unstaged_count.clone(),
                untracked_section: untracked_section.clone(),
                untracked_list: untracked_list.clone(),
                untracked_count: untracked_count.clone(),
                ignored_section: ignored_section.clone(),
                ignored_list: ignored_list.clone(),
                stage_selected_button: stage_selected_button.clone(),
                unstage_selected_button: unstage_selected_button.clone(),
                diff_view: diff_view.clone(),
                commit_panel: commit_panel.clone(),
                request: shared.clone(),
                selected_path: RefCell::new(None),
                show_ignored: RefCell::new(false),
                rows: RefCell::new(Vec::new()),
                rendering: std::cell::Cell::new(false),
            }
        });

        view.diff_view.set_handler({
            let w = Rc::downgrade(&view);
            move |action, hunk| {
                if let Some(v) = w.upgrade() {
                    v.on_diff_action(action, hunk);
                }
            }
        });

        view
    }

    pub fn widget(&self) -> &gtk::Paned {
        &self.root
    }

    pub fn diff_view(&self) -> &Rc<DiffView> {
        &self.diff_view
    }

    pub fn commit_panel(&self) -> &Rc<CommitPanel> {
        &self.commit_panel
    }

    pub fn connect_request(&self, request: Sender) {
        *self.request.borrow_mut() = Some(request.clone());
        self.commit_panel.connect_request(request);
    }

    fn sender(&self) -> Option<Sender> {
        self.request.borrow().clone()
    }

    fn send(&self, req: UiRequest) {
        if let Some(r) = self.request.borrow().as_ref() {
            r(req);
        }
    }

    pub fn set_show_ignored(&self, show: bool) {
        *self.show_ignored.borrow_mut() = show;
    }

    pub fn set_diff_mode(&self, mode: DiffMode) {
        self.diff_view.set_mode(mode);
    }

    pub fn set_file_selection_mode(&self, multiple: bool) {
        let mode = if multiple {
            gtk::SelectionMode::Multiple
        } else {
            gtk::SelectionMode::Single
        };
        for list in [
            &self.conflicts_list,
            &self.staged_list,
            &self.unstaged_list,
            &self.untracked_list,
            &self.ignored_list,
        ] {
            list.set_selection_mode(mode);
        }
    }

    // ------------------------------------------------------------------
    // Rendering
    // ------------------------------------------------------------------

    pub fn render(&self, status: &RepoStatus) {
        let sender = match self.sender() {
            Some(s) => s,
            None => return,
        };
        // Preserve the preview and any multi-selection across refreshes.
        let previous = self.selected_path.borrow().clone();
        let selected_paths: Vec<PathBuf> = self
            .selected_rows()
            .iter()
            .map(|r| r.path.clone())
            .collect();
        self.rendering.set(true);

        for list in [
            &self.conflicts_list,
            &self.staged_list,
            &self.unstaged_list,
            &self.untracked_list,
            &self.ignored_list,
        ] {
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
        }
        self.rows.borrow_mut().clear();

        let mut staged = 0usize;
        let mut unstaged = 0usize;
        let mut untracked = 0usize;
        let mut conflicted = 0usize;
        let mut ignored = 0usize;

        for entry in &status.entries {
            match entry.index {
                ChangeKind::Ignored => {
                    if *self.show_ignored.borrow() {
                        self.add_row(&self.ignored_list, entry, RowKind::Ignored, &sender);
                        ignored += 1;
                    }
                }
                ChangeKind::Untracked => {
                    self.add_row(&self.untracked_list, entry, RowKind::Untracked, &sender);
                    untracked += 1;
                }
                ChangeKind::Unmerged => {
                    self.add_row(&self.conflicts_list, entry, RowKind::Conflict, &sender);
                    conflicted += 1;
                }
                _ => {
                    if entry.is_staged() {
                        self.add_row(&self.staged_list, entry, RowKind::Staged, &sender);
                        staged += 1;
                    }
                    if entry.is_unstaged() {
                        self.add_row(&self.unstaged_list, entry, RowKind::Unstaged, &sender);
                        unstaged += 1;
                    }
                }
            }
        }

        self.conflicts_section.set_visible(conflicted > 0);
        self.conflicts_count.set_label(&conflicted.to_string());
        self.staged_section.set_visible(staged > 0);
        self.staged_count.set_label(&staged.to_string());
        self.unstaged_section.set_visible(unstaged > 0);
        self.unstaged_count.set_label(&unstaged.to_string());
        self.untracked_section.set_visible(untracked > 0);
        self.untracked_count.set_label(&untracked.to_string());
        self.ignored_section.set_visible(ignored > 0);

        self.commit_panel.update_state(staged);
        self.stage_selected_button
            .set_sensitive(unstaged + untracked + conflicted > 0);
        self.unstage_selected_button.set_sensitive(staged > 0);

        // Restore row selections for paths that still exist.
        self.restore_selection(&selected_paths);
        self.rendering.set(false);

        if status.is_clean() {
            self.diff_view.clear();
            self.diff_view.show_placeholder(
                "Working tree clean",
                "There are no changes to show. Edit a file and refresh, or open the History tab.",
            );
        } else if let Some(path) = previous {
            if !self.select_path(&path) {
                self.selected_path.borrow_mut().take();
                self.diff_view.clear();
                self.diff_view
                    .show_placeholder("Select a file", "Its diff will appear here.");
            }
        }
    }

    fn add_row(&self, list: &gtk::ListBox, entry: &StatusEntry, kind: RowKind, sender: &Sender) {
        let row = Rc::new(build_row(entry, kind, sender.clone()));
        list.append(&row.widget);
        self.rows.borrow_mut().push(row);
    }

    fn any_selection(&self) -> bool {
        for list in [
            &self.conflicts_list,
            &self.staged_list,
            &self.unstaged_list,
            &self.untracked_list,
            &self.ignored_list,
        ] {
            if list.selected_row().is_some() {
                return true;
            }
        }
        false
    }

    pub fn selected_rows(&self) -> Vec<Rc<FileRow>> {
        let mut out = Vec::new();
        let rows = self.rows.borrow();
        for list in [
            &self.conflicts_list,
            &self.staged_list,
            &self.unstaged_list,
            &self.untracked_list,
            &self.ignored_list,
        ] {
            for selected in list.selected_rows() {
                if let Some(child) = selected.child() {
                    if let Some(found) = rows.iter().find(|r| r.widget.eq(&child)) {
                        out.push(found.clone());
                    }
                }
            }
        }
        out
    }

    fn stage_selected(&self) {
        let rows = self.selected_rows();
        let paths: Vec<PathBuf> = rows
            .iter()
            .filter(|r| !r.conflicted && !r.ignored)
            .map(|r| r.path.clone())
            .collect();
        if !paths.is_empty() {
            self.send(UiRequest::StagePaths(paths));
        }
    }

    fn unstage_selected(&self) {
        let rows = self.selected_rows();
        let paths: Vec<PathBuf> = rows
            .iter()
            .filter(|r| r.staged)
            .map(|r| r.path.clone())
            .collect();
        if !paths.is_empty() {
            self.send(UiRequest::UnstagePaths(paths));
        }
    }

    fn on_diff_action(&self, action: DiffAction, hunk: Option<usize>) {
        let Some(file) = self.diff_view.current_file() else {
            return;
        };
        let path = file.path().cloned().unwrap_or_default();
        match action {
            DiffAction::Stage => match hunk {
                Some(h) => self.send(UiRequest::StageHunk { file, hunk: h }),
                None => self.send(UiRequest::StagePaths(vec![path])),
            },
            DiffAction::Unstage => match hunk {
                Some(h) => self.send(UiRequest::UnstageHunk { file, hunk: h }),
                None => self.send(UiRequest::UnstagePaths(vec![path])),
            },
            DiffAction::Discard => match hunk {
                Some(h) => self.send(UiRequest::DiscardHunk { file, hunk: h }),
                None => self.send(UiRequest::DiscardPaths(vec![path])),
            },
        }
    }

    /// Show a diff in the pane.
    pub fn show_diff(&self, file: &FileDiff, allow_actions: bool, allow_discard: bool) {
        self.diff_view.show_file(file);
        self.diff_view.set_allow_actions(allow_actions);
        self.diff_view.set_discard_enabled(allow_discard);
    }

    /// Re-select rows for the given paths without triggering previews.
    fn restore_selection(&self, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }
        let rows = self.rows.borrow();
        for list in [
            &self.conflicts_list,
            &self.staged_list,
            &self.unstaged_list,
            &self.untracked_list,
            &self.ignored_list,
        ] {
            let children: Vec<glib::Object> =
                list.observe_children().into_iter().flatten().collect();
            for child in children {
                if let Ok(row) = child.downcast::<gtk::ListBoxRow>() {
                    let Some(row_child) = row.child() else { continue };
                    if let Some(found) = rows.iter().find(|r| r.widget.eq(&row_child)) {
                        if paths.contains(&found.path) {
                            list.select_row(Some(&row));
                        }
                    }
                }
            }
        }
    }

    /// Select the row matching `path`. Returns false when it is not present.
    pub fn select_path(&self, path: &Path) -> bool {
        let rows = self.rows.borrow();
        let target = rows.iter().find(|r| r.path == path).cloned();
        drop(rows);
        let Some(target) = target else { return false };
        self.selected_path.replace(Some(target.path.clone()));
        for list in [
            &self.conflicts_list,
            &self.staged_list,
            &self.unstaged_list,
            &self.untracked_list,
            &self.ignored_list,
        ] {
            let children: Vec<glib::Object> =
                list.observe_children().into_iter().flatten().collect();
            for child in children {
                if let Ok(row) = child.downcast::<gtk::ListBoxRow>() {
                    let is_target = row
                        .child()
                        .map(|c| target.widget.eq(&c))
                        .unwrap_or(false);
                    if is_target {
                        list.select_row(Some(&row));
                        return true;
                    }
                }
            }
        }
        false
    }

    pub fn selected_path(&self) -> Option<PathBuf> {
        self.selected_path.borrow().clone()
    }

    pub fn diff_view_widget(&self) -> &Rc<DiffView> {
        &self.diff_view
    }
}

fn build_row(entry: &StatusEntry, kind: RowKind, sender: Sender) -> FileRow {
    let widget = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    widget.add_css_class("file-row");

    let display_kind = if kind == RowKind::Staged {
        entry.index
    } else {
        entry.display_unstaged_kind()
    };
    let (icon_name, css, tooltip) = if entry.is_conflicted() {
        ("dialog-warning-symbolic", "error", "Conflicted")
    } else {
        util::change_icon(display_kind)
    };
    let icon = gtk::Image::from_icon_name(icon_name);
    icon.set_tooltip_text(Some(tooltip));
    icon.set_pixel_size(14);
    if !css.is_empty() {
        icon.add_css_class(css);
    }

    let path_text = match &entry.orig_path {
        Some(orig) => format!("{} → {}", orig.display(), entry.path.display()),
        None => entry.path.display().to_string(),
    };
    let label = util::path_label(&path_text);
    label.set_tooltip_text(Some(&path_text));

    let action_button = gtk::Button::new();
    action_button.add_css_class("flat");
    action_button.set_valign(gtk::Align::Center);

    let menu = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .valign(gtk::Align::Center)
        .build();
    menu.add_css_class("flat");

    let path = entry.path.clone();

    match kind {
        RowKind::Staged => {
            action_button.set_icon_name("go-down-symbolic");
            action_button.set_tooltip_text(Some("Unstage"));
            let path = path.clone();
            let sender = sender.clone();
            action_button.connect_clicked(move |_| {
                sender(UiRequest::UnstagePaths(vec![path.clone()]));
            });
        }
        RowKind::Conflict => {
            action_button.set_icon_name("emblem-ok-symbolic");
            action_button.set_tooltip_text(Some("Mark as resolved (stage)"));
            let path = path.clone();
            let sender = sender.clone();
            action_button.connect_clicked(move |_| {
                sender(UiRequest::MarkResolved(path.clone()));
            });
        }
        RowKind::Ignored => {
            action_button.set_visible(false);
        }
        _ => {
            action_button.set_icon_name("list-add-symbolic");
            action_button.set_tooltip_text(Some("Stage"));
            let path = path.clone();
            let sender = sender.clone();
            action_button.connect_clicked(move |_| {
                sender(UiRequest::StagePaths(vec![path.clone()]));
            });
        }
    }

    menu.set_popover(Some(&row_popover(entry, kind, sender.clone())));

    widget.append(&icon);
    widget.append(&label);
    widget.append(&action_button);
    widget.append(&menu);

    FileRow {
        widget,
        path,
        orig_path: entry.orig_path.clone(),
        staged: kind == RowKind::Staged,
        untracked: kind == RowKind::Untracked,
        ignored: kind == RowKind::Ignored,
        conflicted: entry.is_conflicted(),
        has_unstaged: entry.is_unstaged(),
    }
}

fn row_popover(entry: &StatusEntry, kind: RowKind, sender: Sender) -> gtk::Popover {
    let popover = gtk::Popover::new();
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 2);
    box_.set_margin_start(6);
    box_.set_margin_end(6);
    box_.set_margin_top(6);
    box_.set_margin_bottom(6);

    let add_button = |label: &str, icon: &str, req: UiRequest, destructive: bool| {
        let button = gtk::Button::builder()
            .label(label)
            .icon_name(icon)
            .has_frame(false)
            .halign(gtk::Align::Fill)
            .build();
        if destructive {
            button.add_css_class("destructive-action");
        }
        let sender = sender.clone();
        button.connect_clicked(move |_| {
            sender(req.clone());
        });
        box_.append(&button);
    };

    let path = entry.path.clone();
    add_button(
        "Open File",
        "document-open-symbolic",
        UiRequest::OpenFile(path.clone()),
        false,
    );
    add_button(
        "Open Containing Folder",
        "folder-open-symbolic",
        UiRequest::OpenFolder(path.clone()),
        false,
    );
    add_button(
        "Copy Path",
        "edit-copy-symbolic",
        UiRequest::CopyPath(path.display().to_string()),
        false,
    );

    match kind {
        RowKind::Untracked => {
            add_button(
                "Add to .gitignore",
                "action-unavailable-symbolic",
                UiRequest::AddIgnore(path.clone()),
                false,
            );
            add_button(
                "Delete File…",
                "user-trash-symbolic",
                UiRequest::DeleteUntracked(path.clone()),
                true,
            );
        }
        RowKind::Ignored => {
            add_button(
                "Stop Ignoring",
                "list-add-symbolic",
                UiRequest::RemoveIgnore(path.clone()),
                false,
            );
        }
        RowKind::Conflict => {
            add_button(
                "Resolve Conflicts…",
                "dialog-warning-symbolic",
                UiRequest::ResolveConflict(path.clone()),
                false,
            );
        }
        _ => {}
    }
    if kind != RowKind::Ignored && !entry.is_conflicted() {
        add_button(
            "Discard Changes…",
            "edit-undo-symbolic",
            UiRequest::DiscardPaths(vec![path.clone()]),
            true,
        );
    }

    popover.set_child(Some(&box_));
    popover
}

fn overflow_popover(request: Rc<RefCell<Option<Sender>>>) -> gtk::Popover {
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
        let request = request.clone();
        let pop = popover.clone();
        button.connect_clicked(move |_| {
            if let Some(sender) = request.borrow().as_ref() {
                sender(req.clone());
            }
            pop.popdown();
        });
        box_.append(&button);
    };

    add("Discard all changes…", UiRequest::DiscardAll, true);
    add("Delete all untracked files…", UiRequest::CleanUntracked, true);

    popover.set_child(Some(&box_));
    popover
}

fn section(title: &str) -> (gtk::Box, gtk::ListBox, gtk::Label) {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let (header, count) = util::list_header(title);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    container.append(&header);
    container.append(&list);
    (container, list, count)
}
