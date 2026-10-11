use std::collections::HashMap;
use std::sync::mpsc;
use eframe::egui;
use egui::{
    Color32, FontId, RichText, Stroke, Vec2,
    text::LayoutJob,
};
use egui_extras::{Column, TableBuilder};
use crate::loader::{self, LoadResult, ParquetData};
use crate::table::TableState;

// ── Palette ───────────────────────────────────────────────────────────────────

struct Palette {
    bg:         Color32,
    surface:    Color32,
    surface2:   Color32,
    accent:     Color32,
    text:       Color32,
    muted:      Color32,
    row_alt:    Color32,
    row_hover:  Color32,
    header_bg:  Color32,
    border:     Color32,
    null:       Color32,
}

impl Palette {
    fn dark() -> Self {
        Self {
            bg:        Color32::from_rgb(24,  25,  30),
            surface:   Color32::from_rgb(32,  33,  40),
            surface2:  Color32::from_rgb(38,  40,  50),
            accent:    Color32::from_rgb(82,  145, 230),
            text:      Color32::from_rgb(220, 222, 228),
            muted:     Color32::from_rgb(120, 125, 145),
            row_alt:   Color32::from_rgb(30,  31,  38),
            row_hover: Color32::from_rgb(42,  48,  65),
            header_bg: Color32::from_rgb(28,  30,  42),
            border:    Color32::from_rgb(50,  52,  65),
            null:      Color32::from_rgb(80,  85,  105),
        }
    }

    fn light() -> Self {
        Self {
            bg:        Color32::from_rgb(248, 248, 252),
            surface:   Color32::from_rgb(255, 255, 255),
            surface2:  Color32::from_rgb(238, 239, 244),
            accent:    Color32::from_rgb(50,  120, 210),
            text:      Color32::from_rgb(25,  27,  35),
            muted:     Color32::from_rgb(110, 115, 135),
            row_alt:   Color32::from_rgb(242, 243, 247),
            row_hover: Color32::from_rgb(218, 228, 248),
            header_bg: Color32::from_rgb(230, 232, 242),
            border:    Color32::from_rgb(205, 207, 220),
            null:      Color32::from_rgb(170, 175, 195),
        }
    }
}

// ── App ───────────────────────────────────────────────────────────────────────

enum State {
    Empty,
    Loading,
    Loaded(ParquetData, TableState),
    Error(String),
}

#[derive(Clone, Debug)]
pub struct ColumnFilter {
    pub query: String,
    pub case_sensitive: bool,
}

#[derive(Clone, Debug)]
pub struct ColumnFilterDialog {
    pub col_idx: usize,
    pub col_name: String,
    pub col_dtype: String,
    pub query: String,
    pub case_sensitive: bool,
    pub focus_input: bool,
}

pub struct ParquetApp {
    state:       State,
    rx:          Option<mpsc::Receiver<LoadResult>>,
    search:      String,
    show_search: bool,
    dark_mode:   bool,
    notice:      Option<String>,

    // Column-name search (independent of the row-value search above)
    column_search:                String,
    show_column_search:           bool,
    column_search_active:         usize,
    column_search_scroll_pending: bool,

    // Per-column value filters and filter dialog state
    column_filters:               HashMap<usize, ColumnFilter>,
    filter_dialog:                Option<ColumnFilterDialog>,
}

impl ParquetApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial_file: Option<String>) -> Self {
        let dark_mode = cc.storage
            .and_then(|s| s.get_string("dark_mode"))
            .map(|v| v != "false")
            .unwrap_or(true);

