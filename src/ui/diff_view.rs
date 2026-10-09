//! Diff rendering: unified and side-by-side views with hunk selection and
//! hunk-level stage/unstage/discard actions.
//!
//! Rendering uses `GtkTextView` with text tags for performance; large diffs
//! are truncated with a notice instead of freezing the UI.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;

use crate::git::{ChangeKind, FileDiff, LineKind};

use super::util::{self, DiffColors};

const MAX_UNIFIED_LINES: usize = 40_000;
const MAX_SBS_ROWS: usize = 20_000;
const SBS_COL_WIDTH: usize = 108;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffAction {
    Stage,
    Unstage,
    Discard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiffMode {
    #[default]
    Unified,
    SideBySide,
}

type ActionHandler = Rc<dyn Fn(DiffAction, Option<usize>)>;

pub struct DiffView {
    root: gtk::Box,
    file: RefCell<Option<FileDiff>>,
    mode: Cell<DiffMode>,
    allow_actions: Cell<bool>,
    discard_enabled: Cell<bool>,
    show_line_numbers: Cell<bool>,
    handler: RefCell<Option<ActionHandler>>,

    path_label: gtk::Label,
    stats_label: gtk::Label,
    status_icon: gtk::Image,
    content: gtk::Stack,
    unified_view: gtk::TextView,
    sbs_left: gtk::TextView,
    sbs_right: gtk::TextView,
    placeholder: gtk::Box,
    action_bar: gtk::Box,
    stage_button: gtk::Button,
    unstage_button: gtk::Button,
    discard_button: gtk::Button,
    numbers_button: gtk::ToggleButton,
    unified_toggle: gtk::ToggleButton,
    sbs_toggle: gtk::ToggleButton,

    selected_hunk: Rc<Cell<Option<usize>>>,
    line_hunk_map: RefCell<Vec<Option<usize>>>,
    hunk_line_ranges: RefCell<Vec<(usize, usize)>>,
}

impl DiffView {
    pub fn new() -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);

        // ---- header ----
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        header.set_margin_top(6);
        header.set_margin_bottom(6);
        header.set_margin_start(10);
        header.set_margin_end(6);
        let status_icon = gtk::Image::new();
        status_icon.set_pixel_size(16);
        let path_label = gtk::Label::builder()
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .xalign(0.0)
            .hexpand(true)
            .build();
        path_label.add_css_class("heading");
        let stats_label = gtk::Label::new(None);
        stats_label.add_css_class("dim-label");
        stats_label.add_css_class("monospace");
        header.append(&status_icon);
        header.append(&path_label);
        header.append(&stats_label);

        let numbers_button = gtk::ToggleButton::builder()
            .icon_name("format-indent-more-symbolic")
            .tooltip_text("Show line numbers")
            .active(true)
            .build();
        numbers_button.add_css_class("flat");

        let unified_toggle = gtk::ToggleButton::with_label("Unified");
        let sbs_toggle = gtk::ToggleButton::with_label("Side by side");
        sbs_toggle.set_group(Some(&unified_toggle));
        unified_toggle.add_css_class("flat");
        sbs_toggle.add_css_class("flat");
        let toggle_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        toggle_box.add_css_class("linked");
        toggle_box.append(&unified_toggle);
        toggle_box.append(&sbs_toggle);

        let mode_button = gtk::MenuButton::builder()
            .icon_name("view-dual-symbolic")
            .tooltip_text("Diff layout")
            .build();
        mode_button.add_css_class("flat");
        let popover = gtk::Popover::new();
        let popover_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        popover_box.set_margin_start(8);
        popover_box.set_margin_end(8);
        popover_box.set_margin_top(8);
        popover_box.set_margin_bottom(8);
        let label = gtk::Label::builder()
            .label("Diff layout")
            .xalign(0.0)
            .build();
        label.add_css_class("heading");
        popover_box.append(&label);
        popover_box.append(&toggle_box);
        popover.set_child(Some(&popover_box));
        mode_button.set_popover(Some(&popover));

        header.append(&numbers_button);
        header.append(&mode_button);

        // ---- content ----
        let content = gtk::Stack::builder()
            .vexpand(true)
            .hexpand(true)
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();

        let unified_view = make_text_view();
        let unified_scroller = gtk::ScrolledWindow::builder()
            .child(&unified_view)
            .vexpand(true)
            .hexpand(true)
            .build();
        content.add_named(&unified_scroller, Some("unified"));

        let sbs_left = make_text_view();
        let sbs_right = make_text_view();
        let left_scroller = gtk::ScrolledWindow::builder()
            .child(&sbs_left)
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .build();
        let right_scroller = gtk::ScrolledWindow::builder()
            .child(&sbs_right)
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .build();
        let sbs_paned = gtk::Paned::new(gtk::Orientation::Horizontal);
        sbs_paned.set_start_child(Some(&left_scroller));
        sbs_paned.set_end_child(Some(&right_scroller));
        sbs_paned.set_resize_start_child(true);
        sbs_paned.set_resize_end_child(true);
        sbs_paned.set_wide_handle(true);
        content.add_named(&sbs_paned, Some("sbs"));

        {
            let adj_left = left_scroller.vadjustment();
            let adj_right = right_scroller.vadjustment();
            adj_right
                .bind_property("value", &adj_left, "value")
                .sync_create()
                .build();
            adj_left
                .bind_property("value", &adj_right, "value")
                .sync_create()
                .build();
        }

        let placeholder = gtk::Box::new(gtk::Orientation::Vertical, 6);
        placeholder.set_valign(gtk::Align::Center);
        placeholder.set_halign(gtk::Align::Center);
        placeholder.set_vexpand(true);
        content.add_named(&placeholder, Some("placeholder"));

        // ---- action bar ----
        let action_bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        action_bar.set_margin_top(4);
        action_bar.set_margin_bottom(6);
        action_bar.set_margin_start(10);
        action_bar.set_margin_end(10);
        let hint_label = gtk::Label::builder()
            .label("Click a hunk header (@@ line) to select it")
            .xalign(0.0)
            .hexpand(true)
            .build();
        hint_label.add_css_class("dim-label");
        let stage_button = gtk::Button::with_label("Stage Hunk");
        let unstage_button = gtk::Button::with_label("Unstage Hunk");
        let discard_button = gtk::Button::with_label("Discard Hunk");
        stage_button.add_css_class("flat");
        unstage_button.add_css_class("flat");
        discard_button.add_css_class("flat");
        discard_button.add_css_class("destructive-action");
        action_bar.append(&hint_label);
        action_bar.append(&stage_button);
        action_bar.append(&unstage_button);
        action_bar.append(&discard_button);
        action_bar.set_visible(false);

        root.append(&header);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        root.append(&content);
        root.append(&action_bar);

        let view = Rc::new(Self {
            root,
            file: RefCell::new(None),
            mode: Cell::new(DiffMode::Unified),
            allow_actions: Cell::new(false),
            discard_enabled: Cell::new(false),
            show_line_numbers: Cell::new(true),
            handler: RefCell::new(None),
            path_label,
            stats_label,
            status_icon,
            content,
            unified_view,
            sbs_left,
            sbs_right,
            placeholder,
            action_bar,
            stage_button,
            unstage_button,
            discard_button,
            numbers_button,
            unified_toggle,
            sbs_toggle,
            selected_hunk: Rc::new(Cell::new(None)),
            line_hunk_map: RefCell::new(Vec::new()),
            hunk_line_ranges: RefCell::new(Vec::new()),
        });

        view.wire_events();
        view.show_placeholder("Select a file", "Its diff will appear here.");
        view
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Register the callback used for hunk actions.
    pub fn set_handler(&self, handler: impl Fn(DiffAction, Option<usize>) + 'static) {
        *self.handler.borrow_mut() = Some(Rc::new(handler));
    }

    pub fn set_allow_actions(&self, allow: bool) {
        self.allow_actions.set(allow);
        self.discard_enabled.set(allow);
        self.action_bar.set_visible(allow);
        self.update_action_buttons();
    }

    /// Discarding is only offered for working-tree diffs; staged changes must
    /// be unstaged first so that the index and worktree stay consistent.
    pub fn set_discard_enabled(&self, enabled: bool) {
        self.discard_enabled.set(enabled);
        self.update_action_buttons();
    }

    pub fn allow_actions(&self) -> bool {
        self.allow_actions.get()
    }

    pub fn set_mode(&self, mode: DiffMode) {
        self.mode.set(mode);
        self.sync_toggles();
        self.render();
    }

    pub fn mode(&self) -> DiffMode {
        self.mode.get()
    }

    pub fn clear(&self) {
        self.file.borrow_mut().take();
        self.show_placeholder("Select a file", "Its diff will appear here.");
    }

    pub fn show_placeholder(&self, title: &str, subtitle: &str) {
        while let Some(child) = self.placeholder.first_child() {
            self.placeholder.remove(&child);
        }
        let icon = gtk::Image::from_icon_name("text-x-generic-symbolic");
        icon.set_pixel_size(48);
        icon.add_css_class("dim-label");
        let t = gtk::Label::new(Some(title));
        t.add_css_class("heading");
        let s = gtk::Label::new(Some(subtitle));
        s.add_css_class("dim-label");
        s.set_wrap(true);
        s.set_justify(gtk::Justification::Center);
        s.set_max_width_chars(60);
        self.placeholder.append(&icon);
        self.placeholder.append(&t);
        self.placeholder.append(&s);
        self.path_label.set_label("");
        self.stats_label.set_label("");
        self.status_icon.set_visible(false);
        self.content.set_visible_child_name("placeholder");
    }

    fn wire_events(self: &Rc<Self>) {
        let this = Rc::clone(self);
        self.unified_view
            .buffer()
            .connect_mark_set(move |_, _, mark| {
                if mark.name().as_deref() == Some("insert") {
                    this.refresh_selection_from_cursor();
                }
            });

        let this = Rc::clone(self);
        self.numbers_button.connect_toggled(move |btn| {
            this.show_line_numbers.set(btn.is_active());
            this.render();
        });

        let this = Rc::clone(self);
        self.unified_toggle.connect_toggled(move |b| {
            if b.is_active() && this.mode.get() != DiffMode::Unified {
                this.set_mode(DiffMode::Unified);
            }
        });
        let this = Rc::clone(self);
        self.sbs_toggle.connect_toggled(move |b| {
            if b.is_active() && this.mode.get() != DiffMode::SideBySide {
                this.set_mode(DiffMode::SideBySide);
            }
        });

        let this = Rc::clone(self);
        self.stage_button.connect_clicked(move |_| {
            if let Some(h) = this.selected_hunk.get() {
                this.emit_action(DiffAction::Stage, Some(h));
            }
        });
        let this = Rc::clone(self);
        self.unstage_button.connect_clicked(move |_| {
            if let Some(h) = this.selected_hunk.get() {
                this.emit_action(DiffAction::Unstage, Some(h));
            }
        });
        let this = Rc::clone(self);
        self.discard_button.connect_clicked(move |_| {
            if let Some(h) = this.selected_hunk.get() {
                this.emit_action(DiffAction::Discard, Some(h));
            }
        });
    }

    fn sync_toggles(&self) {
        self.unified_toggle
            .set_active(self.mode.get() == DiffMode::Unified);
        self.sbs_toggle
            .set_active(self.mode.get() == DiffMode::SideBySide);
    }

    fn emit_action(&self, action: DiffAction, hunk: Option<usize>) {
        if let Some(handler) = self.handler.borrow().as_ref() {
            handler(action, hunk);
        }
    }

    fn refresh_selection_from_cursor(self: &Rc<Self>) {
        if !self.allow_actions.get() {
            return;
        }
        let buffer = self.unified_view.buffer();
        let cursor = buffer.iter_at_offset(buffer.cursor_position());
        let line = cursor.line() as usize;
        let idx = self.line_hunk_map.borrow().get(line).copied().flatten();
        self.selected_hunk.set(idx);
        self.apply_selection_tag();
        self.update_action_buttons();
    }

    fn apply_selection_tag(&self) {
        let buffer = self.unified_view.buffer();
        buffer.remove_tag_by_name(
            "hunk-selected",
            &buffer.start_iter(),
            &buffer.end_iter(),
        );
        if let Some(sel) = self.selected_hunk.get() {
            let ranges = self.hunk_line_ranges.borrow();
            if let Some((start, end)) = ranges.get(sel).copied() {
                if let (Some(s), Some(e)) = (
                    buffer.iter_at_line(start as i32),
                    buffer.iter_at_line(end.saturating_sub(1) as i32),
                ) {
                    let e = e;
                    let mut e_end = e;
                    e_end.forward_to_line_end();
                    buffer.apply_tag_by_name("hunk-selected", &s, &e_end);
                }
            }
        }
    }

    fn update_action_buttons(&self) {
        let has = self.selected_hunk.get().is_some() && self.allow_actions.get();
        let can_discard = has && self.discard_enabled.get();
        self.stage_button.set_sensitive(has);
        self.unstage_button.set_sensitive(has);
        self.discard_button.set_sensitive(can_discard);
        self.discard_button.set_visible(self.discard_enabled.get());
    }

    pub fn show_file(self: &Rc<Self>, file: &FileDiff) {
        *self.file.borrow_mut() = Some(file.clone());
        self.selected_hunk.set(None);
        self.line_hunk_map.borrow_mut().clear();
        self.hunk_line_ranges.borrow_mut().clear();

        self.set_header(file);

        if file.is_binary {
            self.show_placeholder(
                "Binary file",
                "This file cannot be displayed as text. It can still be staged, unstaged or discarded as a whole.",
            );
            self.set_header(file);
            return;
        }
        if file.hunks.is_empty() {
            self.show_placeholder(
                "No text changes",
                "Only file metadata (mode or rename) changed.",
            );
            self.set_header(file);
            return;
        }
        self.render();
    }

    fn set_header(&self, file: &FileDiff) {
        let (icon, class, _) = util::change_icon(file.status);
        self.status_icon.set_icon_name(Some(icon));
        for c in ["error", "success", "warning", "accent", "dim-label"] {
            self.status_icon.remove_css_class(c);
        }
        self.status_icon.add_css_class(class);
        self.status_icon.set_visible(true);
        self.path_label.set_label(&file.display_path());
        let stats = if file.is_binary {
            "binary file".to_string()
        } else {
            format!("+{} −{}", file.added_lines(), file.removed_lines())
        };
        self.stats_label.set_label(&stats);
    }

    fn render(&self) {
        let file = self.file.borrow().clone();
        let Some(file) = file else { return };
        if file.hunks.is_empty() || file.is_binary {
            return;
        }
        let colors = util::diff_colors(&self.root);
        match self.mode.get() {
            DiffMode::Unified => {
                self.content.set_visible_child_name("unified");
                self.render_unified(&file, &colors);
            }
            DiffMode::SideBySide => {
                self.content.set_visible_child_name("sbs");
                self.render_side_by_side(&file, &colors);
            }
        }
    }

    fn render_unified(&self, file: &FileDiff, colors: &DiffColors) {
        let buffer = self.unified_view.buffer();
        buffer.set_text("");
        create_common_tags(&buffer, colors);
        let show_numbers = self.show_line_numbers.get();

        let mut line_map: Vec<Option<usize>> = Vec::new();
        let mut hunk_ranges: Vec<(usize, usize)> = Vec::new();
        let mut truncated = false;

        'outer: for (hunk_idx, hunk) in file.hunks.iter().enumerate() {
            let start_line = line_map.len();
            insert_tagged(
                &buffer,
                &format!("{}\n", hunk.header()),
                &["diff-hunk"],
            );
            line_map.push(Some(hunk_idx));

            let mut old_no = hunk.old_start;
            let mut new_no = hunk.new_start;
            for line in &hunk.lines {
                if line_map.len() >= MAX_UNIFIED_LINES {
                    truncated = true;
                    break 'outer;
                }
                let text = match line.kind {
                    LineKind::NoNewline => "\\ No newline at end of file\n".to_string(),
                    LineKind::Context => {
                        let t = numbered(show_numbers, Some(old_no), Some(new_no), ' ', &line.text);
                        old_no += 1;
                        new_no += 1;
                        t
                    }
                    LineKind::Removed => {
                        let t = numbered(show_numbers, Some(old_no), None, '-', &line.text);
                        old_no += 1;
                        t
                    }
                    LineKind::Added => {
                        let t = numbered(show_numbers, None, Some(new_no), '+', &line.text);
                        new_no += 1;
                        t
                    }
                };
                let tag = match line.kind {
                    LineKind::Added => vec!["diff-add", "diff-num-region"],
                    LineKind::Removed => vec!["diff-del", "diff-num-region"],
                    _ => vec!["diff-num-region"],
                };
                insert_number_tagged(&buffer, &text, &tag, show_numbers, line.kind == LineKind::NoNewline);
                line_map.push(Some(hunk_idx));
            }
            hunk_ranges.push((start_line, line_map.len()));
        }

        if truncated {
            let mut end = buffer.end_iter();
            buffer.insert(
                &mut end,
                "\n… diff truncated for display. The full diff can be copied from the Git console.\n",
            );
        }
        *self.line_hunk_map.borrow_mut() = line_map;
        *self.hunk_line_ranges.borrow_mut() = hunk_ranges;
        self.apply_selection_tag();
        self.update_action_buttons();
    }

    fn render_side_by_side(&self, file: &FileDiff, colors: &DiffColors) {
        let left_buffer = self.sbs_left.buffer();
        let right_buffer = self.sbs_right.buffer();
        left_buffer.set_text("");
        right_buffer.set_text("");
        left_buffer.create_tag(Some("del"), &[("background-rgba", &colors.del_bg)]);
        right_buffer.create_tag(Some("add"), &[("background-rgba", &colors.add_bg)]);
        right_buffer.create_tag(Some("word"), &[("background-rgba", &colors.word_bg)]);
        left_buffer.create_tag(Some("word"), &[("background-rgba", &colors.word_bg)]);
        for buf in [&left_buffer, &right_buffer] {
            buf.create_tag(Some("hunk"),
                &[
                    ("background-rgba", &colors.hunk_bg),
                    ("foreground-rgba", &colors.hunk_fg),
                    ("weight", &700i32),
                ],
            );
            buf.create_tag(Some("num"),
                &[("foreground-rgba", &colors.dim_fg), ("scale", &0.9f64)],
            );
        }

        let width = SBS_COL_WIDTH;
        let mut rows = 0usize;
        let mut truncated = false;
        let mut hunk_ranges: Vec<(usize, usize)> = Vec::new();

        'outer: for hunk in &file.hunks {
            let start_row = rows;
            let header = format!("{}\n", hunk.header());
            insert_tagged(&left_buffer, &header, &["hunk"]);
            insert_tagged(&right_buffer, &header, &["hunk"]);
            rows += 1;

            let mut old_no = hunk.old_start;
            let mut new_no = hunk.new_start;
            let mut i = 0usize;
            while i < hunk.lines.len() {
                if rows >= MAX_SBS_ROWS {
                    truncated = true;
                    break 'outer;
                }
                match hunk.lines[i].kind {
                    LineKind::Context => {
                        let t = &hunk.lines[i].text;
                        insert_tagged(
                            &left_buffer,
                            &sbs_line(Some(old_no), t, width),
                            &["num"],
                        );
                        insert_tagged(
                            &right_buffer,
                            &sbs_line(Some(new_no), t, width),
                            &["num"],
                        );
                        old_no += 1;
                        new_no += 1;
                        i += 1;
                        rows += 1;
                    }
                    LineKind::NoNewline => {
                        insert_tagged(
                            &left_buffer,
                            "\\ No newline at end of file\n",
                            &["num"],
                        );
                        insert_tagged(
                            &right_buffer,
                            "\\ No newline at end of file\n",
                            &["num"],
                        );
                        i += 1;
                        rows += 1;
                    }
                    LineKind::Removed | LineKind::Added => {
                        let mut removed: Vec<&str> = Vec::new();
                        let mut added: Vec<&str> = Vec::new();
                        while i < hunk.lines.len() && hunk.lines[i].kind == LineKind::Removed {
                            removed.push(&hunk.lines[i].text);
                            i += 1;
                        }
                        while i < hunk.lines.len() && hunk.lines[i].kind == LineKind::Added {
                            added.push(&hunk.lines[i].text);
                            i += 1;
                        }
                        let pairs = removed.len().max(added.len());
                        for k in 0..pairs {
                            if rows >= MAX_SBS_ROWS {
                                truncated = true;
                                break 'outer;
                            }
                            let l = removed.get(k).copied();
                            let r = added.get(k).copied();
                            let left_text = match l {
                                Some(t) => sbs_line(Some(old_no), t, width),
                                None => sbs_line(None, "", width),
                            };
                            let right_text = match r {
                                Some(t) => sbs_line(Some(new_no), t, width),
                                None => sbs_line(None, "", width),
                            };
                            let left_off = left_buffer.char_count();
                            let right_off = right_buffer.char_count();
                            let left_tags: &[&str] = if l.is_some() {
                                &["num", "del"]
                            } else {
                                &["num"]
                            };
                            let right_tags: &[&str] = if r.is_some() {
                                &["num", "add"]
                            } else {
                                &["num"]
                            };
                            insert_tagged(&left_buffer, &left_text, left_tags);
                            insert_tagged(&right_buffer, &right_text, right_tags);

                            if let (Some(l), Some(r)) = (l, r) {
                                if let Some((prefix, span)) = differing_span(l, r) {
                                    let base = 7 + prefix;
                                    apply_word_tag(&left_buffer, left_off, base, span);
                                    apply_word_tag(&right_buffer, right_off, base, span);
                                }
                            }
                            if l.is_some() {
                                old_no += 1;
                            }
                            if r.is_some() {
                                new_no += 1;
                            }
                            rows += 1;
                        }
                    }
                }
            }
            hunk_ranges.push((start_row, rows));
        }

        if truncated {
            let mut end = left_buffer.end_iter();
            left_buffer.insert(&mut end, "… truncated\n");
            let mut end = right_buffer.end_iter();
            right_buffer.insert(&mut end, "… truncated\n");
        }
        *self.hunk_line_ranges.borrow_mut() = hunk_ranges;
        self.line_hunk_map.borrow_mut().clear();
        self.selected_hunk.set(None);
        self.update_action_buttons();
    }

    pub fn current_file(&self) -> Option<FileDiff> {
        self.file.borrow().clone()
    }
}

