//! Repository page: header, section switcher, all operations and dialogs.
//!
//! Views send [`UiRequest`] values; this page executes them against the
//! repository (on worker threads) and re-renders the views.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use vte4::prelude::*;

use crate::core::{load_snapshot, RepoSnapshot, Settings, SnapshotOptions};
use crate::git::{
    Branch, BranchIntegration, Cancellation, Commit, CommitOptions, ConflictChoice, FileDiff,
    GitError, GitResult, MergeOutcome, OperationState, Repository, ResetMode, StatusEntry,
    StreamKind,
};

use super::changes_view::ChangesView;
use super::diff_view::DiffMode;
use super::history_view::{HistoryView, SearchField};
use super::tasks::{self, StreamingOp};
use super::util;
use super::{branch_menu::BranchMenu, conflict_view::ConflictView, remotes_view::RemotesView};
use super::{stashes_view::StashesView, tags_view::TagsView};

pub type Sender = Rc<dyn Fn(UiRequest)>;

#[derive(Debug, Clone)]
pub enum UiRequest {
    // Changes view
    SelectDiff {
        path: PathBuf,
        staged: bool,
        rev: Option<String>,
    },
    StagePaths(Vec<PathBuf>),
    UnstagePaths(Vec<PathBuf>),
    DiscardPaths(Vec<PathBuf>),
    DeleteUntracked(PathBuf),
    StageAll,
    UnstageAll,
    DiscardAll,
    CleanUntracked,
    AddIgnore(PathBuf),
    RemoveIgnore(PathBuf),
    StageHunk {
        file: FileDiff,
        hunk: usize,
    },
    UnstageHunk {
        file: FileDiff,
        hunk: usize,
    },
    DiscardHunk {
        file: FileDiff,
        hunk: usize,
    },
    OpenFile(PathBuf),
    OpenFolder(PathBuf),
    CopyPath(String),
    CopyText(String),
    OpenContainingFolder,
    // Commits
    Commit(Box<CommitOptions>),
    // History
    RefreshAll,
    ShowSection(&'static str),
    QuickPull,
    QuickPush,
    PullOptions,
    RefreshHistory,
    LoadMoreHistory,
    SelectCommit(String),
    HistorySearch {
        query: String,
        field: SearchField,
    },
    CreateBranchAt(String),
    CreateTagAt(String),
    CherryPick(String),
    Revert(String),
    ResetTo(String),
    CheckoutDetached(String),
    CompareWithWorktree(String),
    CompareCommits,
    ShowTagInHistory(String),
    // Branches
    CreateBranch,
    CheckoutBranch(String),
    CheckoutRemoteBranch {
        remote: String,
        branch: String,
    },
    DeleteBranch(String),
    DeleteRemoteBranch {
        remote: String,
        branch: String,
    },
    RenameBranch(String),
    MergeBranch(String),
    RebaseOnto(String),
    IntegrateBranches {
        source: String,
        source_tip: String,
        target: String,
        target_tip: String,
    },
    PickBranchIntegration {
        source: Option<String>,
    },
    SetUpstream(String),
    CompareWithBranch(String),
    CompareBranches,
    PushBranch(String),
    // Tags
    CreateTag,
    DeleteTag(String),
    PushTag(String),
    DeleteRemoteTag(String),
    CheckoutTag(String),
    // Stashes
    CreateStash,
    ShowStash(usize),
    ApplyStash(usize),
    ApplyStashIndex(usize),
    PopStash(usize),
    DropStash(usize),
    BranchFromStash(usize),
    // Remotes
    AddRemote,
    EditRemote(String),
    RenameRemote(String),
    RemoveRemote(String),
    FetchRemote(String),
    FetchAll,
    PushTo(String),
    PullFrom(String),
    PushAllTags,
    PushAllTagsTo(String),
    // Conflicts
    SelectConflict(PathBuf),
    ResolveConflict(PathBuf),
    AcceptConflict {
        path: PathBuf,
        region: usize,
        choice: ConflictChoice,
    },
    AcceptAllConflicts {
        path: PathBuf,
        choice: ConflictChoice,
    },
    ResolveConflictSide {
        path: PathBuf,
        ours: bool,
    },
    MarkResolved(PathBuf),
    ContinueOperation,
    SkipOperation,
    AbortOperation,
    // Misc
    RepoPreferences,
}

/// Build the refspecs used for pushing. Short names such as `main` become
/// `refs/heads/main`; full refs and `HEAD` are passed through. Returns `None`
/// for empty or option-like names, which callers report instead of silently
/// doing nothing.
pub fn push_refspecs(local: &str, remote_branch: &str) -> Option<(String, String)> {
    let local = local.trim();
    let remote_branch = remote_branch.trim();
    let invalid = |name: &str| name.is_empty() || name.starts_with('-');
    if invalid(local) || invalid(remote_branch) {
        return None;
    }
    let local_ref = if local.starts_with("refs/") || local == "HEAD" {
        local.to_string()
    } else {
        format!("refs/heads/{local}")
    };
    let remote_ref = if remote_branch.starts_with("refs/") {
        remote_branch.to_string()
    } else {
        format!("refs/heads/{remote_branch}")
    };
    Some((local_ref, remote_ref))
}

/// Split an upstream like `origin/feature/x` into `("origin", "feature/x")`.
pub fn split_upstream(upstream: &str) -> Option<(String, String)> {
    let (remote, branch) = upstream.split_once('/')?;
    if remote.is_empty() || branch.is_empty() {
        return None;
    }
    Some((remote.to_string(), branch.to_string()))
}

#[derive(Default)]
struct RepoState {
    snapshot: RepoSnapshot,
    history: Vec<Commit>,
    history_has_more: bool,
    history_limit: usize,
    history_search: Option<(String, SearchField)>,
    history_filter_rev: Option<String>,
    active_op: Option<StreamingOp>,
    integrating_branches: bool,
    output_lines: Vec<String>,
    loading: bool,
    pending_refresh: bool,
    compare_base: Option<String>,
}

pub struct RepoPage {
    pub root: gtk::Box,
    pub toolbar_view: adw::ToolbarView,
    repo: Repository,
    repo_path: PathBuf,
    overlay: adw::ToastOverlay,
    view_stack: adw::ViewStack,
    conflicts_page: adw::ViewStackPage,
    changes: Rc<ChangesView>,
    history: Rc<HistoryView>,
    tags: Rc<TagsView>,
    stashes: Rc<StashesView>,
    remotes: Rc<RemotesView>,
    conflicts: Rc<ConflictView>,
    header: adw::HeaderBar,
    branch_menu: BranchMenu,
    branch_label: gtk::Label,
    sync_label: gtk::Label,
    op_banner: gtk::Box,
    op_label: gtk::Label,
    progress_box: gtk::Box,
    progress_label: gtk::Label,
    progress_bar: gtk::ProgressBar,
    cancel_button: gtk::Button,
    output_view: gtk::TextView,
    output_expander: gtk::Expander,
    terminal_panel: gtk::Box,
    terminal_button: gtk::ToggleButton,
    terminal: vte4::Terminal,
    terminal_pid: RefCell<Option<glib::Pid>>,
    terminal_spawning: std::cell::Cell<bool>,
    state: RefCell<RepoState>,
    settings: RefCell<Settings>,
}

impl RepoPage {
    pub fn new(repo: Repository, settings: Settings) -> Rc<Self> {
        let repo_path = repo.workdir().to_path_buf();
        let display_name = repo.display_name();

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let overlay = adw::ToastOverlay::new();
        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_css_class("repo-page");

        // ---------------- header ----------------
        let header = adw::HeaderBar::new();
        header.set_show_title(true);
        let title_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let title_label = gtk::Label::builder()
            .label(&display_name)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(28)
            .tooltip_text(repo_path.display().to_string())
            .build();
        title_label.add_css_class("heading");

        let branch_label = gtk::Label::new(Some("…"));
        branch_label.set_max_width_chars(24);
        branch_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let branch_button = gtk::MenuButton::builder()
            .child(&branch_label)
            .tooltip_text("Branch actions")
            .build();
        branch_button.add_css_class("flat");
        branch_button.add_css_class("pill");

        let sync_label = gtk::Label::new(None);
        sync_label.add_css_class("dim-label");
        title_box.append(&title_label);
        title_box.append(&branch_button);
        title_box.append(&sync_label);
        header.set_title_widget(Some(&title_box));

        let fetch_button = util::icon_button("view-refresh-symbolic", "Fetch (Ctrl+F)");
        let pull_button =
            util::icon_button("go-down-symbolic", "Pull from upstream (Ctrl+Shift+P)");
        let push_button = util::icon_button("go-up-symbolic", "Push to upstream (Ctrl+P)");
        let refresh_button = util::icon_button("emblem-synchronizing-symbolic", "Refresh (Ctrl+R)");
        let menu_button = util::menu_button("open-menu-symbolic", "Repository menu");

        header.pack_end(&menu_button);
        header.pack_end(&refresh_button);
        header.pack_end(&push_button);
        header.pack_end(&pull_button);
        header.pack_end(&fetch_button);
        toolbar_view.add_top_bar(&header);

        // ---------------- section switcher ----------------
        let view_stack = adw::ViewStack::new();
        view_stack.set_vexpand(true);
        let changes = ChangesView::new();
        let history = HistoryView::new();
        let tags = TagsView::new();
        let stashes = StashesView::new();
        let remotes = RemotesView::new();
        let conflicts = ConflictView::new();

        let page_names = [
            ("changes", "Changes", "document-edit-symbolic"),
            ("history", "History", "document-open-recent-symbolic"),
            ("tags", "Tags", "tag-symbolic"),
            ("stashes", "Stashes", "view-paged-symbolic"),
            ("remotes", "Remotes", "network-server-symbolic"),
            ("conflicts", "Conflicts", "dialog-warning-symbolic"),
        ];
        let widgets: [gtk::Widget; 6] = [
            changes.widget().clone().upcast(),
            history.widget().clone().upcast(),
            tags.widget().clone().upcast(),
            stashes.widget().clone().upcast(),
            remotes.widget().clone().upcast(),
            conflicts.widget().clone().upcast(),
        ];
        let mut conflicts_page_handle = None;
        for ((name, title, icon), widget) in page_names.iter().zip(widgets.iter()) {
            let page = view_stack.add_titled(widget, Some(name), title);
            page.set_icon_name(Some(icon));
            if *name == "conflicts" {
                conflicts_page_handle = Some(page.clone());
            }
        }
        view_stack.set_visible_child_name("changes");
        let conflicts_page = conflicts_page_handle.expect("conflicts page");
        conflicts_page.set_visible(false);

        let switcher = adw::ViewSwitcher::builder()
            .stack(&view_stack)
            .policy(adw::ViewSwitcherPolicy::Wide)
            .build();
        let switcher_bar = gtk::CenterBox::new();
        switcher_bar.set_center_widget(Some(&switcher));
        switcher_bar.add_css_class("view-switcher-bar");
        toolbar_view.add_top_bar(&switcher_bar);

        // ---------------- operation banner ----------------
        let op_banner = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        op_banner.add_css_class("op-banner");
        op_banner.set_margin_top(6);
        op_banner.set_margin_start(8);
        op_banner.set_margin_end(8);
        let op_icon = gtk::Image::from_icon_name("dialog-warning-symbolic");
        op_icon.set_pixel_size(16);
        let op_label = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .wrap(true)
            .build();
        let op_continue = gtk::Button::with_label("Continue");
        op_continue.add_css_class("suggested-action");
        let op_abort = gtk::Button::with_label("Abort");
        op_abort.add_css_class("destructive-action");
        op_banner.append(&op_icon);
        op_banner.append(&op_label);
        op_banner.append(&op_continue);
        op_banner.append(&op_abort);
        op_banner.set_visible(false);

        // ---------------- body ----------------
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.append(&op_banner);
        body.append(&view_stack);

        overlay.set_child(Some(&body));
        toolbar_view.set_content(Some(&overlay));

        // ---------------- bottom bar ----------------
        let bottom = gtk::Box::new(gtk::Orientation::Vertical, 0);

        let progress_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        progress_box.set_margin_top(4);
        progress_box.set_margin_bottom(4);
        progress_box.set_margin_start(10);
        progress_box.set_margin_end(10);
        let progress_label = gtk::Label::builder()
            .label("")
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        progress_label.add_css_class("monospace");
        progress_label.add_css_class("dim-label");
        let progress_bar = gtk::ProgressBar::builder()
            .show_text(false)
            .width_request(220)
            .valign(gtk::Align::Center)
            .build();
        let cancel_button = gtk::Button::with_label("Cancel");
        cancel_button.add_css_class("flat");
        progress_box.append(&progress_label);
        progress_box.append(&progress_bar);
        progress_box.append(&cancel_button);
        progress_box.set_visible(false);

        let output_view = gtk::TextView::builder()
            .editable(false)
            .monospace(true)
            .wrap_mode(gtk::WrapMode::WordChar)
            .height_request(120)
            .build();
        output_view.add_css_class("dim-label");
        let output_scroll = gtk::ScrolledWindow::builder()
            .child(&output_view)
            .min_content_height(120)
            .max_content_height(200)
            .has_frame(true)
            .build();
        let output_expander = gtk::Expander::builder()
            .label("Git output")
            .child(&output_scroll)
            .build();

        // ---------------- embedded terminal ----------------
        let terminal = vte4::Terminal::new();
        terminal.set_vexpand(true);
        terminal.set_hexpand(true);
        terminal.set_scrollback_lines(10_000);
        terminal.set_mouse_autohide(true);
        let font = gtk::pango::FontDescription::from_string("Monospace 10");
        terminal.set_font(Some(&font));
        terminal.add_css_class("terminal");

        let terminal_scroll = gtk::ScrolledWindow::builder()
            .child(&terminal)
            .vexpand(true)
            .hexpand(true)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .build();

        let terminal_header = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        terminal_header.set_margin_top(2);
        terminal_header.set_margin_bottom(2);
        terminal_header.set_margin_start(8);
        terminal_header.set_margin_end(4);
        let terminal_title = gtk::Label::builder()
            .label("Terminal")
            .xalign(0.0)
            .hexpand(true)
            .build();
        terminal_title.add_css_class("heading");
        let terminal_clear = util::icon_button("edit-clear-symbolic", "Clear terminal");
        let terminal_close = util::icon_button("window-close-symbolic", "Close terminal");
        terminal_header.append(&terminal_title);
        terminal_header.append(&terminal_clear);
        terminal_header.append(&terminal_close);
        {
            let terminal = terminal.clone();
            terminal_clear.connect_clicked(move |_| {
                terminal.reset(true, true);
            });
        }

        let terminal_panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
        terminal_panel.add_css_class("terminal-panel");
        terminal_panel.set_height_request(320);
        terminal_panel.set_visible(false);
        terminal_panel.append(&terminal_header);
        terminal_panel.append(&terminal_scroll);

        bottom.append(&progress_box);
        bottom.append(&terminal_panel);
        bottom.append(&output_expander);
        toolbar_view.add_bottom_bar(&bottom);

        let output_expander_for_sync = output_expander.clone();
        let output_button = gtk::ToggleButton::builder()
            .icon_name("text-x-generic-symbolic")
            .tooltip_text("Show Git output")
            .build();
        output_button.add_css_class("flat");
        output_button
            .bind_property("active", &output_expander_for_sync, "expanded")
            .bidirectional()
            .sync_create()
            .build();
        let terminal_button = gtk::ToggleButton::builder()
            .icon_name("utilities-terminal-symbolic")
            .tooltip_text("Toggle terminal (Ctrl+`)")
            .build();
        terminal_button.add_css_class("flat");
        header.pack_start(&terminal_button);
        header.pack_start(&output_button);

        root.set_vexpand(true);
        root.set_hexpand(true);
        root.append(&toolbar_view);

        let page = Rc::new_cyclic(|weak: &std::rc::Weak<Self>| {
            let sender: Sender = {
                let weak = weak.clone();
                Rc::new(move |req: UiRequest| {
                    if let Some(page) = weak.upgrade() {
                        page.handle_request(req);
                    }
                })
            };

            let menu_request: Rc<RefCell<Option<Sender>>> = Rc::new(RefCell::new(None));
            *menu_request.borrow_mut() = Some(sender.clone());
            changes.connect_request(sender.clone());
            history.connect_request(sender.clone());
            let branch_menu = BranchMenu::new(&branch_button, sender.clone());
            tags.connect_request(sender.clone());
            stashes.connect_request(sender.clone());
            remotes.connect_request(sender.clone());
            conflicts.connect_request(sender.clone());

            // Header buttons
            {
                let w = weak.clone();
                fetch_button.connect_clicked(move |_| {
                    if let Some(p) = w.upgrade() {
                        p.quick_fetch();
                    }
                });
            }
            {
                let w = weak.clone();
                pull_button.connect_clicked(move |_| {
                    if let Some(p) = w.upgrade() {
                        p.quick_pull();
                    }
                });
            }
            {
                let w = weak.clone();
                push_button.connect_clicked(move |_| {
                    if let Some(p) = w.upgrade() {
                        p.quick_push();
                    }
                });
            }
            {
                let w = weak.clone();
                refresh_button.connect_clicked(move |_| {
                    if let Some(p) = w.upgrade() {
                        p.refresh_all();
                    }
                });
            }
            {
                let w = weak.clone();
                op_continue.connect_clicked(move |_| {
                    if let Some(p) = w.upgrade() {
                        p.handle_request(UiRequest::ContinueOperation);
                    }
                });
            }
            {
                let w = weak.clone();
                op_abort.connect_clicked(move |_| {
                    if let Some(p) = w.upgrade() {
                        p.handle_request(UiRequest::AbortOperation);
                    }
                });
            }
            {
                let w = weak.clone();
                cancel_button.connect_clicked(move |_| {
                    if let Some(p) = w.upgrade() {
                        p.cancel_active_op();
                    }
                });
            }
            {
                menu_button.set_popover(Some(&repo_menu_popover(
                    menu_request.clone(),
                    repo_path.clone(),
                )));
            }
            {
                let w = weak.clone();
                terminal_button.connect_toggled(move |button| {
                    if let Some(p) = w.upgrade() {
                        p.on_terminal_toggled(button.is_active());
                    }
                });
            }
            {
                let w = weak.clone();
                terminal_close.connect_clicked(move |_| {
                    if let Some(p) = w.upgrade() {
                        p.terminal_button.set_active(false);
                    }
                });
            }
            {
                let terminal_button = terminal_button.clone();
                output_expander.connect_expanded_notify(move |expander| {
                    if expander.is_expanded() {
                        terminal_button.set_active(false);
                    }
                });
            }
            {
                let w = weak.clone();
                terminal.connect_child_exited(move |_, _| {
                    if let Some(p) = w.upgrade() {
                        p.terminal_pid.borrow_mut().take();
                    }
                });
            }

            let page_settings = settings.clone();
            let page = Self {
                root: root.clone(),
                toolbar_view: toolbar_view.clone(),
                repo: repo.clone(),
                repo_path: repo_path.clone(),
                overlay: overlay.clone(),
                view_stack: view_stack.clone(),
                conflicts_page: conflicts_page.clone(),
                changes: changes.clone(),
                history: history.clone(),
                tags: tags.clone(),
                stashes: stashes.clone(),
                remotes: remotes.clone(),
                conflicts: conflicts.clone(),
                header: header.clone(),
                branch_menu,
                branch_label: branch_label.clone(),
                sync_label: sync_label.clone(),
                op_banner: op_banner.clone(),
                op_label: op_label.clone(),
                progress_box: progress_box.clone(),
                progress_label: progress_label.clone(),
                progress_bar: progress_bar.clone(),
                cancel_button: cancel_button.clone(),
                output_view: output_view.clone(),
                output_expander: output_expander.clone(),
                terminal_panel: terminal_panel.clone(),
                terminal_button: terminal_button.clone(),
                terminal: terminal.clone(),
                terminal_pid: RefCell::new(None),
                terminal_spawning: std::cell::Cell::new(false),
                state: RefCell::new(RepoState {
                    history_limit: 2000,
                    ..Default::default()
                }),
                settings: RefCell::new(page_settings),
            };
            let settings = page.settings.borrow().clone();
            page.changes.set_show_ignored(settings.show_ignored_files);
            page.changes
                .set_diff_mode(diff_mode_from(settings.diff_view_mode));
            page.changes
                .commit_panel()
                .set_signoff(settings.commit_sign_off);
            page.changes
                .commit_panel()
                .set_gpg(settings.commit_gpg_sign);
            page
        });

        let initial = page.clone();
        glib::idle_add_local_once(move || {
            initial.refresh_all();
        });

        page
    }