        let mut app = Self {
            state:       State::Empty,
            rx:          None,
            search:      String::new(),
            show_search: false,
            dark_mode,
            notice:      None,

            column_search:                String::new(),
            show_column_search:           false,
            column_search_active:         0,
            column_search_scroll_pending: false,

            column_filters:               HashMap::new(),
            filter_dialog:                None,
        };
        let palette = if dark_mode { Palette::dark() } else { Palette::light() };
        style_egui(&cc.egui_ctx, &palette, dark_mode);
        if let Some(path) = initial_file {
            app.start_load(path);
        }
        app
    }

    fn start_load(&mut self, path: String) {
        self.state = State::Loading;
        self.search.clear();
        self.show_search = false;
        self.column_search.clear();
        self.show_column_search = false;
        self.column_search_active = 0;
        self.column_filters.clear();
        self.filter_dialog = None;
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        loader::load_async(path, tx);
    }

    fn poll_loader(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.rx {
            if let Ok(result) = rx.try_recv() {
                self.rx = None;
                match result {
                    LoadResult::Ok(data) => {
                        let ts = TableState::new(data.row_count);
                        self.state = State::Loaded(data, ts);
                    }
                    LoadResult::Err(e) => {
                        self.state = State::Error(e);
                    }
                }
                ctx.request_repaint();
            } else {
                ctx.request_repaint();
            }
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        ctx.input(|i| {
            if let Some(dropped) = i.raw.dropped_files.first() {
                if let Some(path) = &dropped.path {
                    let p = path.to_string_lossy().to_string();
                    if p.to_lowercase().ends_with(".parquet") || p.to_lowercase().ends_with(".parq") {
                        self.start_load(p);
                    }
                }
            }
        });
    }

    /// Open or close the column-name search bar. Mirrors the row-search
    /// toggle in `update()`'s Ctrl+F handler.
    fn toggle_column_search(&mut self) {
        self.show_column_search = !self.show_column_search;
        if self.show_column_search {
            self.column_search_scroll_pending = true;
        } else {
            self.column_search.clear();
        }
    }

    /// Register the app for .parquet/.parq and open Windows' Default-apps
    /// settings so the user can finish setting it as the default handler.
    #[cfg(windows)]
    fn set_default_app(&mut self) {
        match crate::assoc::register() {
            Ok(()) => {
                self.notice = Some(
                    "Registered .parquet and .parq. In the Settings window that opened, set \
                     Fast Parquet Viewer as the default — or right-click a file, choose \
                     \"Open with\", then \"Choose another app\"."
                        .to_string(),
                );
                let _ = std::process::Command::new("explorer")
                    .arg("ms-settings:defaultapps")
                    .spawn();
            }
            Err(e) => {
                self.notice = Some(format!("Couldn't register file types: {e}"));
            }
        }
    }

    fn show_column_filter_dialog(&mut self, ctx: &egui::Context, palette: &Palette) {
        if let Some(dialog) = &mut self.filter_dialog {
            let mut is_open = true;
            let mut should_close = false;
            let mut clear_clicked = false;
            let col_idx = dialog.col_idx;
            let col_name = dialog.col_name.clone();
            let col_dtype = dialog.col_dtype.clone();

            let total_rows = if let State::Loaded(data, _) = &self.state { data.row_count } else { 0 };
            let matches_count = if let State::Loaded(data, _) = &self.state {
                if dialog.query.is_empty() {
                    total_rows
                } else {
                    let q_lower = dialog.query.to_lowercase();
                    data.rows.iter().filter(|row| {
                        let val = row.get(col_idx).map(String::as_str).unwrap_or("");
                        if dialog.case_sensitive {
                            val.contains(&dialog.query)
                        } else {
                            val.to_lowercase().contains(&q_lower)
                        }
                    }).count()
                }
            } else { 0 };

            egui::Window::new(format!("Filter Column: {col_name}"))
                .id(egui::Id::new("column_filter_dialog_window"))
                .open(&mut is_open)
                .collapsible(false)
                .resizable(false)
                .default_width(320.0)
                .pivot(egui::Align2::CENTER_CENTER)
                .default_pos(ctx.screen_rect().center())
                .show(ctx, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);

                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Column:").color(palette.muted).size(12.0));
                        ui.label(RichText::new(&col_name).strong().size(13.0).color(palette.text));
                        ui.label(RichText::new(format!("({col_dtype})")).monospace().size(11.0).color(palette.muted));
                    });

                    ui.add_space(2.0);

                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Filter:").color(palette.muted).size(12.0));
                        let text_edit = ui.add(
                            egui::TextEdit::singleline(&mut dialog.query)
                                .desired_width(220.0)
                                .hint_text("type value to search…")
                                .font(FontId::monospace(13.0))
                        );
                        if dialog.focus_input {
                            text_edit.request_focus();
                            dialog.focus_input = false;
                        }
                        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            should_close = true;
                        }
                        if !dialog.query.is_empty() && ui.small_button("×").clicked() {
                            dialog.query.clear();
                        }
                    });

                    ui.horizontal(|ui| {
                        ui.checkbox(&mut dialog.case_sensitive, "Case sensitive");
                    });

                    ui.add_space(2.0);
                    let count_text = format!("{} / {} rows matching", fmt_num(matches_count), fmt_num(total_rows));
                    ui.label(RichText::new(count_text).color(palette.accent).size(12.0));

                    ui.add_space(6.0);
                    ui.separator();

                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("Done").size(12.0)).clicked() {
                            should_close = true;
                        }
                        if ui.button(RichText::new("Clear").size(12.0)).clicked() {
                            clear_clicked = true;
                        }
                    });
                });

            if clear_clicked {
                dialog.query.clear();
                self.column_filters.remove(&col_idx);
                should_close = true;
            } else if !dialog.query.is_empty() {
                self.column_filters.insert(col_idx, ColumnFilter {
                    query: dialog.query.clone(),
                    case_sensitive: dialog.case_sensitive,
                });
            } else {
                self.column_filters.remove(&col_idx);
            }

            if !is_open || should_close {
                self.filter_dialog = None;
            }
        }
    }
}