fn create_common_tags(buffer: &gtk::TextBuffer, colors: &DiffColors) {
    buffer.create_tag(Some("diff-add"), &[("background-rgba", &colors.add_bg)]);
    buffer.create_tag(Some("diff-del"), &[("background-rgba", &colors.del_bg)]);
    buffer.create_tag(Some("diff-hunk"),
        &[
            ("background-rgba", &colors.hunk_bg),
            ("foreground-rgba", &colors.hunk_fg),
            ("weight", &700i32),
        ],
    );
    buffer.create_tag(Some("diff-num"),
        &[("foreground-rgba", &colors.dim_fg), ("scale", &0.9f64)],
    );
    buffer.create_tag(Some("hunk-selected"), &[("background-rgba", &colors.word_bg)]);
}

fn numbered(show: bool, old: Option<u32>, new: Option<u32>, prefix: char, text: &str) -> String {
    if !show {
        return format!("{prefix}{text}\n");
    }
    let old_s = old.map(|n| n.to_string()).unwrap_or_default();
    let new_s = new.map(|n| n.to_string()).unwrap_or_default();
    format!("{old_s:>5} {new_s:>5} {prefix}{text}\n")
}

fn insert_number_tagged(
    buffer: &gtk::TextBuffer,
    text: &str,
    tags: &[&str],
    show_numbers: bool,
    no_newline_marker: bool,
) {
    let start = buffer.char_count();
    let mut end = buffer.end_iter();
    buffer.insert(&mut end, text);
    let end = buffer.char_count();
    let s = buffer.iter_at_offset(start);
    let e = buffer.iter_at_offset(end);
    buffer.remove_tag_by_name("diff-num", &s, &e);
    for tag in tags {
        if *tag == "diff-num-region" {
            continue;
        }
        buffer.apply_tag_by_name(tag, &s, &e);
    }
    if show_numbers && !no_newline_marker {
        let prefix_chars = 13.min(text.chars().count());
        let num_end = buffer.iter_at_offset(start + prefix_chars as i32);
        buffer.apply_tag_by_name("diff-num", &s, &num_end);
    }
}