    /// True while a background load or a long-running operation is active.
    pub fn is_busy(&self) -> bool {
        let state = self.state.borrow();
        state.loading || state.integrating_branches || state.active_op.is_some()
    }

    pub fn repository(&self) -> &Repository {
        &self.repo
    }

    pub fn path(&self) -> &PathBuf {
        &self.repo_path
    }

    pub fn title(&self) -> String {
        self.repo.display_name()
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn toast(&self, message: &str) {
        util::toast(&self.overlay, message);
    }

    fn window(&self) -> Option<gtk::Window> {
        self.root.root().and_downcast::<gtk::Window>()
    }

    fn error(&self, error: &GitError) {
        crate::log_error!(
            "{}: {}",
            self.repo.display_name(),
            error.technical_details()
        );
        match self.window() {
            Some(window) => super::error_dialog::show(&window, error),
            None => {
                self.toast(&error.user_message());
            }
        }
    }

    fn append_output(&self, line: &str) {
        let mut state = self.state.borrow_mut();
        state.output_lines.push(line.to_string());
        if state.output_lines.len() > 4000 {
            let drain = state.output_lines.len() - 4000;
            state.output_lines.drain(0..drain);
        }
        let text = state.output_lines.join("\n");
        drop(state);
        self.output_view.buffer().set_text(&text);
        let mut end = self.output_view.buffer().end_iter();
        self.output_view
            .scroll_to_iter(&mut end, 0.0, false, 0.0, 0.0);
    }

    fn set_progress(&self, label: Option<&str>) {
        match label {
            Some(text) => {
                self.progress_box.set_visible(true);
                self.progress_label.set_label(text);
                self.progress_bar.pulse();
                self.remotes.set_progress(Some(text), None);
            }
            None => {
                self.progress_box.set_visible(false);
                self.progress_label.set_label("");
                self.cancel_button.set_sensitive(true);
                self.remotes.set_progress(None, None);
            }
        }
    }

    fn cancel_active_op(&self) {
        let cancelled = {
            let state = self.state.borrow();
            match state.active_op.as_ref() {
                Some(op) => {
                    op.cancel();
                    true
                }
                None => false,
            }
        };
        if cancelled {
            // The worker kills the whole process group; the bar disappears as
            // soon as the result arrives.
            self.progress_label.set_label("Cancelling…");
            self.cancel_button.set_sensitive(false);
        }
    }

    fn on_terminal_toggled(self: &Rc<Self>, active: bool) {
        self.terminal_panel.set_visible(active);
        if active {
            // Terminal and Git output share the bottom area.
            self.output_expander.set_expanded(false);
            self.ensure_terminal();
            self.terminal.grab_focus();
        }
    }

    fn ensure_terminal(self: &Rc<Self>) {
        if self.terminal_pid.borrow().is_some() || self.terminal_spawning.get() {
            return;
        }
        let shell = std::env::var("SHELL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| {
                if std::path::Path::new("/bin/bash").exists() {
                    "/bin/bash".to_string()
                } else {
                    "/bin/sh".to_string()
                }
            });
        let cwd = self.repo_path.to_string_lossy().to_string();
        let env = ["TERM=xterm-256color", "COLORTERM=truecolor"];
        let weak = Rc::downgrade(self);
        self.terminal_spawning.set(true);
        self.terminal.spawn_async(
            vte4::PtyFlags::DEFAULT,
            Some(&cwd),
            &[shell.as_str()],
            &env,
            glib::SpawnFlags::DEFAULT,
            || {},
            -1,
            None::<&gio::Cancellable>,
            move |result| {
                let Some(page) = weak.upgrade() else { return };
                page.terminal_spawning.set(false);
                match result {
                    Ok(pid) => {
                        *page.terminal_pid.borrow_mut() = Some(pid);
                    }
                    Err(error) => {
                        crate::log_warn!("could not start terminal: {error}");
                        page.toast(&format!("Could not start terminal: {error}"));
                        page.terminal_button.set_active(false);
                    }
                }
            },
        );
    }

    /// Open or hide the embedded terminal.
    pub fn toggle_terminal(&self) {
        self.terminal_button
            .set_active(!self.terminal_button.is_active());
    }

    // -----------------------------------------------------------------
    // Task helpers
    // -----------------------------------------------------------------

    /// Run a blocking git operation and refresh afterwards.
    fn run<T, F, D>(self: &Rc<Self>, work: F, on_done: D)
    where
        F: FnOnce() -> GitResult<T> + Send + 'static,
        T: Send + 'static,
        D: FnOnce(&Rc<Self>, T) + 'static,
    {
        let weak = Rc::downgrade(self);
        tasks::run_blocking(work, move |result| {
            if let Some(page) = weak.upgrade() {
                match result {
                    Ok(value) => on_done(&page, value),
                    Err(e) => page.error(&e),
                }
            }
        });
    }

    /// Like [`RepoPage::run`], but lets the caller handle errors.
    fn run_with_error<T, F, D, E>(self: &Rc<Self>, work: F, on_done: D, on_error: E)
    where
        F: FnOnce() -> GitResult<T> + Send + 'static,
        T: Send + 'static,
        D: FnOnce(&Rc<Self>, T) + 'static,
        E: FnOnce(&Rc<Self>, GitError) + 'static,
    {
        let weak = Rc::downgrade(self);
        tasks::run_blocking(work, move |result| {
            if let Some(page) = weak.upgrade() {
                match result {
                    Ok(value) => on_done(&page, value),
                    Err(e) => on_error(&page, e),
                }
            }
        });
    }

    /// Run a git operation with progress reporting and cancellation.
    fn run_streaming<T, F, D>(self: &Rc<Self>, label: &str, work: F, on_done: D)
    where
        T: Send + 'static,
        F: FnOnce(Cancellation, Box<dyn FnMut(StreamKind, &str) + Send>) -> GitResult<T>
            + Send
            + 'static,
        D: FnOnce(&Rc<Self>, GitResult<T>) + 'static,
    {
        if self.state.borrow().active_op.is_some() {
            self.toast("Another operation is already running");
            return;
        }
        self.set_progress(Some(label));
        self.append_output(&format!("== {label} =="));
        let weak = Rc::downgrade(self);
        let on_line = {
            let weak = weak.clone();
            move |line: String| {
                if let Some(page) = weak.upgrade() {
                    page.set_progress(Some(&line));
                    page.append_output(&line);
                }
            }
        };
        let op = tasks::run_streaming(work, on_line, move |result| {
            if let Some(page) = weak.upgrade() {
                page.state.borrow_mut().active_op = None;
                page.set_progress(None);
                match &result {
                    Ok(_) => {
                        page.append_output("== done ==");
                    }
                    Err(GitError::Cancelled) => {
                        page.append_output("== cancelled ==");
                        page.toast("Operation cancelled");
                    }
                    Err(e) => {
                        page.append_output(&format!("== failed: {}", e.technical_details()));
                    }
                }
                on_done(&page, result);
            }
        });
        self.state.borrow_mut().active_op = Some(op);
    }

    // -----------------------------------------------------------------
    // Refresh
    // -----------------------------------------------------------------

    pub fn refresh_all(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            if state.loading {
                state.pending_refresh = true;
                return;
            }
            state.loading = true;
            state.pending_refresh = false;
        }
        let repo = self.repo.clone();
        let options = SnapshotOptions::default();
        let weak = Rc::downgrade(self);
        tasks::run_blocking(
            move || load_snapshot(&repo, options),
            move |(snapshot, errors)| {
                let Some(page) = weak.upgrade() else { return };
                let pending = {
                    let mut state = page.state.borrow_mut();
                    state.loading = false;
                    let pending = state.pending_refresh;
                    state.pending_refresh = false;
                    pending
                };
                for (section, error) in &errors {
                    crate::log_warn!("failed to load {section}: {}", error.user_message());
                }
                page.state.borrow_mut().snapshot = snapshot;
                page.render();
                page.refresh_history();
                if pending {
                    page.refresh_all();
                }
            },
        );
    }

    fn refresh_status(self: &Rc<Self>) {
        let repo = self.repo.clone();
        let weak = Rc::downgrade(self);
        tasks::run_blocking(
            move || load_snapshot(&repo, SnapshotOptions::status_only()),
            move |(status, _errors)| {
                let Some(page) = weak.upgrade() else { return };
                page.state.borrow_mut().snapshot.status = status.status;
                page.render_status();
            },
        );
    }

    fn render(&self) {
        let state = self.state.borrow();
        let snapshot = state.snapshot.clone();
        drop(state);

        self.changes
            .set_show_ignored(self.settings.borrow().show_ignored_files);
        self.changes.render(&snapshot.status);
        self.branch_menu.render(&snapshot.branches);
        self.tags.render(&snapshot.tags);
        self.stashes.render(&snapshot.stashes);
        self.remotes.render(&snapshot.remotes);

        let conflicted = snapshot.status.conflicted();
        self.conflicts.render_list(
            &conflicted
                .iter()
                .map(|e| e.path.clone())
                .collect::<Vec<_>>(),
        );
        self.conflicts.set_operation(&snapshot.status.operation);
        self.conflicts_page
            .set_visible(!conflicted.is_empty() || !snapshot.status.operation.is_none());

        self.render_status();
    }

    fn render_status(&self) {
        let state = self.state.borrow();
        let status = state.snapshot.status.clone();
        drop(state);

        let branch_name = if status.detached {
            "detached HEAD".to_string()
        } else {
            status.head_name.clone().unwrap_or_else(|| "unborn".into())
        };
        self.branch_label.set_label(&branch_name);
        let mut sync = String::new();
        if status.ahead > 0 {
            sync.push_str(&format!("↑{} ", status.ahead));
        }
        if status.behind > 0 {
            sync.push_str(&format!("↓{} ", status.behind));
        }
        if let Some(upstream) = &status.upstream {
            sync.push_str(&format!("· {upstream}"));
        }
        self.sync_label.set_label(sync.trim());

        // Operation banner
        if status.operation.is_none() {
            self.op_banner.set_visible(false);
        } else {
            self.op_banner.set_visible(true);
            self.op_label
                .set_label(&util::operation_label(&status.operation));
        }
    }

    // -----------------------------------------------------------------
    // History
    // -----------------------------------------------------------------