impl eframe::App for ParquetApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string("dark_mode", self.dark_mode.to_string());
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let palette = if self.dark_mode { Palette::dark() } else { Palette::light() };

        self.poll_loader(ctx);
        self.handle_dropped_files(ctx);

        // Keyboard shortcuts
        ctx.input(|i| {
            if i.key_pressed(egui::Key::F) && i.modifiers.ctrl && !i.modifiers.shift {
                self.show_search = !self.show_search;
                if !self.show_search { self.search.clear(); }
            }
            if i.key_pressed(egui::Key::F) && i.modifiers.ctrl && i.modifiers.shift {
                self.toggle_column_search();
            }
            if i.key_pressed(egui::Key::Escape) {
                if self.filter_dialog.is_some() {
                    self.filter_dialog = None;
                } else if self.show_search || self.show_column_search {
                    self.show_search = false;
                    self.search.clear();
                    self.show_column_search = false;
                    self.column_search.clear();
                }
            }
        });

        // Column search: match list is rebuilt each frame from the current
        // query; kept separate from the row-search `filtered` list below.
        let mut column_matches: Vec<usize> = Vec::new();

        // Top menu bar
        egui::TopBottomPanel::top("menubar")
            .frame(egui::Frame::new().fill(palette.surface).inner_margin(egui::Margin::symmetric(12, 6)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.visuals_mut().widgets.inactive.fg_stroke = Stroke::new(1.0_f32, palette.text);
                    if ui.add(egui::Button::new(
                        RichText::new("Open…").color(palette.text).size(13.0)
                    ).frame(false)).clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Parquet", &["parquet", "parq"])
                            .pick_file()
                        {
                            self.start_load(path.to_string_lossy().to_string());
                        }
                    }

                    // Ctrl+O
                    ctx.input(|i| {
                        if i.key_pressed(egui::Key::O) && i.modifiers.ctrl {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("Parquet", &["parquet", "parq"])
                                .pick_file()
                            {
                                let _ = path;
                            }
                        }
                    });

                    if ui.add(egui::Button::new(
                        RichText::new("Find column…").color(palette.text).size(13.0)
                    ).frame(false)).clicked() {
                        self.toggle_column_search();
                    }

                    ui.add_space(4.0);
                    ui.add(egui::Separator::default().vertical().spacing(8.0));
                    ui.add_space(4.0);

                    ui.label(RichText::new("Ctrl+O  open   Ctrl+F  search   Ctrl+Shift+F  find column").color(palette.muted).size(11.0));

                    #[cfg(windows)]
                    {
                        ui.add_space(4.0);
                        ui.add(egui::Separator::default().vertical().spacing(8.0));
                        ui.add_space(4.0);
                        if ui.add(egui::Button::new(
                            RichText::new("Set as default app…").color(palette.text).size(13.0)
                        ).frame(false)).clicked() {
                            self.set_default_app();
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let label = if self.dark_mode { "☀ Light" } else { "🌙 Dark" };
                        if ui.add(egui::Button::new(
                            RichText::new(label).color(palette.muted).size(12.0)
                        ).frame(false)).clicked() {
                            self.dark_mode = !self.dark_mode;
                            let new_palette = if self.dark_mode { Palette::dark() } else { Palette::light() };
                            style_egui(ctx, &new_palette, self.dark_mode);
                        }
                    });
                });
            });

        // Status bar
        egui::TopBottomPanel::bottom("statusbar")
            .frame(egui::Frame::new().fill(palette.surface).inner_margin(egui::Margin::symmetric(12, 5)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    match &self.state {
                        State::Empty => {
                            ui.label(RichText::new("Drop a .parquet file here or use Open…").color(palette.muted).size(12.0));
                        }
                        State::Loading => {
                            ui.spinner();
                            ui.add_space(6.0);
                            ui.label(RichText::new("Loading…").color(palette.muted).size(12.0));
                        }
                        State::Loaded(data, ts) => {
                            let global_q = if self.show_search && !self.search.is_empty() {
                                Some(self.search.as_str())
                            } else {
                                None
                            };
                            let visible = count_matching_rows(data, ts, global_q, &self.column_filters);
                            let size_str = fmt_size(data.file_size);
                            let has_filter = global_q.is_some() || !self.column_filters.is_empty();
                            let row_info = if has_filter {
                                format!("{} / {} rows", fmt_num(visible), fmt_num(data.row_count))
                            } else {
                                format!("{} rows", fmt_num(visible))
                            };
                            ui.label(
                                RichText::new(format!(
                                    "{}  ×  {} cols   │   {}   │   {}",
                                    row_info, data.col_count, size_str, data.file_path
                                ))
                                .color(palette.muted)
                                .size(12.0)
                            );
                        }
                        State::Error(e) => {
                            ui.label(RichText::new(format!("Error: {e}")).color(Color32::from_rgb(230, 80, 80)).size(12.0));
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let resp = ui.add(
                            egui::Label::new(
                                RichText::new("Created by Jakob Aungiers")
                                    .color(palette.muted)
                                    .size(11.0)
                            ).sense(egui::Sense::click())
                        ).on_hover_cursor(egui::CursorIcon::PointingHand);
                        if resp.clicked() {
                            ui.ctx().open_url(egui::OpenUrl::new_tab("https://jakob-aungiers.com"));
                        }
                    });
                });
            });

        // Search bar
        if self.show_search {
            egui::TopBottomPanel::top("searchbar")
                .frame(egui::Frame::new().fill(palette.surface2).inner_margin(egui::Margin::symmetric(12, 6)))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Filter:").color(palette.muted).size(12.0));
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut self.search)
                                .desired_width(320.0)
                                .hint_text("type to filter rows…")
                                .font(FontId::monospace(13.0))
                        );
                        resp.request_focus();
                        if !self.search.is_empty() {
                            if ui.small_button("×").clicked() {
                                self.search.clear();
                            }
                        }
                    });
                });
        }

        // Column search bar
        if self.show_column_search {
            egui::TopBottomPanel::top("columnsearchbar")
                .frame(egui::Frame::new().fill(palette.surface2).inner_margin(egui::Margin::symmetric(12, 6)))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Column:").color(palette.muted).size(12.0));
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut self.column_search)
                                .desired_width(240.0)
                                .hint_text("search column name…")
                                .font(FontId::monospace(13.0))
                        );
                        resp.request_focus();

                        // Rebuild the match list against this frame's (possibly
                        // just-edited) query so the count/highlight stay in sync.
                        if let State::Loaded(data, _) = &self.state {
                            if !self.column_search.is_empty() {
                                let q = self.column_search.to_lowercase();
                                column_matches = data.columns.iter().enumerate()
                                    .filter(|(_, c)| c.name.to_lowercase().contains(&q))
                                    .map(|(i, _)| i)
                                    .collect();
                            }
                        }

                        if resp.changed() {
                            self.column_search_active = 0;
                            self.column_search_scroll_pending = true;
                        }
                        self.column_search_active = if column_matches.is_empty() {
                            0
                        } else {
                            self.column_search_active.min(column_matches.len() - 1)
                        };

                        let count = column_matches.len();
                        let pos = if count == 0 { 0 } else { self.column_search_active + 1 };
                        ui.label(RichText::new(format!("{pos} / {count}")).color(palette.muted).size(12.0));

                        if ui.small_button("◀").clicked() && count > 0 {
                            self.column_search_active = (self.column_search_active + count - 1) % count;
                            self.column_search_scroll_pending = true;
                        }
                        if ui.small_button("▶").clicked() && count > 0 {
                            self.column_search_active = (self.column_search_active + 1) % count;
                            self.column_search_scroll_pending = true;
                        }

                        if !self.column_search.is_empty() && ui.small_button("×").clicked() {
                            self.column_search.clear();
                        }
                    });
                });

            // Enter / Shift+Enter cycle through matches while the bar is open.
            if !column_matches.is_empty() {
                let n = column_matches.len();
                ctx.input(|i| {
                    if i.key_pressed(egui::Key::Enter) {
                        self.column_search_active = if i.modifiers.shift {
                            (self.column_search_active + n - 1) % n
                        } else {
                            (self.column_search_active + 1) % n
                        };
                        self.column_search_scroll_pending = true;
                    }
                });
            }
        } else {
            self.column_search_active = 0;
        }

        let active_col = column_matches.get(self.column_search_active).copied();
        let column_scroll_pending = self.column_search_scroll_pending;
        self.column_search_scroll_pending = false;

        // Column filter chips bar
        if !self.column_filters.is_empty() {
            egui::TopBottomPanel::top("column_filter_chips")
                .frame(egui::Frame::new().fill(palette.surface2).inner_margin(egui::Margin::symmetric(12, 6)))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Column filters:").color(palette.muted).size(12.0));
                        let mut to_remove = None;
                        if let State::Loaded(data, _) = &self.state {
                            for (&col_idx, filter) in &self.column_filters {
                                let col_name = data.columns.get(col_idx).map(|c| c.name.as_str()).unwrap_or("col");
                                let text = format!("{}: \"{}\"", col_name, filter.query);
                                if ui.button(RichText::new(format!("{text} ✕")).size(12.0).color(palette.accent)).clicked() {
                                    to_remove = Some(col_idx);
                                }
                            }
                        }
                        if let Some(col_idx) = to_remove {
                            self.column_filters.remove(&col_idx);
                        }
                        if ui.small_button("Clear all").clicked() {
                            self.column_filters.clear();
                        }
                    });
                });
        }

        // Notice strip (file-association feedback)
        if let Some(msg) = self.notice.clone() {
            egui::TopBottomPanel::top("notice")
                .frame(egui::Frame::new().fill(palette.surface2).inner_margin(egui::Margin::symmetric(12, 6)))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(msg).color(palette.text).size(12.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("×").clicked() {
                                self.notice = None;
                            }
                        });
                    });
                });
        }

        // Central panel
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(palette.bg))
            .show(ctx, |ui| {
                match &self.state {
                    State::Empty   => draw_drop_zone(ui, &palette),
                    State::Loading => draw_loading(ui, &palette),
                    State::Error(e) => draw_error(ui, &palette, e),
                    State::Loaded(_, _) => {}
                }
            });

        let mut open_filter_dialog = None;

        // Draw table separately so we can mutate self
        if let State::Loaded(data, ts) = &mut self.state {
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(palette.bg))
                .show(ctx, |ui| {
                    draw_table(
                        ui, &palette, data, ts, &self.search, self.show_search,
                        &self.column_filters, &mut open_filter_dialog,
                        &column_matches, active_col, column_scroll_pending,
                    );
                });
        }

        if let Some((col_idx, col_name, col_dtype)) = open_filter_dialog {
            let existing_filter = self.column_filters.get(&col_idx);
            let query = existing_filter.map(|f| f.query.clone()).unwrap_or_default();
            let case_sensitive = existing_filter.map(|f| f.case_sensitive).unwrap_or(false);
            self.filter_dialog = Some(ColumnFilterDialog {
                col_idx,
                col_name,
                col_dtype,
                query,
                case_sensitive,
                focus_input: true,
            });
        }

        self.show_column_filter_dialog(ctx, &palette);
    }
}