fn insert_tagged(buffer: &gtk::TextBuffer, text: &str, tags: &[&str]) {
    let start = buffer.char_count();
    let mut end = buffer.end_iter();
    buffer.insert(&mut end, text);
    let end = buffer.char_count();
    let s = buffer.iter_at_offset(start);
    let e = buffer.iter_at_offset(end);
    for tag in tags {
        buffer.apply_tag_by_name(tag, &s, &e);
    }
}

fn apply_word_tag(buffer: &gtk::TextBuffer, base_offset: i32, prefix: usize, span: usize) {
    if span == 0 {
        return;
    }
    let start = base_offset + prefix as i32;
    let end = start + span as i32;
    let len = buffer.char_count();
    let start = start.clamp(0, len);
    let end = end.clamp(start, len);
    let s = buffer.iter_at_offset(start);
    let e = buffer.iter_at_offset(end);
    buffer.apply_tag_by_name("word", &s, &e);
}

fn sbs_line(number: Option<u32>, text: &str, width: usize) -> String {
    let num = number.map(|n| n.to_string()).unwrap_or_default();
    let chars = text.chars().count();
    let padded: String = if chars >= width {
        text.to_string()
    } else {
        format!("{text}{}", " ".repeat(width - chars))
    };
    format!("{num:>5}  {padded}\n")
}