    fn refresh_history(self: &Rc<Self>) {
        let repo = self.repo.clone();
        let (limit, search, rev) = {
            let state = self.state.borrow();
            (
                state.history_limit,
                state.history_search.clone(),
                state.history_filter_rev.clone(),
            )
        };
        let weak = Rc::downgrade(self);
        tasks::run_blocking(
            move || {
                let mut options = crate::git::LogOptions {
                    all_refs: search.is_none() && rev.is_none(),
                    limit,
                    rev: rev.clone(),
                    ..Default::default()
                };
                if let Some((query, field)) = &search {
                    match field {
                        SearchField::Message => options.grep = Some(query.clone()),
                        SearchField::Author => options.author = Some(query.clone()),
                        SearchField::Path => options.paths = vec![path_substring_pathspec(query)],
                        SearchField::Hash => {
                            options.grep = Some(query.clone());
                        }
                    }
                }
                let commits = repo.log(&options)?;
                Ok((commits, limit))
            },
            move |result: GitResult<(Vec<Commit>, usize)>| {
                let Some(page) = weak.upgrade() else { return };
                match result {
                    Ok((commits, limit)) => {
                        let has_more = commits.len() >= limit;
                        let max_lane = 0;
                        let branches = page.state.borrow().snapshot.branches.clone();
                        page.history.render(&commits, &branches, has_more, max_lane);
                        page.state.borrow_mut().history = commits;
                        page.state.borrow_mut().history_has_more = has_more;
                    }
                    Err(e) => page.error(&e),
                }
            },
        );
    }

    fn load_more_history(self: &Rc<Self>) {
        let mut state = self.state.borrow_mut();
        state.history_limit += 2000;
        let limit = state.history_limit;
        let search = state.history_search.clone();
        drop(state);
        let repo = self.repo.clone();
        let weak = Rc::downgrade(self);
        tasks::run_blocking(
            move || {
                let mut options = crate::git::LogOptions {
                    all_refs: search.is_none(),
                    limit,
                    ..Default::default()
                };
                if let Some((query, field)) = &search {
                    match field {
                        SearchField::Message => options.grep = Some(query.clone()),
                        SearchField::Author => options.author = Some(query.clone()),
                        SearchField::Path => options.paths = vec![path_substring_pathspec(query)],
                        SearchField::Hash => {}
                    }
                }
                repo.log(&options)
            },
            move |result: GitResult<Vec<Commit>>| {
                let Some(page) = weak.upgrade() else { return };
                match result {
                    Ok(commits) => {
                        let has_more = commits.len() >= limit;
                        let branches = page.state.borrow().snapshot.branches.clone();
                        page.history.render(&commits, &branches, has_more, 0);
                        page.state.borrow_mut().history = commits;
                    }
                    Err(e) => page.error(&e),
                }
            },
        );
    }

    fn select_commit(self: &Rc<Self>, oid: String) {
        let repo = self.repo.clone();
        let weak = Rc::downgrade(self);
        let oid_for_log = oid.clone();
        tasks::run_blocking(
            move || {
                let commit = repo.log_commit(&oid_for_log)?;
                let files = repo.changed_files_commit(&oid_for_log)?;
                let diffs = repo.diff_commit(&oid_for_log)?;
                Ok((commit, files, diffs))
            },
            move |result: GitResult<(Commit, Vec<crate::git::ChangedFile>, Vec<FileDiff>)>| {
                let Some(page) = weak.upgrade() else { return };
                match result {
                    Ok((commit, files, diffs)) => {
                        let branches = page.state.borrow().snapshot.branches.clone();
                        page.history.show_commit(&commit, &branches, files, diffs);
                        let _ = oid;
                    }
                    Err(e) => page.error(&e),
                }
            },
        );
    }

    fn select_diff(self: &Rc<Self>, path: PathBuf, staged: bool, rev: Option<String>) {
        let repo = self.repo.clone();
        let path_clone = path.clone();
        let weak = Rc::downgrade(self);
        tasks::run_blocking(
            move || {
                let diffs = if let Some(rev) = rev {
                    repo.diff_commit(&rev)?
                } else if staged {
                    repo.diff_index(Some(&path_clone))?
                } else {
                    repo.diff_worktree(Some(&path_clone))?
                };
                Ok(diffs)
            },
            move |result: GitResult<Vec<FileDiff>>| {
                let Some(page) = weak.upgrade() else { return };
                match result {
                    Ok(diffs) => {
                        let file = diffs
                            .iter()
                            .find(|d| d.path().map(|p| p == &path).unwrap_or(false))
                            .or_else(|| diffs.first())
                            .cloned();
                        match file {
                            Some(file) => {
                                let actionable = super::diff_view::is_actionable(&file);
                                let allow = actionable;
                                let allow_discard = actionable && !staged;
                                page.changes.show_diff(&file, allow, allow_discard);
                            }
                            None if !staged => page.show_unstaged_file_or_placeholder(&path),
                            None => {}
                        }
                    }
                    Err(e) => {
                        crate::log_warn!("diff failed: {}", e.user_message());
                        page.changes.diff_view().clear();
                        page.changes
                            .diff_view()
                            .show_placeholder("Diff unavailable", &e.user_message());
                    }
                }
            },
        );
    }

    /// When no diff is produced for an unstaged path, it is usually an
    /// untracked file: show its content as an all-added diff.
    fn show_unstaged_file_or_placeholder(self: &Rc<Self>, path: &PathBuf) {
        let untracked = self
            .state
            .borrow()
            .snapshot
            .status
            .entries
            .iter()
            .any(|e| &e.path == path && e.is_untracked());
        if untracked {
            let repo = self.repo.clone();
            let path = path.clone();
            let path_for_read = path.clone();
            let weak = Rc::downgrade(self);
            tasks::run_blocking(
                move || {
                    let content = repo.read_worktree_file(&path_for_read)?;
                    Ok(content)
                },
                move |result: GitResult<Option<String>>| {
                    let Some(page) = weak.upgrade() else { return };
                    match result {
                        Ok(Some(content)) => {
                            let file = crate::git::synthetic_added_diff(&path, &content);
                            let actionable = !file.hunks.is_empty();
                            page.changes.show_diff(&file, actionable, false);
                        }
                        Ok(None) => {
                            page.changes.diff_view().clear();
                            page.changes.diff_view().show_placeholder(
                                "Binary file",
                                "This untracked file cannot be previewed as text.",
                            );
                        }
                        Err(e) => page.error(&e),
                    }
                },
            );
            return;
        }
        self.changes.diff_view().clear();
        self.changes.diff_view().show_placeholder(
            "No changes",
            "This file has no differences in the working tree.",
        );
    }

    // -----------------------------------------------------------------
    // Request handling
    // -----------------------------------------------------------------