// ── Table rendering ───────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)] // row-search and column-search state are independent concerns
fn draw_table(
    ui: &mut egui::Ui,
    p: &Palette,
    data: &ParquetData,
    ts: &mut TableState,
    search: &str,
    show_search: bool,
    column_filters: &HashMap<usize, ColumnFilter>,
    open_filter_dialog: &mut Option<(usize, String, String)>,
    column_matches: &[usize],
    active_col: Option<usize>,
    scroll_to_active_col: bool,
) {
    let global_query = if show_search && !search.is_empty() {
        Some(search.to_lowercase())
    } else {
        None
    };

    let filtered: Vec<usize> = ts.row_order.iter().copied().filter(|&ri| {
        row_matches_filters(&data.rows[ri], global_query.as_deref(), column_filters)
    }).collect();

    let col_count = data.col_count;
    let row_count = filtered.len();

    let text_height = ui.text_style_height(&egui::TextStyle::Body);
    let row_height = text_height + 8.0;
    let header_height = 48.0;

    let mut sort_request: Option<usize> = None;

    egui::ScrollArea::horizontal()
        .id_salt("table_hscroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
    let mut builder = TableBuilder::new(ui)
        .striped(false)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::exact(64.0))
        .resizable(true);

    for col_idx in 0..col_count {
        let meta = &data.columns[col_idx];
        let header_text_len = meta.name.len().max(meta.dtype.len());
        let initial_w = (header_text_len as f32 * 9.0 + 48.0).max(120.0);
        let min_w = (header_text_len as f32 * 8.0 + 36.0).max(50.0);
        builder = builder.column(Column::initial(initial_w).at_least(min_w).clip(true));
    }

    // Trailing spacer column: ensures the last data column can be resized freely
    // and scrolled into view without hitting the right viewport boundary!
    builder = builder.column(Column::remainder().at_least(140.0));

    builder
        .header(header_height, |mut header| {
            header.col(|ui| {
                ui.painter().rect_filled(ui.available_rect_before_wrap(), 0.0, p.header_bg);
                ui.add_space(2.0);
            });

            for col_idx in 0..col_count {
                let meta = &data.columns[col_idx];
                let is_sorted = ts.sort_col == Some(col_idx);
                let sort_asc = ts.sort_asc;
                let is_column_match = column_matches.contains(&col_idx);
                let is_active_column_match = active_col == Some(col_idx);
                let active_filter = column_filters.get(&col_idx);

                header.col(|ui| {
                    let rect = ui.available_rect_before_wrap();
                    ui.painter().rect_filled(rect, 0.0, p.header_bg);
                    if active_filter.is_some() {
                        // Gentle accent tint for columns with active filters
                        ui.painter().rect_filled(
                            rect, 0.0,
                            Color32::from_rgba_premultiplied(
                                p.accent.r(), p.accent.g(), p.accent.b(), 35,
                            ),
                        );
                    }
                    if is_column_match {
                        // Subtle amber tint, same hue as row-search match highlighting.
                        ui.painter().rect_filled(
                            rect, 0.0,
                            Color32::from_rgba_premultiplied(255, 200, 50, 40),
                        );
                    }
                    ui.painter().line_segment(
                        [rect.right_top(), rect.right_bottom()],
                        Stroke::new(1.0_f32, p.border),
                    );
                    ui.painter().line_segment(
                        [rect.left_bottom(), rect.right_bottom()],
                        Stroke::new(1.0_f32, p.border),
                    );
                    if is_active_column_match {
                        ui.painter().rect_stroke(rect, 0.0, Stroke::new(2.0_f32, p.accent), egui::StrokeKind::Inside);
                        if scroll_to_active_col {
                            ui.scroll_to_rect(rect, Some(egui::Align::Center));
                        }
                    }

                    let response = ui.allocate_rect(rect, egui::Sense::click());
                    if response.hovered() {
                        ui.painter().rect_filled(rect, 0.0, p.row_hover);
                    }
                    if response.clicked() {
                        sort_request = Some(col_idx);
                    }
                    if response.secondary_clicked() {
                        *open_filter_dialog = Some((col_idx, meta.name.clone(), meta.dtype.clone()));
                    }

                    if let Some(f) = active_filter {
                        response.on_hover_text(format!(
                            "Filter active: \"{}\"\nRight-click to edit filter\nLeft-click to sort",
                            f.query
                        ));
                    } else {
                        response.on_hover_text(
                            "Right-click to filter column\nLeft-click to sort"
                        );
                    }

                    let pad = 8.0;
                    let name_rect = egui::Rect::from_min_size(
                        rect.min + Vec2::new(pad, 6.0),
                        Vec2::new(rect.width() - pad * 2.0 - 16.0, 20.0),
                    );
                    let dtype_rect = egui::Rect::from_min_size(
                        rect.min + Vec2::new(pad, 26.0),
                        Vec2::new(rect.width() - pad * 2.0, 16.0),
                    );

                    let name_color = if is_sorted { p.accent } else { p.text };
                    let header_display = if active_filter.is_some() {
                        format!("{} 🔍", meta.name)
                    } else {
                        meta.name.clone()
                    };
                    ui.painter().text(
                        name_rect.left_center(),
                        egui::Align2::LEFT_CENTER,
                        &header_display,
                        FontId::new(13.0, egui::FontFamily::Proportional),
                        name_color,
                    );

                    ui.painter().text(
                        dtype_rect.left_center(),
                        egui::Align2::LEFT_CENTER,
                        &meta.dtype,
                        FontId::new(11.0, egui::FontFamily::Monospace),
                        p.muted,
                    );

                    if is_sorted {
                        let arrow = if sort_asc { "▲" } else { "▼" };
                        ui.painter().text(
                            egui::Pos2::new(rect.right() - 14.0, rect.top() + 14.0),
                            egui::Align2::CENTER_CENTER,
                            arrow,
                            FontId::new(10.0, egui::FontFamily::Monospace),
                            p.accent,
                        );
                    }
                });
            }

            // Trailing spacer column header
            header.col(|ui| {
                let rect = ui.available_rect_before_wrap();
                ui.painter().rect_filled(rect, 0.0, p.header_bg);
                ui.painter().line_segment(
                    [rect.left_bottom(), rect.right_bottom()],
                    Stroke::new(1.0_f32, p.border),
                );
            });
        })
        .body(|body| {
            body.rows(row_height, row_count, |mut row| {
                let row_idx = row.index();
                let data_row = filtered[row_idx];
                let bg = if row_idx % 2 == 0 { p.bg } else { p.row_alt };

                row.col(|ui| {
                    let rect = ui.available_rect_before_wrap();
                    ui.painter().rect_filled(rect, 0.0, p.surface);
                    ui.painter().line_segment(
                        [rect.right_top(), rect.right_bottom()],
                        Stroke::new(1.0_f32, p.border),
                    );
                    ui.painter().text(
                        rect.right_center() - Vec2::new(8.0, 0.0),
                        egui::Align2::RIGHT_CENTER,
                        data_row.to_string(),
                        FontId::new(11.0, egui::FontFamily::Monospace),
                        p.muted,
                    );
                });

                for col_idx in 0..col_count {
                    let meta = &data.columns[col_idx];
                    let active_filter = column_filters.get(&col_idx);

                    row.col(|ui| {
                        let rect = ui.available_rect_before_wrap();
                        ui.painter().rect_filled(rect, 0.0, bg);
                        ui.painter().line_segment(
                            [rect.right_top(), rect.right_bottom()],
                            Stroke::new(1.0_f32, p.border),
                        );

                        let cell_resp = ui.allocate_rect(rect, egui::Sense::click());
                        if cell_resp.secondary_clicked() {
                            *open_filter_dialog = Some((col_idx, meta.name.clone(), meta.dtype.clone()));
                        }

                        let cell_val = data.rows[data_row].get(col_idx).map(String::as_str).unwrap_or("");
                        let is_null = cell_val.is_empty();

                        if is_null {
                            ui.painter().text(
                                rect.left_center() + Vec2::new(6.0, 0.0),
                                egui::Align2::LEFT_CENTER,
                                "null",
                                FontId::new(12.0, egui::FontFamily::Monospace),
                                p.null,
                            );
                        } else {
                            if let Some(f) = active_filter {
                                if !f.query.is_empty() {
                                    let matches = if f.case_sensitive {
                                        cell_val.contains(&f.query)
                                    } else {
                                        cell_val.to_lowercase().contains(&f.query.to_lowercase())
                                    };
                                    if matches {
                                        let mut job = LayoutJob::default();
                                        highlight_matches(&mut job, cell_val, &f.query, p.text);
                                        ui.painter().galley(
                                            rect.left_center() + Vec2::new(6.0, 0.0),
                                            ui.fonts(|fonts| fonts.layout_job(job)),
                                            Color32::WHITE,
                                        );
                                        return;
                                    }
                                }
                            }

                            if let Some(q) = &global_query {
                                if cell_val.to_lowercase().contains(q.as_str()) {
                                    let mut job = LayoutJob::default();
                                    highlight_matches(&mut job, cell_val, q, p.text);
                                    ui.painter().galley(
                                        rect.left_center() + Vec2::new(6.0, 0.0),
                                        ui.fonts(|fonts| fonts.layout_job(job)),
                                        Color32::WHITE,
                                    );
                                    return;
                                }
                            }

                            ui.painter().text(
                                rect.left_center() + Vec2::new(6.0, 0.0),
                                egui::Align2::LEFT_CENTER,
                                cell_val,
                                FontId::new(12.0, egui::FontFamily::Proportional),
                                p.text,
                            );
                        }
                    });
                }

                // Trailing spacer row cell
                row.col(|ui| {
                    let rect = ui.available_rect_before_wrap();
                    ui.painter().rect_filled(rect, 0.0, bg);
                });
            });
        });

    }); // end ScrollArea::horizontal

    if let Some(col) = sort_request {
        ts.sort_by(col, &data.rows);
    }
}

