//! Commit history: virtualised graph list, commit details and commit actions.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::git::{ChangedFile, Commit, FileDiff, RefKind};

use super::graph::{self, GraphRow, Segment, DOT_RADIUS, ROW_HEIGHT};
use super::repo_page::UiRequest;
use super::util;

#[derive(Clone)]
pub struct HistoryItemData {
    pub commit: Commit,
    pub row: GraphRow,
    pub max_lane: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchField {
    Message,
    Author,
    Hash,
    Path,
}

pub struct HistoryView {
    root: gtk::Paned,
    store: gio::ListStore,
    selection: gtk::SingleSelection,
    load_more: gtk::Button,
    search_entry: gtk::SearchEntry,
    search_field: gtk::DropDown,
    empty_label: gtk::Label,
    count_label: gtk::Label,
    request: Rc<RefCell<Option<Rc<dyn Fn(UiRequest)>>>>,
    // Details panel
    detail_summary: gtk::Label,
    detail_body: gtk::Label,
    detail_author: gtk::Label,
    detail_committer: gtk::Label,
    detail_sha: gtk::Label,
    detail_refs: gtk::Box,
    detail_files: gtk::ListBox,
    detail_stack: gtk::Stack,
    diff_view: Rc<super::diff_view::DiffView>,
    current_commit: RefCell<Option<Commit>>,
    current_files: RefCell<Vec<ChangedFile>>,
    current_diffs: RefCell<Vec<FileDiff>>,
    compare_base: RefCell<Option<String>>,
    has_more: RefCell<bool>,
}

impl HistoryView {
    pub fn new() -> Rc<Self> {
        let root = gtk::Paned::new(gtk::Orientation::Horizontal);
        root.set_wide_handle(true);
        root.set_position(560);

        // ---------------- left: commit list ----------------
        let left = gtk::Box::new(gtk::Orientation::Vertical, 0);

        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        toolbar.set_margin_top(6);
        toolbar.set_margin_bottom(6);
        toolbar.set_margin_start(8);
        toolbar.set_margin_end(8);
        let search_entry = gtk::SearchEntry::builder()
            .placeholder_text("Search commits…")
            .hexpand(true)
            .build();
        let field_model = gtk::StringList::new(&["Message", "Author", "Hash", "File path"]);
        let search_field = gtk::DropDown::builder()
            .model(&field_model)
            .selected(0)
            .tooltip_text("Search field")
            .build();
        let compare_button = util::icon_button(
            "git-compare-symbolic",
            "Compare two commits: select a Base, then a Target",
        );
        let refresh_button = util::icon_button("view-refresh-symbolic", "Reload history (Ctrl+R)");
        toolbar.append(&search_entry);
        toolbar.append(&search_field);
        toolbar.append(&compare_button);
        toolbar.append(&refresh_button);

        let sender_holder: Rc<RefCell<Option<Rc<dyn Fn(UiRequest)>>>> =
            Rc::new(RefCell::new(None));
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let selection = gtk::SingleSelection::new(Some(store.clone()));
        selection.set_autoselect(false);
        selection.set_can_unselect(true);
        let list_view = gtk::ListView::builder()
            .model(&selection)
            .factory(&history_factory(Rc::clone(&sender_holder)))
            .show_separators(false)
            .build();
        list_view.add_css_class("history-list");
        let list_scroll = gtk::ScrolledWindow::builder()
            .child(&list_view)
            .vexpand(true)
            .build();

        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        footer.set_margin_top(4);
        footer.set_margin_bottom(6);
        footer.set_margin_start(8);
        footer.set_margin_end(8);
        let empty_label = gtk::Label::builder()
            .label("No commits yet")
            .hexpand(true)
            .xalign(0.0)
            .build();
        empty_label.add_css_class("dim-label");
        let count_label = gtk::Label::builder().xalign(1.0).build();
        count_label.add_css_class("dim-label");
        let load_more = gtk::Button::with_label("Load more");
        load_more.add_css_class("flat");
        load_more.set_visible(false);
        footer.append(&empty_label);
        footer.append(&count_label);
        footer.append(&load_more);

        left.append(&toolbar);
        left.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        left.append(&list_scroll);
        left.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        left.append(&footer);

        // ---------------- right: details ----------------
        let right = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let detail_stack = gtk::Stack::builder()
            .vexpand(true)
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();

        let info_placeholder = gtk::Box::new(gtk::Orientation::Vertical, 6);
        info_placeholder.set_valign(gtk::Align::Center);
        info_placeholder.set_halign(gtk::Align::Center);
        let icon = gtk::Image::from_icon_name("document-open-recent-symbolic");
        icon.set_pixel_size(48);
        icon.add_css_class("dim-label");
        let title = gtk::Label::new(Some("Select a commit"));
        title.add_css_class("heading");
        let subtitle = gtk::Label::new(Some("Commit details and changed files appear here."));
        subtitle.add_css_class("dim-label");
        info_placeholder.append(&icon);
        info_placeholder.append(&title);
        info_placeholder.append(&subtitle);
        detail_stack.add_named(&info_placeholder, Some("empty"));

        let details = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let info_scroll = gtk::ScrolledWindow::builder()
            .max_content_height(280)
            .propagate_natural_height(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();

        let info_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        info_box.set_margin_start(12);
        info_box.set_margin_end(12);
        info_box.set_margin_top(12);
        info_box.set_margin_bottom(12);

        let detail_summary = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .selectable(true)
            .build();
        detail_summary.add_css_class("title-4");
        let detail_refs = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        detail_refs.set_halign(gtk::Align::Start);
        let detail_body = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .selectable(true)
            .build();
        detail_body.add_css_class("dim-label");

        let meta_grid = gtk::Grid::builder()
            .column_spacing(12)
            .row_spacing(3)
            .margin_top(4)
            .build();
        let detail_author = meta_label();
        let detail_committer = meta_label();
        let detail_sha = meta_label();
        detail_sha.set_selectable(true);
        meta_grid.attach(&meta_key("Author"), 0, 0, 1, 1);
        meta_grid.attach(&detail_author, 1, 0, 1, 1);
        meta_grid.attach(&meta_key("Committed"), 0, 1, 1, 1);
        meta_grid.attach(&detail_committer, 1, 1, 1, 1);
        meta_grid.attach(&meta_key("Commit"), 0, 2, 1, 1);
        meta_grid.attach(&detail_sha, 1, 2, 1, 1);

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        actions.set_margin_top(4);
        let copy_sha = util::icon_button("edit-copy-symbolic", "Copy commit SHA");
        let copy_message = util::icon_button("edit-copy-symbolic", "Copy commit message");
        let create_branch = util::icon_button("git-branch-symbolic", "Create branch here…");
        let create_tag = util::icon_button("tag-symbolic", "Create tag here…");
        let cherry_pick = util::icon_button("document-send-symbolic", "Cherry-pick this commit");
        let revert = util::icon_button("edit-undo-symbolic", "Revert this commit");
        let reset = util::icon_button("edit-redo-symbolic", "Reset current branch to here…");
        let checkout = util::icon_button("go-next-symbolic", "Checkout this commit (detached)");
        let compare = util::icon_button(
            "git-compare-symbolic",
            "Compare with working tree",
        );
        actions.append(&copy_sha);
        actions.append(&copy_message);
        actions.append(&create_branch);
        actions.append(&create_tag);
        actions.append(&cherry_pick);
        actions.append(&revert);
        actions.append(&reset);
        actions.append(&checkout);
        actions.append(&compare);

        let files_header = gtk::Label::builder()
            .label("Changed files")
            .xalign(0.0)
            .margin_top(4)
            .build();
        files_header.add_css_class("heading");
        let detail_files = gtk::ListBox::new();
        detail_files.add_css_class("boxed-list");
        detail_files.set_selection_mode(gtk::SelectionMode::Single);

        info_box.append(&detail_summary);
        info_box.append(&detail_refs);
        info_box.append(&detail_body);
        info_box.append(&meta_grid);
        info_box.append(&actions);
        info_box.append(&files_header);
        info_box.append(&detail_files);
        info_scroll.set_child(Some(&info_box));

        let diff_view = super::diff_view::DiffView::new();
        let diff_paned = gtk::Paned::new(gtk::Orientation::Vertical);
        diff_paned.set_start_child(Some(&info_scroll));
        diff_paned.set_end_child(Some(diff_view.widget()));
        diff_paned.set_position(300);
        diff_paned.set_resize_start_child(false);
        diff_paned.set_resize_end_child(true);
        details.append(&diff_paned);
        detail_stack.add_named(&details, Some("details"));

        let detail_stack_final = detail_stack;
        right.append(&detail_stack_final);

        root.set_start_child(Some(&left));
        root.set_end_child(Some(&right));
        root.set_position(620);

        let view = Rc::new_cyclic(|weak: &std::rc::Weak<Self>| {
            // Search
            {
                let w = weak.clone();
                let entry = search_entry.clone();
                let field = search_field.clone();
                search_entry.connect_search_changed(move |_| {
                    if let Some(v) = w.upgrade() {
                        v.emit_search(&entry, &field);
                    }
                });
                let w = weak.clone();
                let entry = search_entry.clone();
                let field = search_field.clone();
                search_entry.connect_activate(move |_| {
                    if let Some(v) = w.upgrade() {
                        v.emit_search(&entry, &field);
                    }
                });
            }
            {
                let w = weak.clone();
                refresh_button.connect_clicked(move |_| {
                    if let Some(v) = w.upgrade() {
                        v.send(UiRequest::RefreshHistory);
                    }
                });
            }
            {
                let w = weak.clone();
                load_more.connect_clicked(move |_| {
                    if let Some(v) = w.upgrade() {
                        v.send(UiRequest::LoadMoreHistory);
                    }
                });
            }
            {
                let w = weak.clone();
                compare_button.connect_clicked(move |_| {
                    if let Some(v) = w.upgrade() {
                        v.send(UiRequest::CompareCommits);
                    }
                });
            }
            // Commit selection
            {
                let w = weak.clone();
                selection.connect_selection_changed(move |sel, _, _| {
                    let Some(v) = w.upgrade() else { return };
                    let Some(item) = sel.selected_item() else { return };
                    let Ok(boxed) = item.downcast::<glib::BoxedAnyObject>() else {
                        return;
                    };
                    let data = boxed.borrow::<HistoryItemData>();
                    let sha = data.commit.id.clone();
                    drop(data);
                    v.send(UiRequest::SelectCommit(sha));
                });
            }
            // Details actions use the currently selected commit.
            {
                let w = weak.clone();
                copy_sha.connect_clicked(move |_| {
                    if let Some(v) = w.upgrade() {
                        if let Some(commit) = v.current_commit.borrow().clone() {
                            v.send(UiRequest::CopyText(commit.id));
                        }
                    }
                });
            }
            {
                let w = weak.clone();
                copy_message.connect_clicked(move |_| {
                    if let Some(v) = w.upgrade() {
                        if let Some(commit) = v.current_commit.borrow().clone() {
                            v.send(UiRequest::CopyText(commit.body));
                        }
                    }
                });
            }
            {
                let w = weak.clone();
                create_branch.connect_clicked(move |_| {
                    if let Some(v) = w.upgrade() {
                        if let Some(commit) = v.current_commit.borrow().clone() {
                            v.send(UiRequest::CreateBranchAt(commit.id));
                        }
                    }
                });
            }
            {
                let w = weak.clone();
                create_tag.connect_clicked(move |_| {
                    if let Some(v) = w.upgrade() {
                        if let Some(commit) = v.current_commit.borrow().clone() {
                            v.send(UiRequest::CreateTagAt(commit.id));
                        }
                    }
                });
            }
            {
                let w = weak.clone();
                cherry_pick.connect_clicked(move |_| {
                    if let Some(v) = w.upgrade() {
                        if let Some(commit) = v.current_commit.borrow().clone() {
                            v.send(UiRequest::CherryPick(commit.id));
                        }
                    }
                });
            }
            {
                let w = weak.clone();
                revert.connect_clicked(move |_| {
                    if let Some(v) = w.upgrade() {
                        if let Some(commit) = v.current_commit.borrow().clone() {
                            v.send(UiRequest::Revert(commit.id));
                        }
                    }
                });
            }
            {
                let w = weak.clone();
                reset.connect_clicked(move |_| {
                    if let Some(v) = w.upgrade() {
                        if let Some(commit) = v.current_commit.borrow().clone() {
                            v.send(UiRequest::ResetTo(commit.id));
                        }
                    }
                });
            }
            {
                let w = weak.clone();
                checkout.connect_clicked(move |_| {
                    if let Some(v) = w.upgrade() {
                        if let Some(commit) = v.current_commit.borrow().clone() {
                            v.send(UiRequest::CheckoutDetached(commit.id));
                        }
                    }
                });
            }
            {
                let w = weak.clone();
                compare.connect_clicked(move |_| {
                    if let Some(v) = w.upgrade() {
                        if let Some(commit) = v.current_commit.borrow().clone() {
                            v.send(UiRequest::CompareWithWorktree(commit.id));
                        }
                    }
                });
            }

            Self {
                root: root.clone(),
                store: store.clone(),
                selection: selection.clone(),
                load_more: load_more.clone(),
                search_entry: search_entry.clone(),
                search_field: search_field.clone(),
                empty_label: empty_label.clone(),
                count_label: count_label.clone(),
                request: Rc::clone(&sender_holder),
                detail_summary: detail_summary.clone(),
                detail_body: detail_body.clone(),
                detail_author: detail_author.clone(),
                detail_committer: detail_committer.clone(),
                detail_sha: detail_sha.clone(),
                detail_refs: detail_refs.clone(),
                detail_files: detail_files.clone(),
                detail_stack: detail_stack_final.clone(),
                diff_view: diff_view.clone(),
                current_commit: RefCell::new(None),
                current_files: RefCell::new(Vec::new()),
                current_diffs: RefCell::new(Vec::new()),
                compare_base: RefCell::new(None),
                has_more: RefCell::new(false),
            }
        });

        // File selection inside the details panel.
        {
            let w = Rc::downgrade(&view);
            view.detail_files.connect_row_selected(move |_, row| {
                let Some(row) = row else { return };
                let Some(v) = w.upgrade() else { return };
                let index = row.index() as usize;
                let files = v.current_files.borrow();
                let Some(file) = files.get(index) else { return };
                let path = file.path.clone();
                drop(files);
                let diffs = v.current_diffs.borrow();
                if let Some(diff) = diffs
                    .iter()
                    .find(|d| d.path().map(|p| p == &path).unwrap_or(false))
                    .cloned()
                {
                    v.diff_view.show_file(&diff);
                    v.diff_view.set_allow_actions(false);
                }
            });
        }

        view
    }

    pub fn widget(&self) -> &gtk::Paned {
        &self.root
    }

    pub fn diff_view(&self) -> &Rc<super::diff_view::DiffView> {
        &self.diff_view
    }

    pub fn connect_request(&self, request: Rc<dyn Fn(UiRequest)>) {
        *self.request.borrow_mut() = Some(request);
    }

    fn send(&self, req: UiRequest) {
        if let Some(r) = self.request.borrow().as_ref() {
            r(req);
        }
    }

    fn emit_search(&self, entry: &gtk::SearchEntry, field: &gtk::DropDown) {
        let query = entry.text().to_string();
        let field = match field.selected() {
            1 => SearchField::Author,
            2 => SearchField::Hash,
            3 => SearchField::Path,
            _ => SearchField::Message,
        };
        self.send(UiRequest::HistorySearch { query, field });
    }

    pub fn focus_search(&self) {
        self.search_entry.grab_focus();
    }

    pub fn search_field(&self) -> SearchField {
        match self.search_field.selected() {
            1 => SearchField::Author,
            2 => SearchField::Hash,
            3 => SearchField::Path,
            _ => SearchField::Message,
        }
    }

    pub fn search_query(&self) -> String {
        self.search_entry.text().to_string()
    }

    /// Replace the commit list.
    pub fn render(&self, commits: &[Commit], has_more: bool, max_lane: usize) {
        self.store.remove_all();
        let layout = graph::layout_commits(commits);
        let max = max_lane.max(layout.max_lane);
        for (commit, row) in commits.iter().zip(layout.rows.iter()) {
            self.store.append(&glib::BoxedAnyObject::new(HistoryItemData {
                commit: commit.clone(),
                row: row.clone(),
                max_lane: max,
            }));
        }
        *self.has_more.borrow_mut() = has_more;
        self.load_more.set_visible(has_more);
        let has_commits = !commits.is_empty();
        self.empty_label
            .set_label(if has_commits { "" } else { "No commits yet" });
        self.count_label
            .set_label(&format!("{} commits", commits.len()));
    }

    /// Append commits (pagination).
    pub fn append(&self, commits: &[Commit], has_more: bool, max_lane: usize) {
        let layout = graph::layout_commits(commits);
        let max = max_lane.max(layout.max_lane);
        for (commit, row) in commits.iter().zip(layout.rows.iter()) {
            self.store.append(&glib::BoxedAnyObject::new(HistoryItemData {
                commit: commit.clone(),
                row: row.clone(),
                max_lane: max,
            }));
        }
        *self.has_more.borrow_mut() = has_more;
        self.load_more.set_visible(has_more);
        self.count_label
            .set_label(&format!("{} commits", self.store.n_items()));
    }

    pub fn select_commit(&self, oid: &str) {
        for i in 0..self.store.n_items() {
            if let Some(item) = self.store.item(i) {
                if let Ok(boxed) = item.downcast::<glib::BoxedAnyObject>() {
                    if boxed.borrow::<HistoryItemData>().commit.id == oid {
                        self.selection.set_selected(i);
                        return;
                    }
                }
            }
        }
    }

    pub fn selected_commit(&self) -> Option<String> {
        let item = self.selection.selected_item()?;
        let boxed = item.downcast::<glib::BoxedAnyObject>().ok()?;
        let id = boxed.borrow::<HistoryItemData>().commit.id.clone();
        Some(id)
    }

    /// Show commit details and its diffs.
    pub fn show_commit(
        &self,
        commit: &Commit,
        files: Vec<ChangedFile>,
        diffs: Vec<FileDiff>,
    ) {
        *self.current_commit.borrow_mut() = Some(commit.clone());
        *self.current_files.borrow_mut() = files.clone();
        *self.current_diffs.borrow_mut() = diffs.clone();

        self.detail_summary.set_label(&commit.summary);
        let body = commit
            .body
            .strip_prefix(&commit.summary)
            .unwrap_or(&commit.body)
            .trim_start_matches('\n');
        self.detail_body.set_label(body);
        self.detail_body.set_visible(!body.is_empty());
        self.detail_author.set_label(&format!(
            "{} <{}>  ·  {}",
            commit.author_name,
            commit.author_email,
            util::absolute_time(commit.author_time)
        ));
        self.detail_committer.set_label(&format!(
            "{} <{}>  ·  {}",
            commit.committer_name,
            commit.committer_email,
            util::absolute_time(commit.commit_time)
        ));
        self.detail_sha.set_label(&commit.id);

        while let Some(child) = self.detail_refs.first_child() {
            self.detail_refs.remove(&child);
        }
        for r in &commit.refs {
            let (class, label) = match r.kind {
                RefKind::Head => ("head", format!("HEAD → {}", r.name)),
                RefKind::Branch => ("branch", r.name.clone()),
                RefKind::RemoteBranch => ("remote", r.name.clone()),
                RefKind::Tag => ("tag", format!("🏷 {}", r.name)),
            };
            if r.kind == RefKind::Head && r.name == "HEAD" {
                // detached HEAD marker
            }
            let badge = gtk::Label::new(Some(&label));
            badge.add_css_class("ref-badge");
            badge.add_css_class(class);
            self.detail_refs.append(&badge);
        }
        self.detail_refs
            .set_visible(!commit.refs.is_empty());

        while let Some(child) = self.detail_files.first_child() {
            self.detail_files.remove(&child);
        }
        for file in &files {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let (icon, css, tooltip) = util::change_icon(file.status);
            let image = gtk::Image::from_icon_name(icon);
            image.set_pixel_size(14);
            image.set_tooltip_text(Some(tooltip));
            if !css.is_empty() {
                image.add_css_class(css);
            }
            let text = match &file.orig_path {
                Some(orig) => format!("{} → {}", orig.display(), file.path.display()),
                None => file.path.display().to_string(),
            };
            let label = util::path_label(&text);
            row.append(&image);
            row.append(&label);
            row.set_margin_start(2);
            row.set_margin_end(2);
            row.set_margin_top(2);
            row.set_margin_bottom(2);
            self.detail_files.append(&row);
        }

        self.diff_view.clear();
        if let Some(first) = diffs.first() {
            self.diff_view.show_file(first);
            self.diff_view.set_allow_actions(false);
            self.detail_files.select_row(self.detail_files.row_at_index(0).as_ref());
        }
        self.detail_stack.set_visible_child_name("details");
    }

    pub fn prepare_compare(&self, base: &str) {
        *self.compare_base.borrow_mut() = Some(base.to_string());
    }

    pub fn compare_base(&self) -> Option<String> {
        self.compare_base.borrow().clone()
    }

    pub fn clear_compare(&self) {
        *self.compare_base.borrow_mut() = None;
    }

    pub fn show_compare(
        &self,
        base: &str,
        target: &str,
        files: Vec<ChangedFile>,
        diffs: Vec<FileDiff>,
    ) {
        let mut commit = Commit {
            id: target.to_string(),
            short_id: util::short_oid(target).to_string(),
            parents: vec![],
            author_name: String::new(),
            author_email: String::new(),
            author_time: 0,
            committer_name: String::new(),
            committer_email: String::new(),
            commit_time: 0,
            summary: format!(
                "Comparing {} → {}",
                util::short_oid(base),
                util::short_oid(target)
            ),
            body: String::new(),
            refs: vec![],
        };
        commit.parents = vec![base.to_string()];
        self.show_commit(&commit, files, diffs);
        self.detail_author.set_label("Base");
        self.detail_committer.set_label(target);
    }

    pub fn show_empty(&self) {
        self.detail_stack.set_visible_child_name("empty");
    }
}

fn history_factory(
    sender_holder: Rc<RefCell<Option<Rc<dyn Fn(UiRequest)>>>>,
) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.set_margin_start(4);
        row.set_margin_end(8);
        row.set_margin_top(2);
        row.set_margin_bottom(2);
        let graph = gtk::DrawingArea::new();
        graph.set_content_height(ROW_HEIGHT as i32);
        graph.set_valign(gtk::Align::Center);
        graph.add_css_class("commit-graph");
        let text_box = gtk::Box::new(gtk::Orientation::Vertical, 1);
        text_box.set_valign(gtk::Align::Center);
        text_box.set_hexpand(true);
        let summary = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        summary.add_css_class("commit-summary");
        let meta = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let sha = gtk::Label::new(None);
        sha.add_css_class("dim-label");
        sha.add_css_class("monospace");
        let author = gtk::Label::new(None);
        author.add_css_class("dim-label");
        let time = gtk::Label::new(None);
        time.add_css_class("dim-label");
        let refs = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        meta.append(&sha);
        meta.append(&author);
        meta.append(&time);
        meta.append(&refs);
        text_box.append(&summary);
        text_box.append(&meta);
        row.append(&graph);
        row.append(&text_box);
        let menu = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .valign(gtk::Align::Center)
            .tooltip_text("Commit actions")
            .build();
        menu.add_css_class("flat");
        menu.set_visible(false);
        row.append(&menu);
        item.set_child(Some(&row));
    });
    let holder_for_bind = Rc::clone(&sender_holder);
    factory.connect_bind(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(child) = item.child() else { return };
        let Ok(row) = child.downcast::<gtk::Box>() else {
            return;
        };
        let Some(boxed) = item.item().and_downcast::<glib::BoxedAnyObject>() else {
            return;
        };
        let data = boxed.borrow::<HistoryItemData>();
        let commit = data.commit.clone();
        let graph_row = data.row.clone();
        let max_lane = data.max_lane;
        drop(data);

        // Commit actions menu
        if let Some(menu) = row
            .last_child()
            .and_downcast::<gtk::MenuButton>()
        {
            menu.set_visible(true);
            let sender = holder_for_bind.borrow().clone();
            menu.set_popover(Some(&commit_context_popover(&commit, sender)));
        }

        // Graph drawing
        if let Some(graph) = row.first_child().and_downcast::<gtk::DrawingArea>() {
            let lane = graph_row.lane;
            let color = graph_row.color;
            let segments = graph_row.segments.clone();
            graph.set_content_width((graph::graph_width(max_lane)) as i32);
            graph.set_draw_func(move |_, cr, w, h| {
                draw_graph_row(cr, w as f64, h as f64, lane, color, &segments);
            });
        }

        // Text
        let mut children = Vec::new();
        let mut iter = row.first_child();
        while let Some(c) = iter {
            children.push(c.clone());
            iter = c.next_sibling();
        }
        if let Some(text_box) = children
            .get(1)
            .and_then(|c| c.clone().downcast::<gtk::Box>().ok())
        {
            let mut tb = Vec::new();
            let mut it = text_box.first_child();
            while let Some(c) = it {
                tb.push(c.clone());
                it = c.next_sibling();
            }
            if let Some(summary) = tb.first().and_then(|c| c.clone().downcast::<gtk::Label>().ok()) {
                summary.set_label(&commit.summary);
            }
            if let Some(meta) = tb.get(1).and_then(|c| c.clone().downcast::<gtk::Box>().ok()) {
                let mut m = Vec::new();
                let mut it = meta.first_child();
                while let Some(c) = it {
                    m.push(c.clone());
                    it = c.next_sibling();
                }
                if let Some(sha) = m.first().and_then(|c| c.clone().downcast::<gtk::Label>().ok()) {
                    sha.set_label(&commit.short_id);
                }
                if let Some(author) = m.get(1).and_then(|c| c.clone().downcast::<gtk::Label>().ok()) {
                    author.set_label(&format!("· {}", commit.author_name));
                }
                if let Some(time) = m.get(2).and_then(|c| c.clone().downcast::<gtk::Label>().ok()) {
                    time.set_label(&format!("· {}", util::relative_time(commit.commit_time)));
                }
                if let Some(refs) = m.get(3).and_then(|c| c.clone().downcast::<gtk::Box>().ok()) {
                    while let Some(c) = refs.first_child() {
                        refs.remove(&c);
                    }
                    for r in commit.refs.iter().take(6) {
                        let (class, label) = match r.kind {
                            RefKind::Head => ("head", "HEAD".to_string()),
                            RefKind::Branch => ("branch", r.name.clone()),
                            RefKind::RemoteBranch => ("remote", r.name.clone()),
                            RefKind::Tag => ("tag", format!("🏷 {}", r.name)),
                        };
                        let badge = gtk::Label::new(Some(&label));
                        badge.add_css_class("ref-badge");
                        badge.add_css_class(class);
                        refs.append(&badge);
                    }
                }
            }
        }
    });
    factory
}