    fn handle_request(self: &Rc<Self>, req: UiRequest) {
        crate::log_debug!("ui request: {:?}", request_name(&req));
        match req {
            UiRequest::RefreshAll => self.refresh_all(),
            UiRequest::ShowSection(name) => self.show_section(name),
            UiRequest::QuickPull => self.quick_pull(),
            UiRequest::QuickPush => self.quick_push(),
            UiRequest::PullOptions => self.pull_dialog(),
            UiRequest::RefreshHistory => self.refresh_history(),
            UiRequest::LoadMoreHistory => self.load_more_history(),
            UiRequest::SelectCommit(oid) => self.select_commit(oid),
            UiRequest::HistorySearch { query, field } => {
                {
                    let mut state = self.state.borrow_mut();
                    state.history_search = if query.trim().is_empty() {
                        None
                    } else {
                        Some((query.clone(), field))
                    };
                    state.history_limit = 2000;
                }
                if field == SearchField::Hash && !query.trim().is_empty() {
                    if let Ok(full) = self.repo.rev_parse(&query) {
                        self.select_commit(full);
                    }
                }
                self.refresh_history();
            }
            UiRequest::SelectDiff { path, staged, rev } => self.select_diff(path, staged, rev),

            UiRequest::StagePaths(paths) => {
                let repo = self.repo.clone();
                self.run(
                    move || repo.add_paths(&paths),
                    |page, _| {
                        page.refresh_status();
                        page.refresh_all();
                    },
                );
            }
            UiRequest::UnstagePaths(paths) => {
                let repo = self.repo.clone();
                self.run(
                    move || repo.reset_paths(&paths),
                    |page, _| page.refresh_all(),
                );
            }
            UiRequest::StageAll => {
                let repo = self.repo.clone();
                self.run(move || repo.add_all(), |page, _| page.refresh_all());
            }
            UiRequest::UnstageAll => {
                let repo = self.repo.clone();
                self.run(move || repo.reset_all(), |page, _| page.refresh_all());
            }
            UiRequest::DiscardPaths(paths) => self.confirm_discard(paths),
            UiRequest::DiscardAll => {
                let entries: Vec<StatusEntry> = self
                    .state
                    .borrow()
                    .snapshot
                    .status
                    .entries
                    .iter()
                    .filter(|e| !e.is_untracked() && !e.is_conflicted())
                    .cloned()
                    .collect();
                let paths: Vec<PathBuf> = entries.iter().map(|e| e.path.clone()).collect();
                if paths.is_empty() {
                    self.toast("Nothing to discard");
                    return;
                }
                self.confirm_discard(paths);
            }
            UiRequest::CleanUntracked => {
                let paths: Vec<PathBuf> = self
                    .state
                    .borrow()
                    .snapshot
                    .status
                    .untracked()
                    .iter()
                    .map(|e| e.path.clone())
                    .collect();
                if paths.is_empty() {
                    self.toast("No untracked files");
                    return;
                }
                let Some(window) = self.window() else { return };
                let weak = Rc::downgrade(self);
                super::confirm::confirm_dangerous(
                    &window,
                    "Delete untracked files?",
                    &format!(
                        "{} untracked file(s) will be permanently deleted.",
                        paths.len()
                    ),
                    "Untracked files are not stored in Git. This cannot be undone.",
                    "Delete Files",
                    move || {
                        if let Some(page) = weak.upgrade() {
                            let repo = page.repo.clone();
                            let paths = paths.clone();
                            page.run(
                                move || repo.remove_untracked(&paths),
                                |page, _| page.refresh_all(),
                            );
                        }
                    },
                );
            }
            UiRequest::DeleteUntracked(path) => {
                let Some(window) = self.window() else { return };
                let weak = Rc::downgrade(self);
                super::confirm::confirm_dangerous(
                    &window,
                    "Delete this file?",
                    &path.display().to_string(),
                    "Untracked files are not stored in Git. This cannot be undone.",
                    "Delete",
                    move || {
                        if let Some(page) = weak.upgrade() {
                            let repo = page.repo.clone();
                            let path = path.clone();
                            page.run(
                                move || repo.remove_untracked(&[path]),
                                |page, _| page.refresh_all(),
                            );
                        }
                    },
                );
            }
            UiRequest::AddIgnore(path) => {
                let repo = self.repo.clone();
                let path_for_ignore = path.clone();
                self.run(
                    move || {
                        let full = repo.resolve_in_workdir(&path_for_ignore)?;
                        let ignore_path = repo.workdir().join(".gitignore");
                        let mut content = std::fs::read_to_string(&ignore_path).unwrap_or_default();
                        if !content.ends_with('\n') && !content.is_empty() {
                            content.push('\n');
                        }
                        let file_name = full
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let pattern = if file_name.is_empty() {
                            path_for_ignore.display().to_string()
                        } else {
                            file_name
                        };
                        if !content.lines().any(|l| l.trim() == pattern) {
                            content.push_str(&pattern);
                            content.push('\n');
                        }
                        std::fs::write(&ignore_path, content)?;
                        Ok(())
                    },
                    |page, _| {
                        page.toast("Added to .gitignore");
                        page.refresh_all();
                    },
                );
            }
            UiRequest::RemoveIgnore(path) => {
                let repo = self.repo.clone();
                let file_name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                self.run(
                    move || {
                        let ignore_path = repo.workdir().join(".gitignore");
                        let content = std::fs::read_to_string(&ignore_path).unwrap_or_default();
                        let filtered: Vec<&str> =
                            content.lines().filter(|l| l.trim() != file_name).collect();
                        let mut new_content = filtered.join("\n");
                        if !new_content.is_empty() {
                            new_content.push('\n');
                        }
                        std::fs::write(&ignore_path, new_content)?;
                        Ok(())
                    },
                    |page, _| {
                        page.toast("Removed from .gitignore");
                        page.refresh_all();
                    },
                );
            }
            UiRequest::StageHunk { file, hunk } => {
                let Some(patch) = super::diff_view::hunk_patch(&file, hunk) else {
                    self.toast("This hunk cannot be staged individually");
                    return;
                };
                let repo = self.repo.clone();
                let path = file.path().cloned();
                self.run(
                    move || repo.apply_patch(&patch, true, false, false),
                    move |page, _| {
                        if let Some(path) = path {
                            page.refresh_all();
                            let _ = path;
                        } else {
                            page.refresh_all();
                        }
                    },
                );
            }
            UiRequest::UnstageHunk { file, hunk } => {
                let Some(patch) = super::diff_view::hunk_patch(&file, hunk) else {
                    self.toast("This hunk cannot be unstaged individually");
                    return;
                };
                let repo = self.repo.clone();
                self.run(
                    move || repo.apply_patch(&patch, true, true, false),
                    |page, _| page.refresh_all(),
                );
            }
            UiRequest::DiscardHunk { file, hunk } => {
                let Some(patch) = super::diff_view::hunk_patch(&file, hunk) else {
                    self.toast("This hunk cannot be discarded individually");
                    return;
                };
                let Some(window) = self.window() else { return };
                let weak = Rc::downgrade(self);
                super::confirm::confirm(
                    &window,
                    "Discard this hunk?",
                    "The selected lines will be restored from the last commit.",
                    "Discard Hunk",
                    true,
                    move || {
                        if let Some(page) = weak.upgrade() {
                            let repo = page.repo.clone();
                            let patch = patch.clone();
                            page.run(
                                move || repo.apply_patch(&patch, false, true, true),
                                |page, _| {
                                    page.refresh_all();
                                    page.toast("Hunk discarded");
                                },
                            );
                        }
                    },
                );
            }
            UiRequest::Commit(options) => self.do_commit(*options),
            UiRequest::OpenFile(path) => {
                let full = self
                    .repo
                    .resolve_in_workdir(&path)
                    .unwrap_or_else(|_| path.clone());
                if let Err(e) = util::open_path(&full) {
                    self.toast(&format!("Could not open file: {e}"));
                }
            }
            UiRequest::OpenFolder(path) => {
                let full = self.repo.resolve_in_workdir(&path).unwrap_or(path);
                let folder = if full.is_dir() {
                    full
                } else {
                    full.parent().map(|p| p.to_path_buf()).unwrap_or_default()
                };
                if let Err(e) = util::open_path(&folder) {
                    self.toast(&format!("Could not open folder: {e}"));
                }
            }
            UiRequest::CopyPath(text) => {
                if let Some(window) = self.window() {
                    window.clipboard().set_text(&text);
                    self.toast("Path copied");
                }
            }
            UiRequest::CopyText(text) => {
                if let Some(window) = self.window() {
                    window.clipboard().set_text(&text);
                    self.toast("Copied to clipboard");
                }
            }
            UiRequest::OpenContainingFolder => {
                if let Err(e) = util::open_containing_folder(&self.repo_path) {
                    self.toast(&format!("Could not open folder: {e}"));
                }
            }

            // ---------------- branch actions ----------------
            UiRequest::CreateBranch => self.create_branch_dialog(),
            UiRequest::CheckoutBranch(name) => {
                let repo = self.repo.clone();
                self.run(
                    move || repo.checkout_branch(&name),
                    |page, _| {
                        page.toast("Branch switched");
                        page.refresh_all();
                    },
                );
            }
            UiRequest::CheckoutRemoteBranch { remote, branch } => {
                let repo = self.repo.clone();
                self.run(
                    move || {
                        repo.checkout_new_branch(&branch, Some(&format!("{remote}/{branch}")))?;
                        repo.set_upstream(&branch, &format!("{remote}/{branch}"))
                    },
                    |page, _| {
                        page.toast("Branch checked out");
                        page.refresh_all();
                    },
                );
            }
            UiRequest::DeleteBranch(name) => self.delete_branch_flow(name),
            UiRequest::DeleteRemoteBranch { remote, branch } => {
                let Some(window) = self.window() else { return };
                let weak = Rc::downgrade(self);
                super::confirm::confirm_dangerous(
                    &window,
                    "Delete remote branch?",
                    &format!("{remote}/{branch} will be deleted on the remote."),
                    "Other people may be using this branch.",
                    "Delete Remote Branch",
                    move || {
                        if let Some(page) = weak.upgrade() {
                            let repo = page.repo.clone();
                            let remote = remote.clone();
                            let branch = branch.clone();
                            page.run_streaming(
                                &format!("Deleting {remote}/{branch}"),
                                move |cancel, progress| {
                                    repo.push_delete_branch(
                                        &remote,
                                        &format!("refs/heads/{branch}"),
                                        Some(cancel),
                                        progress,
                                    )
                                },
                                |page, result| {
                                    if result.is_ok() {
                                        page.toast("Remote branch deleted");
                                    } else if let Err(e) = result {
                                        page.error(&e);
                                    }
                                    page.refresh_all();
                                },
                            );
                        }
                    },
                );
            }
            UiRequest::RenameBranch(name) => {
                let Some(window) = self.window() else { return };
                let weak = Rc::downgrade(self);
                let current = name.clone();
                super::confirm::prompt(
                    &window,
                    "Rename branch",
                    &name,
                    "Rename",
                    move |new_name| {
                        if new_name.trim().is_empty() || new_name == current {
                            return;
                        }
                        if let Some(page) = weak.upgrade() {
                            let repo = page.repo.clone();
                            let old = current.clone();
                            let new = new_name.trim().to_string();
                            page.run(
                                move || repo.rename_branch(&old, &new, false),
                                |page, _| {
                                    page.toast("Branch renamed");
                                    page.refresh_all();
                                },
                            );
                        }
                    },
                );
            }
            UiRequest::MergeBranch(name) => self.confirm_merge(name),
            UiRequest::RebaseOnto(name) => self.confirm_rebase(name),
            UiRequest::IntegrateBranches {
                source,
                source_tip,
                target,
                target_tip,
            } => {
                let find = |reference: &str, tip: &str| {
                    self.state
                        .borrow()
                        .snapshot
                        .branches
                        .iter()
                        .find(|branch| {
                            !branch.is_remote && branch.full_ref == reference && branch.tip == tip
                        })
                        .cloned()
                };
                match (find(&source, &source_tip), find(&target, &target_tip)) {
                    (Some(source), Some(target)) if source.full_ref != target.full_ref => {
                        self.integrate_branches_dialog(Some((source, target)), None);
                    }
                    _ => {
                        self.toast("Branch tips changed. Refresh History and choose them again.");
                        self.refresh_all();
                    }
                }
            }
            UiRequest::PickBranchIntegration { source } => {
                self.integrate_branches_dialog(None, source.as_deref());
            }
            UiRequest::SetUpstream(name) => {
                let repo = self.repo.clone();
                let current = self
                    .state
                    .borrow()
                    .snapshot
                    .status
                    .head_name
                    .clone()
                    .unwrap_or_default();
                self.run(
                    move || repo.set_upstream(&current, &name),
                    |page, _| {
                        page.toast("Upstream set");
                        page.refresh_all();
                    },
                );
            }
            UiRequest::CompareWithBranch(reference) => {
                let target = self
                    .state
                    .borrow()
                    .snapshot
                    .branches
                    .iter()
                    .find(|branch| branch.is_head)
                    .map(|branch| branch.full_ref.clone())
                    .unwrap_or_else(|| "HEAD".into());
                if reference != target {
                    self.show_compare(&reference, &target);
                }
            }
            UiRequest::CompareBranches => self.compare_branches_dialog(),
            UiRequest::CompareCommits => self.compare_commits_pick(),
            UiRequest::PushBranch(name) => {
                self.push_dialog(Some(name));
            }
            UiRequest::CreateBranchAt(oid) => self.create_branch_at_dialog(&oid),
            UiRequest::CheckoutDetached(oid) => {
                let short = util::short_oid(&oid).to_string();
                let repo = self.repo.clone();
                self.run(
                    move || repo.checkout_detached(&oid),
                    move |page, _| {
                        page.toast(&format!("Checked out {short} (detached HEAD)"));
                        page.refresh_all();
                    },
                );
            }

            // ---------------- history actions ----------------
            UiRequest::CherryPick(oid) => {
                let short = util::short_oid(&oid).to_string();
                let repo = self.repo.clone();
                self.run(
                    move || repo.cherry_pick(&oid),
                    move |page, outcome| {
                        match outcome {
                            MergeOutcome::Conflicts(_) => {
                                page.toast("Cherry-pick has conflicts");
                                page.view_stack.set_visible_child_name("conflicts");
                            }
                            _ => page.toast(&format!("Cherry-picked {short}")),
                        }
                        page.refresh_all();
                    },
                );
            }
            UiRequest::Revert(oid) => self.confirm_revert(oid),
            UiRequest::ResetTo(oid) => self.reset_dialog(oid),
            UiRequest::CompareWithWorktree(oid) => self.show_worktree_compare(oid),
            UiRequest::CreateTagAt(oid) => self.create_tag_dialog(Some(oid)),
            UiRequest::ShowTagInHistory(name) => {
                let repo = self.repo.clone();
                let weak = Rc::downgrade(self);
                tasks::run_blocking(
                    move || repo.rev_parse(&name),
                    move |result: GitResult<String>| {
                        if let Some(page) = weak.upgrade() {
                            match result {
                                Ok(oid) => {
                                    page.view_stack.set_visible_child_name("history");
                                    page.history.select_commit(&oid);
                                    page.select_commit(oid);
                                }
                                Err(e) => page.error(&e),
                            }
                        }
                    },
                );
            }

            // ---------------- tags ----------------
            UiRequest::CreateTag => self.create_tag_dialog(None),
            UiRequest::DeleteTag(name) => {
                let Some(window) = self.window() else { return };
                let weak = Rc::downgrade(self);
                super::confirm::confirm(
                    &window,
                    "Delete tag?",
                    &format!("The tag `{name}` will be deleted locally."),
                    "Delete Tag",
                    true,
                    move || {
                        if let Some(page) = weak.upgrade() {
                            let repo = page.repo.clone();
                            let name = name.clone();
                            page.run(
                                move || repo.delete_tag(&name),
                                |page, _| {
                                    page.toast("Tag deleted");
                                    page.refresh_all();
                                },
                            );
                        }
                    },
                );
            }
            UiRequest::PushTag(name) => self.push_tag_dialog(name),
            UiRequest::DeleteRemoteTag(name) => self.delete_remote_tag_dialog(name),
            UiRequest::CheckoutTag(name) => {
                let repo = self.repo.clone();
                let name_for_msg = name.clone();
                self.run(
                    move || repo.checkout_detached(&name),
                    move |page, _| {
                        page.toast(&format!("Checked out tag {name_for_msg} (detached HEAD)"));
                        page.refresh_all();
                    },
                );
            }

            // ---------------- stashes ----------------
            UiRequest::CreateStash => self.create_stash_dialog(),
            UiRequest::ShowStash(index) => self.show_stash_diff(index),
            UiRequest::ApplyStash(index) => {
                let repo = self.repo.clone();
                self.run(
                    move || repo.stash_apply(index, false),
                    move |page, outcome| {
                        if let MergeOutcome::Conflicts(_) = outcome {
                            page.toast("Stash applied with conflicts");
                        } else {
                            page.toast("Stash applied");
                        }
                        page.refresh_all();
                    },
                );
            }
            UiRequest::ApplyStashIndex(index) => {
                let repo = self.repo.clone();
                self.run(
                    move || repo.stash_apply(index, true),
                    move |page, outcome| {
                        if let MergeOutcome::Conflicts(_) = outcome {
                            page.toast("Stash applied with conflicts");
                        } else {
                            page.toast("Stash applied and index restored");
                        }
                        page.refresh_all();
                    },
                );
            }
            UiRequest::PopStash(index) => {
                let repo = self.repo.clone();
                self.run(
                    move || repo.stash_pop(index, false),
                    move |page, outcome| {
                        if let MergeOutcome::Conflicts(_) = outcome {
                            page.toast("Stash popped with conflicts");
                        } else {
                            page.toast("Stash popped");
                        }
                        page.refresh_all();
                    },
                );
            }
            UiRequest::DropStash(index) => {
                let Some(window) = self.window() else { return };
                let weak = Rc::downgrade(self);
                super::confirm::confirm_dangerous(
                    &window,
                    "Drop stash?",
                    &format!("stash@{{{index}}} will be permanently deleted."),
                    "The stashed changes will no longer be recoverable.",
                    "Drop Stash",
                    move || {
                        if let Some(page) = weak.upgrade() {
                            let repo = page.repo.clone();
                            page.run(
                                move || repo.stash_drop(index),
                                |page, _| {
                                    page.toast("Stash dropped");
                                    page.refresh_all();
                                },
                            );
                        }
                    },
                );
            }
            UiRequest::BranchFromStash(index) => {
                let Some(window) = self.window() else { return };
                let weak = Rc::downgrade(self);
                super::confirm::prompt(
                    &window,
                    "Create branch from stash",
                    "new-branch-name",
                    "Create Branch",
                    move |name| {
                        let name = name.trim().to_string();
                        if name.is_empty() {
                            return;
                        }
                        if let Some(page) = weak.upgrade() {
                            let repo = page.repo.clone();
                            page.run(
                                move || repo.stash_branch(index, &name),
                                |page, _| {
                                    page.toast("Branch created from stash");
                                    page.refresh_all();
                                },
                            );
                        }
                    },
                );
            }

            // ---------------- remotes ----------------
            UiRequest::AddRemote => self.remote_dialog(None),
            UiRequest::EditRemote(name) => self.remote_dialog(Some(name)),
            UiRequest::RenameRemote(name) => {
                let Some(window) = self.window() else { return };
                let weak = Rc::downgrade(self);
                let old = name.clone();
                super::confirm::prompt(
                    &window,
                    "Rename remote",
                    &name,
                    "Rename",
                    move |new_name| {
                        let new_name = new_name.trim().to_string();
                        if new_name.is_empty() || new_name == old {
                            return;
                        }
                        if let Some(page) = weak.upgrade() {
                            let repo = page.repo.clone();
                            let old = old.clone();
                            page.run(
                                move || repo.rename_remote(&old, &new_name),
                                |page, _| {
                                    page.toast("Remote renamed");
                                    page.refresh_all();
                                },
                            );
                        }
                    },
                );
            }
            UiRequest::RemoveRemote(name) => {
                let Some(window) = self.window() else { return };
                let weak = Rc::downgrade(self);
                super::confirm::confirm(
                    &window,
                    "Remove remote?",
                    &format!(
                        "The remote `{name}` and its remote-tracking branches will be removed."
                    ),
                    "Remove Remote",
                    true,
                    move || {
                        if let Some(page) = weak.upgrade() {
                            let repo = page.repo.clone();
                            let name = name.clone();
                            page.run(
                                move || repo.remove_remote(&name),
                                |page, _| {
                                    page.toast("Remote removed");
                                    page.refresh_all();
                                },
                            );
                        }
                    },
                );
            }
            UiRequest::FetchRemote(name) => self.fetch(Some(name), false),
            UiRequest::FetchAll => self.fetch_dialog(),
            UiRequest::PushTo(remote) => self.push_dialog_for_remote(Some(remote)),
            UiRequest::PullFrom(remote) => self.pull_dialog_for_remote(Some(remote)),
            UiRequest::PushAllTags => self.push_all_tags(None),
            UiRequest::PushAllTagsTo(remote) => self.push_all_tags(Some(remote)),

            // ---------------- conflicts ----------------
            UiRequest::SelectConflict(path) => self.select_conflict(path),
            UiRequest::ResolveConflict(path) => {
                self.view_stack.set_visible_child_name("conflicts");
                self.select_conflict(path);
            }
            UiRequest::AcceptConflict {
                path,
                region,
                choice,
            } => self.accept_conflict(path, Some((region, choice))),
            UiRequest::AcceptAllConflicts { path, choice } => {
                self.accept_conflict(path, Some((usize::MAX, choice)));
            }
            UiRequest::ResolveConflictSide { path, ours } => {
                let repo = self.repo.clone();
                let side = if ours { "--ours" } else { "--theirs" };
                let p = path.clone();
                self.run(
                    move || {
                        repo.command()
                            .args(["checkout", side, "--"])
                            .arg(p.as_os_str())
                            .run()?;
                        repo.add_paths(&[p.clone()])
                    },
                    move |page, _| {
                        page.toast("Conflict resolved");
                        page.refresh_all();
                    },
                );
            }
            UiRequest::MarkResolved(path) => {
                let repo = self.repo.clone();
                let paths = vec![path.clone()];
                self.run(
                    move || repo.add_paths(&paths),
                    |page, _| {
                        page.toast("Marked as resolved");
                        page.refresh_all();
                    },
                );
            }
            UiRequest::ContinueOperation => self.continue_operation(),
            UiRequest::SkipOperation => self.skip_operation(),
            UiRequest::AbortOperation => self.abort_operation(),

            UiRequest::RepoPreferences => self.repo_preferences_dialog(),
        }
    }

    // -----------------------------------------------------------------
    // Operations
    // -----------------------------------------------------------------

    fn do_commit(self: &Rc<Self>, options: CommitOptions) {
        let repo = self.repo.clone();
        let settings_weak = Rc::downgrade(self);
        let message_for_recents = options.message.clone();
        let gpg_requested = options.gpg_sign.is_some();
        self.run(
            move || {
                if gpg_requested && !repo.signing_configured() {
                    // Let git decide; it may use its default key.
                    crate::log_debug!("commit signing requested without configured key");
                }
                repo.create_commit(&options)
            },
            move |page, _oid| {
                let _ = settings_weak;
                page.changes.commit_panel().clear_message();
                page.changes.commit_panel().set_amend(false);
                let mut settings = page.settings.borrow_mut();
                settings.remember_commit_message(&message_for_recents);
                let _ = settings.save();
                drop(settings);
                page.toast("Commit created");
                page.refresh_all();
            },
        );
    }