fn row_matches_filters(
    row: &[String],
    global_query: Option<&str>,
    column_filters: &HashMap<usize, ColumnFilter>,
) -> bool {
    if let Some(q) = global_query {
        if !row.iter().any(|c| c.to_lowercase().contains(q)) {
            return false;
        }
    }
    for (&col_idx, filter) in column_filters {
        if filter.query.is_empty() {
            continue;
        }
        let cell = row.get(col_idx).map(String::as_str).unwrap_or("");
        if filter.case_sensitive {
            if !cell.contains(&filter.query) {
                return false;
            }
        } else {
            let q_lower = filter.query.to_lowercase();
            if !cell.to_lowercase().contains(&q_lower) {
                return false;
            }
        }
    }
    true
}

fn count_matching_rows(
    data: &ParquetData,
    ts: &TableState,
    global_query: Option<&str>,
    column_filters: &HashMap<usize, ColumnFilter>,
) -> usize {
    if global_query.is_none() && column_filters.is_empty() {
        return data.row_count;
    }
    let gq_lower = global_query.map(|q| q.to_lowercase());
    ts.row_order.iter().filter(|&&ri| {
        row_matches_filters(&data.rows[ri], gq_lower.as_deref(), column_filters)
    }).count()
}

// ── Highlight search matches ──────────────────────────────────────────────────