fn draw_graph_row(
    cr: &gtk::cairo::Context,
    _w: f64,
    h: f64,
    lane: usize,
    color: usize,
    segments: &[Segment],
) {
    let mid = h / 2.0;
    for segment in segments {
        match segment {
            Segment::Vertical { lane: l } => {
                set_lane_color(cr, *l);
                cr.move_to(graph::lane_x(*l), 0.0);
                cr.line_to(graph::lane_x(*l), h);
                let _ = cr.stroke();
            }
            Segment::UpperVertical { lane: l } => {
                set_lane_color(cr, *l);
                cr.move_to(graph::lane_x(*l), 0.0);
                cr.line_to(graph::lane_x(*l), mid);
                let _ = cr.stroke();
            }
            Segment::LowerVertical { lane: l } => {
                set_lane_color(cr, *l);
                cr.move_to(graph::lane_x(*l), mid);
                cr.line_to(graph::lane_x(*l), h);
                let _ = cr.stroke();
            }
            Segment::Curve { from_lane, to_lane } => {
                set_lane_color(cr, *to_lane);
                let x1 = graph::lane_x(*from_lane);
                let x2 = graph::lane_x(*to_lane);
                cr.move_to(x1, mid);
                cr.curve_to(x1, mid + (h - mid) * 0.5, x2, mid + (h - mid) * 0.5, x2, h);
                let _ = cr.stroke();
            }
        }
    }
    // Commit dot
    set_lane_color(cr, color);
    cr.arc(graph::lane_x(lane), mid, DOT_RADIUS, 0.0, std::f64::consts::TAU);
    let _ = cr.fill();
}