    fn confirm_discard(self: &Rc<Self>, paths: Vec<PathBuf>) {
        let Some(window) = self.window() else { return };
        let weak = Rc::downgrade(self);
        let count = paths.len();
        let tracked: Vec<PathBuf> = {
            let state = self.state.borrow();
            let mut tracked: Vec<PathBuf> = Vec::new();
            for path in &paths {
                if let Some(entry) = state
                    .snapshot
                    .status
                    .entries
                    .iter()
                    .find(|e| &e.path == path)
                {
                    if entry.is_untracked() || entry.is_conflicted() {
                        continue;
                    }
                    if !tracked.contains(path) {
                        tracked.push(path.clone());
                    }
                    // Renames must list both sides so the restore can undo
                    // the rename on both the index and the worktree.
                    if let Some(orig) = &entry.orig_path {
                        if !tracked.contains(orig) {
                            tracked.push(orig.clone());
                        }
                    }
                }
            }
            tracked
        };
        let untracked: Vec<PathBuf> = paths
            .iter()
            .filter(|p| !tracked.contains(p))
            .cloned()
            .collect();
        let weak_for_confirm = weak.clone();
        super::confirm::confirm_dangerous(
            &window,
            "Discard changes?",
            &format!("Changes in {count} file(s) will be permanently lost."),
            "Staged changes are unstaged first. Modified files are restored from the last commit. This cannot be undone.",
            "Discard Changes",
            move || {
                let Some(page) = weak_for_confirm.upgrade() else { return };
                let repo = page.repo.clone();
                let tracked = tracked.clone();
                let untracked = untracked.clone();
                page.run(
                    move || {
                        if !tracked.is_empty() {
                            repo.discard_paths(&tracked)?;
                        }
                        if !untracked.is_empty() {
                            repo.remove_untracked(&untracked)?;
                        }
                        Ok(())
                    },
                    |page, _| {
                        page.toast("Changes discarded");
                        page.refresh_all();
                    },
                );
            },
        );
    }

    fn delete_branch_flow(self: &Rc<Self>, name: String) {
        let Some(window) = self.window() else { return };
        let weak = Rc::downgrade(self);
        let name_for_flow = name.clone();
        super::confirm::confirm(
            &window,
            "Delete branch?",
            &format!("Delete the branch `{name}`?"),
            "Delete Branch",
            true,
            move || {
                let Some(page) = weak.upgrade() else { return };
                let repo = page.repo.clone();
                let name_for_flow = name_for_flow.clone();
                let branch = name_for_flow.clone();
                page.run_with_error(
                    move || repo.delete_branch(&branch, false),
                    |page, _| {
                        page.toast("Branch deleted");
                        page.refresh_all();
                    },
                    move |page, error| {
                        let not_merged = error
                            .technical_details()
                            .contains("not fully merged");
                        if !not_merged {
                            page.error(&error);
                            return;
                        }
                        let Some(window) = page.window() else { return };
                        let weak2 = Rc::downgrade(&page);
                        let name = name_for_flow.clone();
                        super::confirm::confirm_dangerous(
                            &window,
                            "Branch is not fully merged",
                            &format!(
                                "The branch `{name}` contains commits that are not reachable from anywhere else."
                            ),
                            "Deleting it will permanently lose those commits (they may remain in the reflog for a while).",
                            "Delete Anyway",
                            move || {
                                if let Some(page) = weak2.upgrade() {
                                    let repo = page.repo.clone();
                                    let branch = name.clone();
                                    page.run(
                                        move || repo.delete_branch(&branch, true),
                                        |page, _| {
                                            page.toast("Branch deleted");
                                            page.refresh_all();
                                        },
                                    );
                                }
                            },
                        );
                    },
                );
            },
        );
    }

    fn confirm_merge(self: &Rc<Self>, name: String) {
        let Some(window) = self.window() else { return };
        let branch = self
            .state
            .borrow()
            .snapshot
            .status
            .head_name
            .clone()
            .unwrap_or_else(|| "HEAD".into());
        let weak = Rc::downgrade(self);
        let label = comparison_ref_label(&name);
        let target = name;
        super::confirm::confirm(
            &window,
            "Merge branch?",
            &format!("Merge `{label}` into `{branch}`?"),
            "Merge",
            false,
            move || {
                if let Some(page) = weak.upgrade() {
                    let repo = page.repo.clone();
                    let target = target.clone();
                    page.run(
                        move || repo.merge(&target, false, false, None),
                        move |page, outcome| match outcome {
                            MergeOutcome::Conflicts(_) => {
                                page.toast("Merge conflicts need resolution");
                                page.view_stack.set_visible_child_name("conflicts");
                                page.refresh_all();
                            }
                            MergeOutcome::UpToDate => page.toast("Already up to date"),
                            MergeOutcome::FastForward => {
                                page.toast("Fast-forward merge");
                                page.refresh_all();
                            }
                            MergeOutcome::Merged => {
                                page.toast("Merge commit created");
                                page.refresh_all();
                            }
                        },
                    );
                }
            },
        );
    }

    fn confirm_rebase(self: &Rc<Self>, name: String) {
        let Some(window) = self.window() else { return };
        let branch = self
            .state
            .borrow()
            .snapshot
            .status
            .head_name
            .clone()
            .unwrap_or_else(|| "HEAD".into());
        let weak = Rc::downgrade(self);
        let label = comparison_ref_label(&name);
        let upstream = name;
        super::confirm::confirm(
            &window,
            "Rebase branch?",
            &format!(
                "Reapply the commits of `{branch}` on top of `{label}`?\n\nCommits will be rewritten; conflicts may need resolution."
            ),
            "Rebase",
            false,
            move || {
                if let Some(page) = weak.upgrade() {
                    let repo = page.repo.clone();
                    let upstream = upstream.clone();
                    page.run(
                        move || repo.rebase(Some(&upstream), None, None),
                        move |page, outcome| match outcome {
                            MergeOutcome::Conflicts(_) => {
                                page.toast("Rebase stopped with conflicts");
                                page.view_stack.set_visible_child_name("conflicts");
                                page.refresh_all();
                            }
                            _ => {
                                page.toast("Rebase complete");
                                page.refresh_all();
                            }
                        },
                    );
                }
            },
        );
    }

    fn integrate_branches_dialog(
        self: &Rc<Self>,
        dropped: Option<(Branch, Branch)>,
        source_ref: Option<&str>,
    ) {
        let Some(window) = self.window() else { return };
        let on_drop = dropped.is_some();
        let mut branches = match dropped {
            Some((source, target)) => vec![source, target],
            None => self
                .state
                .borrow()
                .snapshot
                .branches
                .iter()
                .filter(|branch| !branch.is_remote)
                .cloned()
                .collect(),
        };
        if branches.len() < 2 {
            self.toast("At least two local branches are needed to integrate");
            return;
        }

        // Drag targets are already chosen. For keyboard access default the
        // target to the checked-out branch when it differs from the source.
        let (source_index, target_index) = if on_drop {
            (0, 1)
        } else {
            branches.sort_by(|a, b| a.name.cmp(&b.name));
            let head = branches
                .iter()
                .position(|branch| branch.is_head)
                .unwrap_or(0);
            let source = if let Some(reference) = source_ref {
                let Some(index) = branches
                    .iter()
                    .position(|branch| branch.full_ref == reference)
                else {
                    self.toast("Branch changed. Refresh History and choose it again.");
                    self.refresh_all();
                    return;
                };
                index
            } else if head == 0 {
                1
            } else {
                0
            };
            let target = if source == head {
                usize::from(source == 0)
            } else {
                head
            };
            (source, target)
        };

        let dialog = adw::MessageDialog::new(
            Some(&window),
            Some("Integrate branches"),
            Some("Choose how to integrate two local branches."),
        );
        let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let names = branches
            .iter()
            .map(|branch| branch.name.clone())
            .collect::<Vec<_>>();
        let (selectors, source, target) =
            branch_selectors(&names, "Source", "Target", source_index, target_index);
        if !on_drop {
            content.append(&selectors);
        }
        let merge = gtk::CheckButton::with_label("Merge into target");
        let rebase = gtk::CheckButton::with_label("Rebase onto target");
        rebase.set_group(Some(&merge));
        merge.set_active(true);
        let explanation = gtk::Label::builder().xalign(0.0).wrap(true).build();
        explanation.add_css_class("dim-label");
        content.append(&merge);
        content.append(&rebase);
        content.append(&explanation);

        dialog.set_extra_child(Some(&content));
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("ok", "Continue");
        dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("ok"));
        dialog.set_close_response("cancel");