fn highlight_matches(job: &mut LayoutJob, text: &str, query: &str, text_color: Color32) {
    let lower = text.to_lowercase();
    let q_lower = query.to_lowercase();
    if q_lower.is_empty() {
        job.append(text, 0.0, egui::TextFormat {
            font_id: FontId::new(12.0, egui::FontFamily::Proportional),
            color: text_color,
            ..Default::default()
        });
        return;
    }
    let mut last = 0usize;
    let mut search_start = 0usize;

    while let Some(pos) = lower[search_start..].find(&q_lower) {
        let abs = search_start + pos;
        if abs > last {
            job.append(&text[last..abs], 0.0, egui::TextFormat {
                font_id: FontId::new(12.0, egui::FontFamily::Proportional),
                color: text_color,
                ..Default::default()
            });
        }
        let end = (abs + q_lower.len()).min(text.len());
        job.append(&text[abs..end], 0.0, egui::TextFormat {
            font_id: FontId::new(12.0, egui::FontFamily::Proportional),
            color: Color32::BLACK,
            background: Color32::from_rgb(255, 200, 50),
            ..Default::default()
        });
        last = end;
        search_start = end;
        if search_start >= lower.len() { break; }
    }
    if last < text.len() {
        job.append(&text[last..], 0.0, egui::TextFormat {
            font_id: FontId::new(12.0, egui::FontFamily::Proportional),
            color: text_color,
            ..Default::default()
        });
    }
}