fn set_lane_color(cr: &gtk::cairo::Context, lane: usize) {
    let c = util::lane_color(lane);
    cr.set_source_rgba(
        c.red() as f64,
        c.green() as f64,
        c.blue() as f64,
        c.alpha() as f64,
    );
    cr.set_line_width(2.0);
}

fn meta_key(text: &str) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .valign(gtk::Align::Start)
        .build();
    label.add_css_class("dim-label");
    label
}

fn meta_label() -> gtk::Label {
    let label = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .selectable(true)
        .build();
    label
}

fn commit_context_popover(commit: &Commit, sender: Option<Rc<dyn Fn(UiRequest)>>) -> gtk::Popover {
    let popover = gtk::Popover::new();
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 2);
    box_.set_margin_start(6);
    box_.set_margin_end(6);
    box_.set_margin_top(6);
    box_.set_margin_bottom(6);
    let add = |label: &str, req: UiRequest| {
        let button = gtk::Button::builder()
            .label(label)
            .has_frame(false)
            .halign(gtk::Align::Fill)
            .build();
        let sender = sender.clone();
        let pop = popover.clone();
        button.connect_clicked(move |_| {
            if let Some(s) = sender.as_ref() {
                s(req.clone());
            }
            pop.popdown();
        });
        box_.append(&button);
    };
    let sha = commit.id.clone();
    add("Copy SHA", UiRequest::CopyText(sha.clone()));
    add("Copy message", UiRequest::CopyText(commit.body.clone()));
    add("Create branch here…", UiRequest::CreateBranchAt(sha.clone()));
    add("Create tag here…", UiRequest::CreateTagAt(sha.clone()));
    add("Checkout (detached)", UiRequest::CheckoutDetached(sha.clone()));
    add("Cherry-pick", UiRequest::CherryPick(sha.clone()));
    add("Revert", UiRequest::Revert(sha.clone()));
    add("Reset current branch to here…", UiRequest::ResetTo(sha.clone()));
    add(
        "Compare with working tree",
        UiRequest::CompareWithWorktree(sha.clone()),
    );
    popover.set_child(Some(&box_));
    popover
}