        let update = {
            let dialog = dialog.downgrade();
            let merge = merge.downgrade();
            let explanation = explanation.downgrade();
            let names = names.clone();
            Rc::new(move |source: &gtk::DropDown, target: &gtk::DropDown| {
                let (Some(dialog), Some(merge), Some(explanation)) =
                    (dialog.upgrade(), merge.upgrade(), explanation.upgrade())
                else {
                    return;
                };
                let pair = names
                    .get(source.selected() as usize)
                    .zip(names.get(target.selected() as usize));
                let Some((source, target)) = pair.filter(|(source, target)| source != target)
                else {
                    dialog.set_response_enabled("ok", false);
                    explanation.set_label("Choose two different local branches.");
                    return;
                };
                dialog.set_response_enabled("ok", true);
                let text = if merge.is_active() {
                    format!("Switch to {target} and merge {source} into it.")
                } else {
                    format!(
                        "Switch to {source} and rebase it onto {target}. This rewrites commits."
                    )
                };
                explanation.set_label(&text);
            })
        };
        update(&source, &target);
        let target_for_source = target.downgrade();
        let update_source = update.clone();
        source.connect_selected_notify(move |source| {
            if let Some(target) = target_for_source.upgrade() {
                update_source(source, &target);
            }
        });
        let source_for_target = source.downgrade();
        let update_target = update.clone();
        target.connect_selected_notify(move |target| {
            if let Some(source) = source_for_target.upgrade() {
                update_target(&source, target);
            }
        });
        let source_for_mode = source.downgrade();
        let target_for_mode = target.downgrade();
        merge.connect_toggled(move |_| {
            if let (Some(source), Some(target)) =
                (source_for_mode.upgrade(), target_for_mode.upgrade())
            {
                update(&source, &target);
            }
        });

        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            if response != "ok" || source.selected() == target.selected() {
                return;
            }
            let Some(page) = weak.upgrade() else { return };
            let pair = branches
                .get(source.selected() as usize)
                .zip(branches.get(target.selected() as usize));
            if let Some((source, target)) = pair {
                let action = if merge.is_active() {
                    BranchIntegration::Merge
                } else {
                    BranchIntegration::Rebase
                };
                page.perform_branch_integration(source.clone(), target.clone(), action);
            }
        });
        dialog.present();
    }

    fn perform_branch_integration(
        self: &Rc<Self>,
        source: Branch,
        target: Branch,
        action: BranchIntegration,
    ) {
        {
            let mut state = self.state.borrow_mut();
            if state.active_op.is_some() || state.integrating_branches {
                drop(state);
                self.toast("Another operation is already running");
                return;
            }
            state.integrating_branches = true;
        }

        let repo = self.repo.clone();
        let source_name = source.name.clone();
        let target_name = target.name.clone();
        self.run_with_error(
            move || repo.integrate_branches(&source, &target, action),
            move |page, outcome| {
                page.state.borrow_mut().integrating_branches = false;
                match outcome {
                    MergeOutcome::Conflicts(_) => {
                        let message = match action {
                            BranchIntegration::Merge => "Merge stopped with conflicts",
                            BranchIntegration::Rebase => "Rebase stopped with conflicts",
                        };
                        page.toast(message);
                        page.conflicts_page.set_visible(true);
                        page.view_stack.set_visible_child_name("conflicts");
                    }
                    MergeOutcome::UpToDate => page.toast("Already up to date"),
                    MergeOutcome::FastForward => {
                        page.toast(&format!("Fast-forwarded {target_name} to {source_name}"));
                    }
                    MergeOutcome::Merged => {
                        let message = match action {
                            BranchIntegration::Merge => {
                                format!("Merged {source_name} into {target_name}")
                            }
                            BranchIntegration::Rebase => {
                                format!("Rebased {source_name} onto {target_name}")
                            }
                        };
                        page.toast(&message);
                    }
                }
                page.refresh_all();
            },
            |page, error| {
                page.state.borrow_mut().integrating_branches = false;
                page.error(&error);
                page.refresh_all();
            },
        );
    }

    fn confirm_revert(self: &Rc<Self>, oid: String) {
        let Some(window) = self.window() else { return };
        let weak = Rc::downgrade(self);
        let short = util::short_oid(&oid).to_string();
        let is_merge = self
            .state
            .borrow()
            .history
            .iter()
            .find(|c| c.id == oid)
            .map(|c| c.is_merge())
            .unwrap_or(false);
        let body = if is_merge {
            format!("Revert merge commit {short}? The changes relative to its first parent will be undone.")
        } else {
            format!("Create a new commit that undoes {short}?")
        };
        super::confirm::confirm(
            &window,
            "Revert commit?",
            &body,
            "Revert",
            false,
            move || {
                if let Some(page) = weak.upgrade() {
                    let repo = page.repo.clone();
                    let oid = oid.clone();
                    page.run(
                        move || repo.revert(&oid, if is_merge { Some(1) } else { None }),
                        move |page, outcome| {
                            match outcome {
                                MergeOutcome::Conflicts(_) => {
                                    page.toast("Revert has conflicts");
                                    page.view_stack.set_visible_child_name("conflicts");
                                }
                                _ => page.toast("Revert commit created"),
                            }
                            page.refresh_all();
                        },
                    );
                }
            },
        );
    }

    fn reset_dialog(self: &Rc<Self>, oid: String) {
        let Some(window) = self.window() else { return };
        let short = util::short_oid(&oid).to_string();
        let dialog = adw::MessageDialog::new(
            Some(&window),
            Some("Reset current branch"),
            Some(&format!(
                "Move the current branch to {short}. Choose what happens to the changes that are left behind."
            )),
        );
        let modes = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let group = gtk::CheckButton::new();
        group.set_visible(false);
        let mut buttons = Vec::new();
        for (index, mode) in [
            ResetMode::Soft,
            ResetMode::Mixed,
            ResetMode::Keep,
            ResetMode::Hard,
        ]
        .iter()
        .enumerate()
        {
            let button = gtk::CheckButton::with_label(mode.label());
            button.set_group(Some(&group));
            if index == 0 {
                button.set_active(true);
            }
            modes.append(&button);
            buttons.push((button, *mode));
        }
        dialog.set_extra_child(Some(&modes));
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("ok", "Reset");
        dialog.set_response_appearance("ok", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let weak = Rc::downgrade(self);
        let buttons = Rc::new(buttons);
        dialog.connect_response(None, move |_, response| {
            if response != "ok" {
                return;
            }
            let Some(page) = weak.upgrade() else { return };
            let mode = buttons
                .iter()
                .find(|(b, _)| b.is_active())
                .map(|(_, m)| *m)
                .unwrap_or(ResetMode::Mixed);
            let oid = oid.clone();
            if mode == ResetMode::Hard {
                let Some(window) = page.window() else { return };
                let weak2 = Rc::downgrade(&page);
                super::confirm::confirm_dangerous(
                    &window,
                    "Hard reset?",
                    &format!(
                        "The branch will move to {short} and all uncommitted changes will be lost."
                    ),
                    "Hard reset discards staged and unstaged changes. This cannot be undone.",
                    "Hard Reset",
                    move || {
                        if let Some(page) = weak2.upgrade() {
                            page.perform_reset(oid.clone(), ResetMode::Hard);
                        }
                    },
                );
            } else {
                page.perform_reset(oid, mode);
            }
        });
        dialog.present();
    }

    fn perform_reset(self: &Rc<Self>, oid: String, mode: ResetMode) {
        let repo = self.repo.clone();
        self.run(
            move || repo.reset(mode, &oid),
            move |page, _| {
                page.toast("Branch reset");
                page.refresh_all();
            },
        );
    }

    fn accept_conflict(self: &Rc<Self>, path: PathBuf, choice: Option<(usize, ConflictChoice)>) {
        let repo = self.repo.clone();
        let path_clone = path.clone();
        let weak = Rc::downgrade(self);
        tasks::run_blocking(
            move || {
                let text = repo.read_worktree_file(&path_clone)?.ok_or_else(|| {
                    GitError::InvalidState {
                        message: "The conflicted file is binary or unreadable.".into(),
                        recovery: Some("Use \"Keep Current\" or \"Keep Incoming\" instead.".into()),
                    }
                })?;
                let regions = crate::git::parse_conflict_regions(&text);
                if regions.is_empty() {
                    return Err(GitError::InvalidState {
                        message: "No conflict markers found in this file.".into(),
                        recovery: Some(
                            "The file may already be resolved; mark it as resolved instead.".into(),
                        ),
                    });
                }
                let new_text = match choice {
                    None => text,
                    Some((region_index, choice)) => {
                        if region_index == usize::MAX {
                            crate::git::resolve_conflicts(&text, &regions, &|_| choice)
                        } else {
                            crate::git::resolve_conflicts(&text, &regions, &|r| {
                                let index = regions.iter().position(|x| x == r).unwrap_or(0);
                                if index == region_index {
                                    choice
                                } else {
                                    ConflictChoice::Ours
                                }
                            })
                        }
                    }
                };
                repo.write_worktree_file(&path_clone, &new_text)?;
                Ok(new_text)
            },
            move |result: GitResult<String>| {
                let Some(page) = weak.upgrade() else { return };
                match result {
                    Ok(_text) => {
                        page.select_conflict(path.clone());
                        page.refresh_status();
                    }
                    Err(e) => page.error(&e),
                }
            },
        );
    }

    fn select_conflict(self: &Rc<Self>, path: PathBuf) {
        let repo = self.repo.clone();
        let weak = Rc::downgrade(self);
        let path_for_work = path.clone();
        tasks::run_blocking(
            move || {
                let text = repo.read_worktree_file(&path_for_work)?;
                Ok((path_for_work.clone(), text))
            },
            move |result: GitResult<(PathBuf, Option<String>)>| {
                let Some(page) = weak.upgrade() else { return };
                match result {
                    Ok((path, Some(text))) => {
                        let regions = crate::git::parse_conflict_regions(&text);
                        page.conflicts.show_file(&path, &regions);
                    }
                    Ok((_path, None)) => {
                        // Binary or unreadable: show the side-choice buttons.
                        page.conflicts.show_file(&path, &[]);
                    }
                    Err(e) => page.error(&e),
                }
            },
        );
    }

    fn continue_operation(self: &Rc<Self>) {
        let repo = self.repo.clone();
        let operation = self.state.borrow().snapshot.status.operation.clone();
        self.run(
            move || match &operation {
                OperationState::Merge { .. } => repo.merge_continue(),
                OperationState::Rebase { .. } => repo.rebase_continue(),
                OperationState::CherryPick { .. } => repo.cherry_pick_continue(),
                OperationState::Revert { .. } => repo.revert_continue(),
                OperationState::Bisect => Err(GitError::InvalidState {
                    message: "Bisect continuation is not supported in the app.".into(),
                    recovery: Some("Use `git bisect` in a terminal.".into()),
                }),
                OperationState::None => Ok(MergeOutcome::UpToDate),
            },
            move |page, outcome| match outcome {
                MergeOutcome::Conflicts(_) => {
                    page.toast("Still conflicts to resolve");
                    page.refresh_all();
                }
                _ => {
                    page.toast("Operation completed");
                    page.refresh_all();
                }
            },
        );
    }

    fn skip_operation(self: &Rc<Self>) {
        let repo = self.repo.clone();
        let operation = self.state.borrow().snapshot.status.operation.clone();
        self.run(
            move || match &operation {
                OperationState::Rebase { .. } => repo.rebase_skip(),
                OperationState::CherryPick { .. } => repo
                    .command()
                    .args(["cherry-pick", "--skip"])
                    .run()
                    .map(|_| MergeOutcome::Merged),
                OperationState::Revert { .. } => repo
                    .command()
                    .args(["revert", "--skip"])
                    .run()
                    .map(|_| MergeOutcome::Merged),
                _ => Err(GitError::InvalidState {
                    message: "Skipping is not supported for this operation.".into(),
                    recovery: None,
                }),
            },
            |page, _| {
                page.toast("Commit skipped");
                page.refresh_all();
            },
        );
    }

    fn abort_operation(self: &Rc<Self>) {
        let Some(window) = self.window() else { return };
        let operation = self.state.borrow().snapshot.status.operation.clone();
        let label = util::operation_label(&operation);
        let weak = Rc::downgrade(self);
        super::confirm::confirm_dangerous(
            &window,
            "Abort operation?",
            &format!("{label}\n\nThe operation will be rolled back."),
            "Any conflict resolutions you made since the operation started will be lost.",
            "Abort",
            move || {
                if let Some(page) = weak.upgrade() {
                    let repo = page.repo.clone();
                    let operation = operation.clone();
                    page.run(
                        move || match &operation {
                            OperationState::Merge { .. } => repo.merge_abort(),
                            OperationState::Rebase { .. } => repo.rebase_abort(),
                            OperationState::CherryPick { .. } => repo.cherry_pick_abort(),
                            OperationState::Revert { .. } => repo.revert_abort(),
                            OperationState::Bisect => {
                                repo.command().args(["bisect", "reset"]).run().map(|_| ())
                            }
                            OperationState::None => Ok(()),
                        },
                        |page, _| {
                            page.toast("Operation aborted");
                            page.refresh_all();
                        },
                    );
                }
            },
        );
    }

    // -----------------------------------------------------------------
    // Network operations
    // -----------------------------------------------------------------

    fn fetch(self: &Rc<Self>, remote: Option<String>, tags: bool) {
        let repo = self.repo.clone();
        let prune = self.settings.borrow().fetch_prune;
        let label = match &remote {
            Some(r) => format!("Fetching {r}…"),
            None => "Fetching all remotes…".to_string(),
        };
        self.run_streaming(
            &label,
            move |cancel, progress| {
                repo.fetch(remote.as_deref(), prune, tags, Some(cancel), progress)
            },
            |page, result| match result {
                Ok(()) => {
                    page.toast("Fetch complete");
                    page.refresh_all();
                }
                Err(GitError::Cancelled) => {}
                Err(e) => {
                    page.error(&e);
                    page.refresh_all();
                }
            },
        );
    }

    /// One-click push to the configured upstream. Falls back to the push
    /// dialog when the branch has no upstream yet (publishing).
    /// Direct fetch without a dialog: fetches the upstream remote when known,
    /// otherwise all remotes. Options are still available from the Remotes tab.
    pub fn quick_fetch(self: &Rc<Self>) {
        let upstream = self
            .state
            .borrow()
            .snapshot
            .status
            .upstream
            .clone()
            .and_then(|u| split_upstream(&u).map(|(r, _)| r));
        self.fetch_with_options(upstream, self.settings.borrow().fetch_prune, false);
    }

    pub fn quick_push(self: &Rc<Self>) {
        let status = self.state.borrow().snapshot.status.clone();
        let Some(head) = status.head_name.clone() else {
            self.toast("Detached HEAD — create a branch before pushing");
            return;
        };
        let Some(upstream) = status.upstream.clone() else {
            self.push_dialog(Some(head));
            return;
        };
        let Some((remote, remote_branch)) = split_upstream(&upstream) else {
            self.push_dialog(Some(head));
            return;
        };
        self.perform_push(
            remote,
            format!("refs/heads/{head}"),
            format!("refs/heads/{remote_branch}"),
            false,
            crate::git::ForceMode::None,
        );
    }

    /// One-click pull (fetch + merge) from the configured upstream.
    pub fn quick_pull(self: &Rc<Self>) {
        let status = self.state.borrow().snapshot.status.clone();
        if status.detached || status.head_name.is_none() {
            self.toast("Detached HEAD — check out a branch before pulling");
            return;
        }
        let Some(upstream) = status.upstream.clone() else {
            let branch = status.head_name.clone().unwrap_or_default();
            let Some(window) = self.window() else { return };
            let weak = Rc::downgrade(self);
            super::confirm::confirm(
                &window,
                "No upstream branch",
                &format!(
                    "`{branch}` has no upstream, so there is nothing to pull. Publish the branch first?"
                ),
                "Publish Branch…",
                false,
                move || {
                    if let Some(page) = weak.upgrade() {
                        page.push_dialog(Some(branch.clone()));
                    }
                },
            );
            return;
        };
        let remote = split_upstream(&upstream).map(|(r, _)| r);
        let repo = self.repo.clone();
        self.run_streaming(
            "Pulling…",
            move |cancel, progress| {
                repo.pull(remote.as_deref(), false, false, Some(cancel), progress)
            },
            |page, result| match result {
                Ok(MergeOutcome::Conflicts(_)) => {
                    page.toast("Pull resulted in conflicts");
                    page.view_stack.set_visible_child_name("conflicts");
                    page.refresh_all();
                }
                Ok(MergeOutcome::UpToDate) => {
                    page.toast("Already up to date");
                    page.refresh_all();
                }
                Ok(_) => {
                    page.toast("Pull complete");
                    page.refresh_all();
                }
                Err(GitError::Cancelled) => {}
                Err(e) => {
                    page.error(&e);
                    page.refresh_all();
                }
            },
        );
    }

    fn fetch_dialog(self: &Rc<Self>) {
        let Some(window) = self.window() else { return };
        let remotes = self.state.borrow().snapshot.remotes.clone();
        if remotes.is_empty() {
            self.toast("No remotes configured");
            return;
        }
        let dialog = adw::MessageDialog::new(
            Some(&window),
            Some("Fetch"),
            Some("Fetch updates from a remote without merging."),
        );
        let box_ = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let mut names: Vec<String> = vec!["All remotes".to_string()];
        names.extend(remotes.iter().map(|r| r.name.clone()));
        let model = gtk::StringList::new(&names.iter().map(|s| s.as_str()).collect::<Vec<_>>());
        let dropdown = gtk::DropDown::new(Some(model), gtk::Expression::NONE);
        dropdown.set_selected(0);
        let prune = gtk::CheckButton::with_label("Prune deleted remote-tracking branches");
        prune.set_active(self.settings.borrow().fetch_prune);
        let tags = gtk::CheckButton::with_label("Fetch tags");
        box_.append(&dropdown);
        box_.append(&prune);
        box_.append(&tags);
        dialog.set_extra_child(Some(&box_));
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("ok", "Fetch");
        dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("ok"));
        dialog.set_close_response("cancel");
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            if response != "ok" {
                return;
            }
            if let Some(page) = weak.upgrade() {
                let index = dropdown.selected() as usize;
                let remote = if index == 0 {
                    None
                } else {
                    names.get(index).cloned()
                };
                let prune_value = prune.is_active();
                let tags_value = tags.is_active();
                page.settings.borrow_mut().fetch_prune = prune_value;
                let _ = page.settings.borrow().save();
                page.fetch_with_options(remote, prune_value, tags_value);
            }
        });
        dialog.present();
    }

    fn fetch_with_options(self: &Rc<Self>, remote: Option<String>, prune: bool, tags: bool) {
        let repo = self.repo.clone();
        let label = match &remote {
            Some(r) => format!("Fetching {r}…"),
            None => "Fetching all remotes…".to_string(),
        };
        self.run_streaming(
            &label,
            move |cancel, progress| {
                repo.fetch(remote.as_deref(), prune, tags, Some(cancel), progress)
            },
            |page, result| {
                match result {
                    Ok(()) => page.toast("Fetch complete"),
                    Err(GitError::Cancelled) => {}
                    Err(e) => page.error(&e),
                }
                page.refresh_all();
            },
        );
    }

    fn pull_dialog(self: &Rc<Self>) {
        self.pull_dialog_for_remote(None);
    }

    fn pull_dialog_for_remote(self: &Rc<Self>, preselected: Option<String>) {
        let Some(window) = self.window() else { return };
        let remotes = self.state.borrow().snapshot.remotes.clone();
        if remotes.is_empty() {
            self.toast("No remotes configured");
            return;
        }
        let dialog = adw::MessageDialog::new(
            Some(&window),
            Some("Pull"),
            Some("Fetch and integrate the upstream branch."),
        );
        let box_ = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let names: Vec<String> = remotes.iter().map(|r| r.name.clone()).collect();
        let model = gtk::StringList::new(&names.iter().map(|s| s.as_str()).collect::<Vec<_>>());
        let dropdown = gtk::DropDown::new(Some(model), gtk::Expression::NONE);
        if let Some(pre) = &preselected {
            if let Some(index) = names.iter().position(|n| n == pre) {
                dropdown.set_selected(index as u32);
            }
        }
        let rebase = gtk::CheckButton::with_label("Rebase instead of merge");
        let ff_only = gtk::CheckButton::with_label("Fast-forward only");
        box_.append(&dropdown);
        box_.append(&rebase);
        box_.append(&ff_only);
        dialog.set_extra_child(Some(&box_));
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("ok", "Pull");
        dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("ok"));
        dialog.set_close_response("cancel");
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            if response != "ok" {
                return;
            }
            let Some(page) = weak.upgrade() else { return };
            let index = dropdown.selected() as usize;
            let remote = names.get(index).cloned();
            let rebase_value = rebase.is_active();
            let ff_value = ff_only.is_active();
            let repo = page.repo.clone();
            page.run_streaming(
                "Pulling…",
                move |cancel, progress| {
                    repo.pull(
                        remote.as_deref(),
                        rebase_value,
                        ff_value,
                        Some(cancel),
                        progress,
                    )
                },
                |page, result| match result {
                    Ok(MergeOutcome::Conflicts(_)) => {
                        page.toast("Pull resulted in conflicts");
                        page.view_stack.set_visible_child_name("conflicts");
                        page.refresh_all();
                    }
                    Ok(MergeOutcome::UpToDate) => {
                        page.toast("Already up to date");
                        page.refresh_all();
                    }
                    Ok(_) => {
                        page.toast("Pull complete");
                        page.refresh_all();
                    }
                    Err(GitError::Cancelled) => {}
                    Err(e) => {
                        page.error(&e);
                        page.refresh_all();
                    }
                },
            );
        });
        dialog.present();
    }

    fn push_dialog(self: &Rc<Self>, branch: Option<String>) {
        self.push_dialog_inner(branch, None);
    }

    fn push_dialog_for_remote(self: &Rc<Self>, remote: Option<String>) {
        self.push_dialog_inner(None, remote);
    }

    fn push_dialog_inner(
        self: &Rc<Self>,
        branch: Option<String>,
        remote_preselect: Option<String>,
    ) {
        let Some(window) = self.window() else { return };
        let (remotes, status) = {
            let state = self.state.borrow();
            (
                state.snapshot.remotes.clone(),
                state.snapshot.status.clone(),
            )
        };
        if remotes.is_empty() {
            let weak = Rc::downgrade(self);
            super::confirm::confirm(
                &window,
                "No remotes configured",
                "Add a remote before pushing.",
                "Add Remote…",
                false,
                move || {
                    if let Some(page) = weak.upgrade() {
                        page.remote_dialog(None);
                    }
                },
            );
            return;
        }
        let local_branch = branch
            .or_else(|| status.head_name.clone())
            .unwrap_or_else(|| "HEAD".into());
        let dialog = adw::MessageDialog::new(
            Some(&window),
            Some("Push"),
            Some(&format!("Push `{local_branch}` to a remote.")),
        );
        let grid = gtk::Grid::builder()
            .row_spacing(6)
            .column_spacing(8)
            .build();
        let names: Vec<String> = remotes.iter().map(|r| r.name.clone()).collect();
        let model = gtk::StringList::new(&names.iter().map(|s| s.as_str()).collect::<Vec<_>>());
        let dropdown = gtk::DropDown::new(Some(model), gtk::Expression::NONE);
        if let Some(pre) = &remote_preselect {
            if let Some(i) = names.iter().position(|n| n == pre) {
                dropdown.set_selected(i as u32);
            }
        }
        let branch_entry = gtk::Entry::builder()
            .text(&local_branch)
            .hexpand(true)
            .build();
        let remote_branch_entry = gtk::Entry::builder()
            .text(&local_branch)
            .hexpand(true)
            .build();
        let set_upstream = gtk::CheckButton::with_label("Set upstream");
        set_upstream.set_active(true);
        let force_model = gtk::StringList::new(&[
            "Normal",
            "Force with lease (safe)",
            "Force (overwrite remote)",
        ]);
        let force_dropdown = gtk::DropDown::new(Some(force_model), gtk::Expression::NONE);
        force_dropdown.set_selected(0);
        grid.attach(&gtk::Label::new(Some("Remote")), 0, 0, 1, 1);
        grid.attach(&dropdown, 1, 0, 2, 1);
        grid.attach(&gtk::Label::new(Some("Local branch")), 0, 1, 1, 1);
        grid.attach(&branch_entry, 1, 1, 2, 1);
        grid.attach(&gtk::Label::new(Some("Remote branch")), 0, 2, 1, 1);
        grid.attach(&remote_branch_entry, 1, 2, 2, 1);
        grid.attach(&set_upstream, 1, 3, 2, 1);
        grid.attach(&gtk::Label::new(Some("Force")), 0, 4, 1, 1);
        grid.attach(&force_dropdown, 1, 4, 2, 1);
        dialog.set_extra_child(Some(&grid));
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("ok", "Push");
        dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("ok"));
        dialog.set_close_response("cancel");
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            if response != "ok" {
                return;
            }
            let Some(page) = weak.upgrade() else { return };
            let remote = names
                .get(dropdown.selected() as usize)
                .cloned()
                .unwrap_or_default();
            let local = branch_entry.text().to_string();
            let remote_branch = remote_branch_entry.text().to_string();
            let upstream = set_upstream.is_active();
            let force = match force_dropdown.selected() {
                1 => crate::git::ForceMode::WithLease,
                2 => crate::git::ForceMode::Force,
                _ => crate::git::ForceMode::None,
            };
            let page_for_execute = Rc::clone(&page);
            let execute: Rc<dyn Fn()> = Rc::new(move || {
                page_for_execute.perform_push(
                    remote.clone(),
                    local.clone(),
                    remote_branch.clone(),
                    upstream,
                    force,
                )
            });
            if force != crate::git::ForceMode::None {
                let Some(window) = page.window() else { return };
                let explanation = if force == crate::git::ForceMode::Force {
                    "This may overwrite commits that others have pushed. This cannot be undone."
                        .to_string()
                } else {
                    "Force-with-lease refuses to overwrite commits you have not seen.".to_string()
                };
                let execute = Rc::clone(&execute);
                super::confirm::confirm_dangerous(
                    &window,
                    "Force push?",
                    "The remote branch history will be rewritten.",
                    &explanation,
                    "Force Push",
                    move || execute(),
                );
            } else {
                execute();
            }
        });
        dialog.present();
    }

    fn perform_push(
        self: &Rc<Self>,
        remote: String,
        local: String,
        remote_branch: String,
        set_upstream: bool,
        force: crate::git::ForceMode,
    ) {
        let Some((local_ref, remote_ref)) = push_refspecs(&local, &remote_branch) else {
            self.toast("Enter a valid local and remote branch name");
            return;
        };
        let repo = self.repo.clone();
        let label = format!("Pushing to {remote}…");
        let remote_for_msg = remote.clone();
        self.run_streaming(
            &label,
            move |cancel, progress| {
                repo.push(
                    &remote,
                    &local_ref,
                    &remote_ref,
                    set_upstream,
                    force,
                    Some(cancel),
                    progress,
                )
            },
            move |page, result| {
                match result {
                    Ok(()) => page.toast(&format!("Pushed to {remote_for_msg}")),
                    Err(GitError::Cancelled) => {}
                    Err(e) => page.error(&e),
                }
                page.refresh_all();
            },
        );
    }

    fn push_tag_dialog(self: &Rc<Self>, tag: String) {
        let Some(window) = self.window() else { return };
        let remotes = self.state.borrow().snapshot.remotes.clone();
        if remotes.is_empty() {
            self.toast("No remotes configured");
            return;
        }
        let dialog = adw::MessageDialog::new(
            Some(&window),
            Some("Push tag"),
            Some(&format!("Push `{tag}` to a remote.")),
        );
        let names: Vec<String> = remotes.iter().map(|r| r.name.clone()).collect();
        let model = gtk::StringList::new(&names.iter().map(|s| s.as_str()).collect::<Vec<_>>());
        let dropdown = gtk::DropDown::new(Some(model), gtk::Expression::NONE);
        dropdown.set_selected(0);
        dialog.set_extra_child(Some(&dropdown));
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("ok", "Push Tag");
        dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("ok"));
        dialog.set_close_response("cancel");
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            if response != "ok" {
                return;
            }
            let Some(page) = weak.upgrade() else { return };
            let remote = names
                .get(dropdown.selected() as usize)
                .cloned()
                .unwrap_or_default();
            let repo = page.repo.clone();
            let tag = tag.clone();
            let remote_for_msg = remote.clone();
            page.run_streaming(
                &format!("Pushing tag to {remote}…"),
                move |cancel, progress| repo.push_tag(&remote, &tag, Some(cancel), progress),
                move |page, result| {
                    match result {
                        Ok(()) => page.toast(&format!("Tag pushed to {remote_for_msg}")),
                        Err(GitError::Cancelled) => {}
                        Err(e) => page.error(&e),
                    }
                    page.refresh_all();
                },
            );
        });
        dialog.present();
    }

    fn delete_remote_tag_dialog(self: &Rc<Self>, tag: String) {
        let Some(window) = self.window() else { return };
        let remotes = self.state.borrow().snapshot.remotes.clone();
        if remotes.is_empty() {
            self.toast("No remotes configured");
            return;
        }
        let names: Vec<String> = remotes.iter().map(|r| r.name.clone()).collect();
        let model = gtk::StringList::new(&names.iter().map(|s| s.as_str()).collect::<Vec<_>>());
        let dropdown = gtk::DropDown::new(Some(model), gtk::Expression::NONE);
        dropdown.set_selected(0);
        let weak = Rc::downgrade(self);
        super::confirm::confirm_dangerous(
            &window,
            "Delete remote tag?",
            &format!("The tag `{tag}` will be deleted from the remote."),
            "Other users will no longer see this tag.",
            "Delete Remote Tag",
            move || {
                let Some(page) = weak.upgrade() else { return };
                let remote = names
                    .get(dropdown.selected() as usize)
                    .cloned()
                    .unwrap_or_default();
                let repo = page.repo.clone();
                let tag = tag.clone();
                page.run_streaming(
                    &format!("Deleting tag {tag} on {remote}…"),
                    move |cancel, progress| {
                        repo.delete_remote_tag(&remote, &tag, Some(cancel), progress)
                    },
                    |page, result| {
                        match result {
                            Ok(()) => page.toast("Remote tag deleted"),
                            Err(GitError::Cancelled) => {}
                            Err(e) => page.error(&e),
                        }
                        page.refresh_all();
                    },
                );
            },
        );
    }

    fn push_all_tags(self: &Rc<Self>, remote: Option<String>) {
        let repo = self.repo.clone();
        let label = "Pushing tags…".to_string();
        self.run_streaming(
            &label,
            move |cancel, progress| {
                let shared = std::sync::Arc::new(std::sync::Mutex::new(progress));
                let forward = {
                    let shared = std::sync::Arc::clone(&shared);
                    move |kind: StreamKind, line: &str| {
                        if let Ok(mut cb) = shared.lock() {
                            cb(kind, line);
                        }
                    }
                };
                match &remote {
                    Some(remote) => repo.push_tags(remote, Some(cancel), forward),
                    None => {
                        let remotes = repo.remotes()?;
                        for r in remotes {
                            let forward = {
                                let shared = std::sync::Arc::clone(&shared);
                                move |kind: StreamKind, line: &str| {
                                    if let Ok(mut cb) = shared.lock() {
                                        cb(kind, line);
                                    }
                                }
                            };
                            repo.push_tags(&r.name, Some(cancel.clone()), forward)?;
                        }
                        Ok(())
                    }
                }
            },
            |page, result| {
                match result {
                    Ok(()) => page.toast("Tags pushed"),
                    Err(GitError::Cancelled) => {}
                    Err(e) => page.error(&e),
                }
                page.refresh_all();
            },
        );
    }

    // -----------------------------------------------------------------
    // Dialogs
    // -----------------------------------------------------------------

    fn create_branch_dialog(self: &Rc<Self>) {
        let Some(window) = self.window() else { return };
        let dialog = adw::MessageDialog::new(
            Some(&window),
            Some("New branch"),
            Some("Create a branch and optionally switch to it."),
        );
        let box_ = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let name_entry = gtk::Entry::builder()
            .placeholder_text("branch-name")
            .activates_default(true)
            .build();
        let start_entry = gtk::Entry::builder()
            .placeholder_text("start point (default: HEAD)")
            .build();
        let checkout = gtk::CheckButton::with_label("Switch to the new branch");
        checkout.set_active(true);
        box_.append(&name_entry);
        box_.append(&start_entry);
        box_.append(&checkout);
        dialog.set_extra_child(Some(&box_));
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("ok", "Create");
        dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("ok"));
        dialog.set_close_response("cancel");
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            if response != "ok" {
                return;
            }
            let Some(page) = weak.upgrade() else { return };
            let name = name_entry.text().trim().to_string();
            if name.is_empty() {
                return;
            }
            let start = start_entry.text().trim().to_string();
            let start = if start.is_empty() { None } else { Some(start) };
            let checkout_value = checkout.is_active();
            let repo = page.repo.clone();
            page.run(
                move || repo.create_branch(&name, start.as_deref(), checkout_value),
                move |page, _| {
                    page.toast("Branch created");
                    page.refresh_all();
                },
            );
        });
        dialog.present();
    }

    fn create_branch_at_dialog(self: &Rc<Self>, oid: &str) {
        let Some(window) = self.window() else { return };
        let short = util::short_oid(oid).to_string();
        let weak = Rc::downgrade(self);
        let oid = oid.to_string();
        super::confirm::prompt(
            &window,
            &format!("Create branch at {short}"),
            "branch-name",
            "Create & Switch",
            move |name| {
                let name = name.trim().to_string();
                if name.is_empty() {
                    return;
                }
                if let Some(page) = weak.upgrade() {
                    let repo = page.repo.clone();
                    let oid = oid.clone();
                    page.run(
                        move || repo.checkout_new_branch(&name, Some(&oid)),
                        |page, _| {
                            page.toast("Branch created");
                            page.refresh_all();
                        },
                    );
                }
            },
        );
    }

    fn create_tag_dialog(self: &Rc<Self>, target: Option<String>) {
        let Some(window) = self.window() else { return };
        let dialog = adw::MessageDialog::new(
            Some(&window),
            Some("New tag"),
            Some("Create a lightweight or annotated tag."),
        );
        let box_ = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let name_entry = gtk::Entry::builder().placeholder_text("v1.0.0").build();
        let message_view = gtk::TextView::builder()
            .height_request(80)
            .wrap_mode(gtk::WrapMode::WordChar)
            .build();
        let message_scroll = gtk::ScrolledWindow::builder()
            .child(&message_view)
            .has_frame(true)
            .build();
        let annotated = gtk::CheckButton::with_label("Annotated tag");
        annotated.set_active(true);
        annotated
            .bind_property("active", &message_scroll, "sensitive")
            .sync_create()
            .build();
        let target_entry = gtk::Entry::builder()
            .placeholder_text("target commit (default: HEAD)")
            .build();
        if let Some(target) = &target {
            target_entry.set_text(target);
        }
        let force = gtk::CheckButton::with_label("Replace existing tag");
        box_.append(&name_entry);
        box_.append(&annotated);
        box_.append(&message_scroll);
        box_.append(&target_entry);
        box_.append(&force);
        dialog.set_extra_child(Some(&box_));
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("ok", "Create Tag");
        dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("ok"));
        dialog.set_close_response("cancel");
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            if response != "ok" {
                return;
            }
            let Some(page) = weak.upgrade() else { return };
            let name = name_entry.text().trim().to_string();
            if name.is_empty() {
                return;
            }
            let buffer = message_view.buffer();
            let message = buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .to_string();
            let annotated_value = annotated.is_active();
            let target = target_entry.text().trim().to_string();
            let target = if target.is_empty() {
                None
            } else {
                Some(target)
            };
            let force_value = force.is_active();
            let repo = page.repo.clone();
            let message_opt = if annotated_value && !message.trim().is_empty() {
                Some(message)
            } else if annotated_value {
                Some(name.clone())
            } else {
                None
            };
            page.run(
                move || {
                    repo.create_tag(
                        &name,
                        target.as_deref(),
                        message_opt.as_deref(),
                        false,
                        force_value,
                    )
                },
                |page, _| {
                    page.toast("Tag created");
                    page.refresh_all();
                },
            );
        });
        dialog.present();
    }

    fn create_stash_dialog(self: &Rc<Self>) {
        let Some(window) = self.window() else { return };
        let dialog = adw::MessageDialog::new(
            Some(&window),
            Some("Stash changes"),
            Some("Save the current working tree state and clean the working tree."),
        );
        let box_ = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let message_entry = gtk::Entry::builder()
            .placeholder_text("message (optional)")
            .build();
        let untracked = gtk::CheckButton::with_label("Include untracked files");
        untracked.set_active(true);
        let keep_index = gtk::CheckButton::with_label("Keep staged changes in the index");
        box_.append(&message_entry);
        box_.append(&untracked);
        box_.append(&keep_index);
        dialog.set_extra_child(Some(&box_));
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("ok", "Stash");
        dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("ok"));
        dialog.set_close_response("cancel");
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            if response != "ok" {
                return;
            }
            let Some(page) = weak.upgrade() else { return };
            let message = message_entry.text().to_string();
            let message = if message.trim().is_empty() {
                None
            } else {
                Some(message)
            };
            let untracked_value = untracked.is_active();
            let keep_index_value = keep_index.is_active();
            let repo = page.repo.clone();
            page.run(
                move || {
                    repo.stash_save(message.as_deref(), untracked_value, keep_index_value, None)
                },
                |page, _| {
                    page.toast("Changes stashed");
                    page.refresh_all();
                },
            );
        });
        dialog.present();
    }

    fn show_stash_diff(self: &Rc<Self>, index: usize) {
        let repo = self.repo.clone();
        let weak = Rc::downgrade(self);
        tasks::run_blocking(
            move || repo.diff_stash(index),
            move |result: GitResult<Vec<FileDiff>>| {
                let Some(page) = weak.upgrade() else { return };
                match result {
                    Ok(diffs) => {
                        page.view_stack.set_visible_child_name("changes");
                        match diffs.first() {
                            Some(file) => page.changes.show_diff(file, false, false),
                            None => page
                                .changes
                                .diff_view()
                                .show_placeholder("Empty stash", "No changes in this stash."),
                        }
                    }
                    Err(e) => page.error(&e),
                }
            },
        );
    }

    fn remote_dialog(self: &Rc<Self>, existing: Option<String>) {
        let Some(window) = self.window() else { return };
        let remotes = self.state.borrow().snapshot.remotes.clone();
        let current = existing
            .as_ref()
            .and_then(|name| remotes.iter().find(|r| &r.name == name).cloned());
        let dialog = adw::MessageDialog::new(
            Some(&window),
            Some(if existing.is_some() {
                "Edit remote"
            } else {
                "Add remote"
            }),
            Some("A remote is a repository URL such as https://…, ssh://… or git@host:path."),
        );
        let box_ = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let name_entry = gtk::Entry::builder()
            .placeholder_text("origin")
            .text(existing.clone().unwrap_or_default())
            .editable(existing.is_none())
            .build();
        let url_entry = gtk::Entry::builder()
            .placeholder_text("https://example.com/repo.git")
            .text(
                current
                    .as_ref()
                    .map(|r| r.fetch_url.clone())
                    .unwrap_or_default(),
            )
            .build();
        box_.append(&name_entry);
        box_.append(&url_entry);
        dialog.set_extra_child(Some(&box_));
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("ok", "Save");
        dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("ok"));
        dialog.set_close_response("cancel");
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            if response != "ok" {
                return;
            }
            let Some(page) = weak.upgrade() else { return };
            let name = name_entry.text().trim().to_string();
            let url = url_entry.text().trim().to_string();
            if name.is_empty() || url.is_empty() {
                page.toast("Remote name and URL are required");
                return;
            }
            let repo = page.repo.clone();
            let updating = existing.is_some();
            page.run(
                move || {
                    if updating {
                        repo.set_remote_url(&name, &url, false)?;
                        repo.set_remote_url(&name, &url, true)
                    } else {
                        repo.add_remote(&name, &url)
                    }
                },
                move |page, _| {
                    page.toast(if updating {
                        "Remote updated"
                    } else {
                        "Remote added"
                    });
                    page.refresh_all();
                },
            );
        });
        dialog.present();
    }

    fn repo_preferences_dialog(self: &Rc<Self>) {
        let Some(window) = self.window() else { return };
        let settings = self.settings.borrow().clone();
        let weak = Rc::downgrade(self);
        super::settings_dialog::show(
            &window,
            settings,
            Some(self.repo.clone()),
            None,
            Rc::new(move |settings| {
                if let Some(page) = weak.upgrade() {
                    page.apply_settings(settings);
                }
            }),
        );
    }

    // -----------------------------------------------------------------
    // Compare
    // -----------------------------------------------------------------

    fn compare_branches_dialog(self: &Rc<Self>) {
        let Some(window) = self.window() else { return };
        let branches: Vec<_> = self
            .state
            .borrow()
            .snapshot
            .branches
            .iter()
            .filter(|branch| !branch.is_remote_head())
            .cloned()
            .collect();
        if branches.len() < 2 {
            self.toast("At least two branches are needed to compare");
            return;
        }
        let dialog = adw::MessageDialog::new(
            Some(&window),
            Some("Compare branches"),
            Some("Choose a base and a target branch."),
        );
        let names = branches
            .iter()
            .map(|branch| comparison_ref_label(&branch.full_ref))
            .collect::<Vec<_>>();
        let head = branches
            .iter()
            .position(|branch| branch.is_head)
            .unwrap_or(0);
        let (grid, base, target) =
            branch_selectors(&names, "Base", "Target", head, usize::from(head == 0));
        dialog.set_extra_child(Some(&grid));
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("ok", "Compare");
        dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("ok"));
        dialog.set_close_response("cancel");

        let target_weak = target.downgrade();
        let dialog_weak = dialog.downgrade();
        base.connect_selected_notify(move |base| {
            if let (Some(target), Some(dialog)) = (target_weak.upgrade(), dialog_weak.upgrade()) {
                dialog.set_response_enabled("ok", base.selected() != target.selected());
            }
        });
        let base_weak = base.downgrade();
        let dialog_weak = dialog.downgrade();
        target.connect_selected_notify(move |target| {
            if let (Some(base), Some(dialog)) = (base_weak.upgrade(), dialog_weak.upgrade()) {
                dialog.set_response_enabled("ok", base.selected() != target.selected());
            }
        });
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_, response| {
            if response != "ok" || base.selected() == target.selected() {
                return;
            }
            let Some(page) = weak.upgrade() else { return };
            let pair = branches
                .get(base.selected() as usize)
                .zip(branches.get(target.selected() as usize));
            if let Some((base, target)) = pair {
                page.show_compare(&base.full_ref, &target.full_ref);
            }
        });
        dialog.present();
    }

    fn compare_commits_pick(self: &Rc<Self>) {
        let selected = self.history.selected_commit();
        let base = self.state.borrow().compare_base.clone();
        match (base, selected) {
            (None, Some(sha)) => {
                self.history.prepare_compare(&sha);
                self.state.borrow_mut().compare_base = Some(sha.clone());
                self.toast(&format!(
                    "Base set to {}. Select a target commit and press Compare again.",
                    util::short_oid(&sha)
                ));
            }
            (Some(base), Some(target)) => {
                self.history.clear_compare();
                self.state.borrow_mut().compare_base = None;
                self.show_compare(&base, &target);
            }
            _ => self.toast("Select a commit first"),
        }
    }

    fn show_worktree_compare(self: &Rc<Self>, rev: String) {
        let repo = self.repo.clone();
        let weak = Rc::downgrade(self);
        let rev_for_work = rev.clone();
        tasks::run_blocking(
            move || {
                let files = repo.changed_files_vs_worktree(&rev_for_work)?;
                let diffs = repo.diff_vs_worktree(&rev_for_work)?;
                Ok((files, diffs))
            },
            move |result: GitResult<(Vec<crate::git::ChangedFile>, Vec<FileDiff>)>| {
                let Some(page) = weak.upgrade() else { return };
                match result {
                    Ok((files, diffs)) => {
                        page.view_stack.set_visible_child_name("history");
                        page.history
                            .show_compare(&rev, "working tree", files, diffs);
                    }
                    Err(e) => page.error(&e),
                }
            },
        );
    }

    fn show_compare(self: &Rc<Self>, base: &str, target: &str) {
        let repo = self.repo.clone();
        let base = base.to_string();
        let target = target.to_string();
        let base_for_work = base.clone();
        let target_for_work = target.clone();
        let weak = Rc::downgrade(self);
        tasks::run_blocking(
            move || {
                let files = repo.changed_files_between(&base_for_work, &target_for_work)?;
                let diffs = repo.diff_commits(&base_for_work, &target_for_work)?;
                Ok((files, diffs))
            },
            move |result: GitResult<(Vec<crate::git::ChangedFile>, Vec<FileDiff>)>| {
                let Some(page) = weak.upgrade() else { return };
                match result {
                    Ok((files, diffs)) => {
                        page.view_stack.set_visible_child_name("history");
                        page.history.show_compare(
                            &comparison_ref_label(&base),
                            &comparison_ref_label(&target),
                            files,
                            diffs,
                        );
                    }
                    Err(e) => page.error(&e),
                }
            },
        );
    }

    // -----------------------------------------------------------------
    // Keyboard helpers
    // -----------------------------------------------------------------

    /// Add a widget to the start of this page's header bar.
    pub fn add_header_start(&self, widget: &impl IsA<gtk::Widget>) {
        self.header.pack_start(widget);
    }

    /// Apply changed application settings.
    pub fn apply_settings(self: &Rc<Self>, settings: crate::core::Settings) {
        self.changes.set_show_ignored(settings.show_ignored_files);
        self.changes
            .set_diff_mode(diff_mode_from(settings.diff_view_mode));
        self.changes
            .commit_panel()
            .set_signoff(settings.commit_sign_off);
        self.changes
            .commit_panel()
            .set_gpg(settings.commit_gpg_sign);
        *self.settings.borrow_mut() = settings;
        self.refresh_all();
    }

    /// Switch to a named section (changes, history, tags,
    /// stashes, remotes, conflicts).
    pub fn show_section(self: &Rc<Self>, name: &str) {
        if self.view_stack.child_by_name(name).is_some() {
            let already_there = self.view_stack.visible_child_name().as_deref() == Some(name);
            self.view_stack.set_visible_child_name(name);
            // Pressing the Changes shortcut again focuses the commit message.
            if name == "changes" && already_there {
                self.changes.commit_panel().focus_message();
            }
        }
        if name == "changes" {
            self.refresh_status_views();
        }
    }

    /// Refresh the changes view without touching branches/tags/remotes.
    pub fn refresh_status_views(self: &Rc<Self>) {
        self.refresh_status();
    }

    pub fn open_branch_menu(&self) {
        self.branch_menu.open();
    }

    pub fn focus_search(self: &Rc<Self>) {
        match self.view_stack.visible_child_name().as_deref() {
            Some("history") => self.history.focus_search(),
            Some("tags") => self.tags.focus_filter(),
            _ => {
                self.view_stack.set_visible_child_name("history");
                self.history.focus_search();
            }
        }
    }

    pub fn stage_all(self: &Rc<Self>) {
        self.handle_request(UiRequest::StageAll);
    }

    pub fn stage_selected(self: &Rc<Self>) {
        let rows = self.changes.selected_rows();
        let paths: Vec<PathBuf> = rows
            .iter()
            .filter(|r| !r.conflicted && !r.ignored)
            .map(|r| r.path.clone())
            .collect();
        if paths.is_empty() {
            self.toast("Select files to stage");
            return;
        }
        self.handle_request(UiRequest::StagePaths(paths));
    }

    pub fn commit_now(self: &Rc<Self>) {
        let panel = self.changes.commit_panel();
        if !panel.submit() {
            self.toast("Enter a message and stage changes first");
        }
    }

    pub fn fetch_current(self: &Rc<Self>) {
        self.fetch_dialog();
    }
}