// ── Empty / loading / error states ───────────────────────────────────────────

fn draw_drop_zone(ui: &mut egui::Ui, p: &Palette) {
    ui.centered_and_justified(|ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(80.0);
            ui.label(RichText::new("⬇").size(56.0).color(p.border));
            ui.add_space(16.0);
            ui.label(RichText::new("Drop a .parquet file here").size(20.0).color(p.muted));
            ui.add_space(8.0);
            ui.label(RichText::new("or use  Open…  in the menu bar").size(13.0).color(p.null));
        });
    });
}

fn draw_loading(ui: &mut egui::Ui, p: &Palette) {
    ui.centered_and_justified(|ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(100.0);
            ui.spinner();
            ui.add_space(16.0);
            ui.label(RichText::new("Loading…").size(16.0).color(p.muted));
        });
    });
}

fn draw_error(ui: &mut egui::Ui, p: &Palette, msg: &str) {
    ui.centered_and_justified(|ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(80.0);
            ui.label(RichText::new("×").size(48.0).color(Color32::from_rgb(200, 70, 70)));
            ui.add_space(12.0);
            ui.label(RichText::new("Failed to open file").size(18.0).color(p.text));
            ui.add_space(8.0);
            ui.label(RichText::new(msg).size(12.0).color(p.muted).monospace());
        });
    });
}