fn differing_span(a: &str, b: &str) -> Option<(usize, usize)> {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let mut prefix = 0;
    while prefix < a_chars.len() && prefix < b_chars.len() && a_chars[prefix] == b_chars[prefix] {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix + prefix < a_chars.len()
        && suffix + prefix < b_chars.len()
        && a_chars[a_chars.len() - 1 - suffix] == b_chars[b_chars.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let span = (a_chars.len() - prefix - suffix).max(b_chars.len() - prefix - suffix);
    if span == 0 {
        None
    } else {
        Some((prefix, span))
    }
}

fn make_text_view() -> gtk::TextView {
    let view = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .vexpand(true)
        .hexpand(true)
        .left_margin(8)
        .right_margin(8)
        .top_margin(6)
        .bottom_margin(6)
        .build();
    view.set_wrap_mode(gtk::WrapMode::None);
    view
}

/// Build a patch for a single hunk of a file diff.
pub fn hunk_patch(file: &FileDiff, hunk: usize) -> Option<String> {
    crate::git::patch::build_hunk_patch(file, &[hunk], false)
}

pub fn full_file_patch(file: &FileDiff) -> Option<String> {
    crate::git::patch::build_full_patch(file)
}

pub fn all_hunks(file: &FileDiff) -> Vec<usize> {
    (0..file.hunks.len()).collect()
}

/// Whether hunk-level actions make sense for this diff.
pub fn is_actionable(file: &FileDiff) -> bool {
    !file.is_binary
        && !file.hunks.is_empty()
        && matches!(
            file.status,
            ChangeKind::Modified | ChangeKind::Added | ChangeKind::Deleted
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::parse::parse_diff;

    #[test]
    fn differing_span_basic() {
        assert_eq!(differing_span("hello", "hello"), None);
        assert_eq!(differing_span("hello world", "hello there"), Some((6, 5)));
        assert_eq!(differing_span("abc", "abcdef"), Some((3, 3)));
        assert_eq!(differing_span("", "abc"), Some((0, 3)));
    }

    #[test]
    fn actionable_only_for_text_changes() {
        let diff = parse_diff(
            b"diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\n+b\n",
        )
        .unwrap();
        assert!(is_actionable(&diff[0]));
        let binary = parse_diff(
            b"diff --git a/f b/f\nBinary files a/f and b/f differ\n",
        )
        .unwrap();
        assert!(!is_actionable(&binary[0]));
    }
}