fn comparison_ref_label(reference: &str) -> String {
    if let Some(name) = reference.strip_prefix("refs/heads/") {
        format!("{name} (local)")
    } else if let Some(name) = reference.strip_prefix("refs/remotes/") {
        format!("{name} (remote)")
    } else {
        reference.to_string()
    }
}

fn branch_selectors(
    names: &[String],
    first_label: &str,
    second_label: &str,
    first_index: usize,
    second_index: usize,
) -> (gtk::Grid, gtk::DropDown, gtk::DropDown) {
    let grid = gtk::Grid::builder()
        .row_spacing(6)
        .column_spacing(8)
        .build();
    let model = gtk::StringList::new(&names.iter().map(String::as_str).collect::<Vec<_>>());
    let first = gtk::DropDown::new(Some(model.clone()), gtk::Expression::NONE);
    let second = gtk::DropDown::new(Some(model), gtk::Expression::NONE);
    for (row, label, selector, index) in [
        (0, first_label, &first, first_index),
        (1, second_label, &second, second_index),
    ] {
        let label = gtk::Label::builder().label(label).xalign(0.0).build();
        selector.set_selected(index as u32);
        selector.set_enable_search(true);
        selector.update_relation(&[gtk::accessible::Relation::LabelledBy(&[label.upcast_ref()])]);
        grid.attach(&label, 0, row, 1, 1);
        grid.attach(selector, 1, row, 1, 1);
    }
    (grid, first, second)
}