// ── Styling ───────────────────────────────────────────────────────────────────

fn style_egui(ctx: &egui::Context, p: &Palette, dark: bool) {
    let mut style = (*ctx.style()).clone();

    style.visuals.dark_mode = dark;
    style.visuals.panel_fill = p.bg;
    style.visuals.window_fill = p.surface;
    style.visuals.faint_bg_color = p.row_alt;
    style.visuals.extreme_bg_color = p.bg;
    style.visuals.override_text_color = Some(p.text);
    style.visuals.widgets.noninteractive.bg_fill = p.surface;
    style.visuals.widgets.inactive.bg_fill = p.surface2;
    style.visuals.widgets.hovered.bg_fill = p.row_hover;
    style.visuals.widgets.active.bg_fill = p.accent;
    style.visuals.selection.bg_fill = Color32::from_rgba_premultiplied(
        p.accent.r(), p.accent.g(), p.accent.b(), 60,
    );
    style.visuals.selection.stroke = Stroke::new(1.0_f32, p.accent);
    style.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.text);
    style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, p.text);

    style.spacing.item_spacing = Vec2::new(8.0, 4.0);
    style.spacing.button_padding = Vec2::new(8.0, 4.0);
    style.spacing.scroll = egui::style::ScrollStyle::solid();

    ctx.set_style(style);
}

// ── Utilities ─────────────────────────────────────────────────────────────────

fn fmt_size(n: u64) -> String {
    let mut v = n as f64;
    for unit in &["B", "KB", "MB", "GB"] {
        if v < 1024.0 {
            return if *unit == "B" { format!("{v:.0} {unit}") } else { format!("{v:.1} {unit}") };
        }
        v /= 1024.0;
    }
    format!("{v:.1} TB")
}

fn fmt_num(n: usize) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 { result.push(','); }
        result.push(ch);
    }
    result.chars().rev().collect()
}