fn request_name(req: &UiRequest) -> &'static str {
    match req {
        UiRequest::RefreshAll => "refresh",
        UiRequest::StagePaths(_) => "stage",
        UiRequest::UnstagePaths(_) => "unstage",
        UiRequest::Commit(_) => "commit",
        UiRequest::CheckoutBranch(_) => "checkout",
        UiRequest::MergeBranch(_) => "merge",
        UiRequest::RebaseOnto(_) => "rebase",
        UiRequest::IntegrateBranches { .. } => "branch-drop",
        _ => "other",
    }
}

fn repo_menu_popover(request: Rc<RefCell<Option<Sender>>>, repo_path: PathBuf) -> gtk::Popover {
    let popover = gtk::Popover::new();
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 2);
    box_.set_margin_start(6);
    box_.set_margin_end(6);
    box_.set_margin_top(6);
    box_.set_margin_bottom(6);

    let add = |label: &str, req: Option<UiRequest>, action: Option<&str>| {
        let button = gtk::Button::builder()
            .label(label)
            .has_frame(false)
            .halign(gtk::Align::Fill)
            .build();
        let pop = popover.clone();
        match (req, action) {
            (Some(req), _) => {
                let request = request.clone();
                button.connect_clicked(move |_| {
                    if let Some(sender) = request.borrow().as_ref() {
                        sender(req.clone());
                    }
                    pop.popdown();
                });
            }
            (None, Some(action)) => {
                let action = action.to_string();
                let button_for_click = button.clone();
                button.connect_clicked(move |_| {
                    if let Some(window) = button_for_click.root().and_downcast::<gtk::Window>() {
                        let _ = window.activate_action(&format!("win.{action}"), None);
                    }
                    pop.popdown();
                });
            }
            (None, None) => {}
        }
        box_.append(&button);
    };

    add(
        "Repository Settings…",
        Some(UiRequest::RepoPreferences),
        None,
    );
    add(
        "Open Repository Folder",
        Some(UiRequest::OpenContainingFolder),
        None,
    );
    add(
        "Copy Repository Path",
        Some(UiRequest::CopyText(repo_path.display().to_string())),
        None,
    );
    add("Keyboard Shortcuts", None, Some("shortcuts"));
    add("Diagnostics", None, Some("diagnostics"));
    add("About GitTrellis", None, Some("about"));
    popover.set_child(Some(&box_));
    popover
}

fn diff_mode_from(mode: crate::core::DiffViewMode) -> DiffMode {
    match mode {
        crate::core::DiffViewMode::SideBySide => DiffMode::SideBySide,
        crate::core::DiffViewMode::Unified => DiffMode::Unified,
    }
}

/// A case-insensitive pathspec that matches commits touching any file whose
/// path contains `query` (used by the history "File path" search).
pub fn path_substring_pathspec(query: &str) -> PathBuf {
    let mut escaped = String::with_capacity(query.len());
    for ch in query.chars() {
        if matches!(ch, '*' | '?' | '[' | ']' | '\\') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    PathBuf::from(format!(":(icase,glob)**/*{escaped}*"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_search_spec_escapes_globs() {
        assert_eq!(
            path_substring_pathspec("main.rs").to_string_lossy(),
            ":(icase,glob)**/*main.rs*"
        );
        assert_eq!(
            path_substring_pathspec("a*b").to_string_lossy(),
            ":(icase,glob)**/*a\\*b*"
        );
    }

    #[test]
    fn push_refspecs_handles_short_and_full_names() {
        assert_eq!(
            push_refspecs("main", "main"),
            Some(("refs/heads/main".into(), "refs/heads/main".into()))
        );
        assert_eq!(
            push_refspecs("feature/x", "publish/x"),
            Some(("refs/heads/feature/x".into(), "refs/heads/publish/x".into()))
        );
        assert_eq!(
            push_refspecs("refs/heads/main", "refs/heads/main"),
            Some(("refs/heads/main".into(), "refs/heads/main".into()))
        );
        assert_eq!(
            push_refspecs("HEAD", "main"),
            Some(("HEAD".into(), "refs/heads/main".into()))
        );
        assert_eq!(push_refspecs("", "main"), None);
        assert_eq!(push_refspecs("main", ""), None);
        assert_eq!(push_refspecs("-bad", "main"), None);
    }

    #[test]
    fn upstream_splitting() {
        assert_eq!(
            split_upstream("origin/main"),
            Some(("origin".into(), "main".into()))
        );
        assert_eq!(
            split_upstream("origin/feature/deep/branch"),
            Some(("origin".into(), "feature/deep/branch".into()))
        );
        assert_eq!(split_upstream("nonsense"), None);
        assert_eq!(split_upstream("/nonsense"), None);
        assert_eq!(split_upstream("nonsense/"), None);
    }
}
