//! Rendering for the library browser.

use std::fmt::Write as _;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};
use ratatui_bubbletea_theme::{BubbleTheme, Palette, Symbols};

use crate::app::{App, Focus, ScanMessage};
use crate::entry::{Best, Entry, Ranked};
use dxray_core::analysis::Finding;

const BACKGROUND: Color = Color::Rgb(8, 15, 24);
const FOREGROUND: Color = Color::Rgb(226, 232, 240);
const MUTED: Color = Color::Rgb(148, 163, 184);
const ACCENT: Color = Color::Rgb(34, 211, 238);
const BORDER: Color = Color::Rgb(96, 165, 250);
const WARNING: Color = Color::Rgb(250, 204, 21);
const ERROR: Color = Color::Rgb(248, 113, 113);

fn background_style() -> Style {
    Style::new().fg(FOREGROUND).bg(BACKGROUND)
}

fn theme() -> BubbleTheme {
    let mut theme = BubbleTheme::new(
        Palette {
            foreground: FOREGROUND,
            muted: MUTED,
            accent: ACCENT,
            border: BORDER,
            focused_border: ACCENT,
            success: ACCENT,
            warning: WARNING,
            error: ERROR,
            selected_background: BACKGROUND,
        },
        Symbols::default(),
    );
    theme.text = theme.text.bg(BACKGROUND);
    theme.muted = theme.muted.bg(BACKGROUND);
    theme.accent = theme.accent.bg(BACKGROUND);
    theme.success = theme.success.bg(BACKGROUND);
    theme.warning = theme.warning.bg(BACKGROUND);
    theme.error = theme.error.bg(BACKGROUND);
    theme.border = theme.border.bg(BACKGROUND);
    theme.focused_border = theme.focused_border.bg(BACKGROUND);
    theme.title = theme.accent.add_modifier(Modifier::BOLD);
    theme.selected = theme.accent.add_modifier(Modifier::REVERSED);
    theme.help_key = theme.help_key.bg(BACKGROUND);
    theme.help_desc = theme.help_desc.bg(BACKGROUND);
    theme
}

pub(crate) fn render(app: &App, frame: &mut ratatui::Frame<'_>) {
    if frame.area().width == 0 || frame.area().height == 0 {
        return;
    }
    frame.render_widget(Block::new().style(background_style()), frame.area());
    if frame.area().width < 32 || frame.area().height < 12 {
        frame.render_widget(
            Paragraph::new("Resize: 32x12 min")
                .style(Style::new().fg(WARNING).bg(BACKGROUND))
                .wrap(Wrap { trim: false }),
            frame.area(),
        );
        return;
    }
    let theme = theme();
    let areas = crate::layout::areas(frame.area());

    render_header(app, &theme, frame, areas.header);
    if frame.area().width >= crate::layout::SPLIT_WIDTH || app.focus == Focus::List {
        render_list(app, &theme, frame, areas.list, frame.area().width);
    }
    if frame.area().width >= crate::layout::SPLIT_WIDTH || app.focus == Focus::Detail {
        render_detail(app, &theme, frame, areas.detail);
    }
    let evidence_help = if app.evidence_expanded {
        "Enter collapse evidence"
    } else {
        "Enter expand evidence"
    };
    if app.focus == Focus::Detail && app.selected_entry().is_some() {
        let help = if frame.area().width < crate::layout::SPLIT_WIDTH {
            format!("{evidence_help}\nTab/Shift-Tab pane · ↑↓ scroll\nPgUp/PgDn · Esc · Ctrl-C")
        } else if frame.area().width < 160 {
            format!(
                "{evidence_help} · Tab/Shift-Tab pane · ↑↓ scroll · PgUp/PgDn page\nHome/End ends · Type to filter · Backspace edit · Esc clear/quit · Ctrl-C quit"
            )
        } else {
            format!(
                "{evidence_help} · Tab/Shift-Tab pane · ↑↓ scroll · PgUp/PgDn page · Home/End ends · Backspace edit · Esc clear/quit · Ctrl-C quit"
            )
        };
        render_if_visible(frame, Paragraph::new(help).style(theme.muted), areas.help);
        return;
    }
    if frame.area().width < 160 {
        let help = if frame.area().width < crate::layout::SPLIT_WIDTH {
            "Tab/Shift-Tab pane · ↑↓ move\nPgUp/PgDn page · Home/End ends\nEsc clear/quit · Ctrl-C quit"
        } else {
            "Tab/Shift-Tab pane · ↑↓ move/scroll · PgUp/PgDn page · Home/End first/last\nType to filter · Backspace edit · Esc clear/quit · Ctrl-C quit"
        };
        render_if_visible(frame, Paragraph::new(help).style(theme.muted), areas.help);
        return;
    }
    render_if_visible(
        frame,
        Paragraph::new(theme.help_line([
            ("Esc", "clear/quit"),
            ("Ctrl-C", "quit"),
            ("Tab/Shift-Tab", "pane"),
            ("↑↓", "move/scroll"),
            ("PgUp/PgDn", "page"),
            ("Home/End", "first/last"),
            ("Backspace", "edit"),
        ]))
        .style(theme.muted),
        areas.help,
    );
}

fn render_if_visible<W: ratatui::widgets::Widget>(
    frame: &mut ratatui::Frame<'_>,
    widget: W,
    area: ratatui::layout::Rect,
) {
    if area.width > 0 && area.height > 0 {
        frame.render_widget(widget, area);
    }
}

fn render_header(
    app: &App,
    theme: &BubbleTheme,
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
) {
    let problems = app
        .problems
        .iter()
        .filter(|message| matches!(message, ScanMessage::Problem(_)))
        .count();
    let notes = app.problems.len() - problems;
    if area.width < crate::layout::MEDIUM_HEADER_WIDTH {
        render_if_visible(
            frame,
            Paragraph::new(small_header_line(app, problems, notes, area.width, theme))
                .style(theme.text),
            area,
        );
        return;
    }
    if area.width < crate::layout::SPLIT_WIDTH {
        let lines = vec![
            Line::from(vec![
                Span::styled("dxray", theme.accent),
                Span::styled(
                    if app.finished {
                        " — scan complete"
                    } else {
                        " — scanning"
                    },
                    theme.text,
                ),
                Span::styled(
                    format!("  P:{problems}"),
                    if problems == 0 {
                        theme.muted
                    } else {
                        theme.error
                    },
                ),
                Span::styled(format!(" N:{notes}"), theme.muted),
                Span::styled(
                    format!("  E:{} L:{} R:{}", app.total(), app.libraries, app.roots),
                    theme.muted,
                ),
            ]),
            search_line(app, area.width, theme, true),
        ];
        render_if_visible(frame, Paragraph::new(lines).style(theme.text), area);
        return;
    }
    render_if_visible(
        frame,
        Paragraph::new(Line::from(vec![
            Span::styled("dxray", theme.accent),
            Span::raw(" — "),
            Span::styled(
                if app.finished {
                    "scan complete"
                } else {
                    "scanning"
                },
                theme.text,
            ),
            Span::raw("  "),
            Span::styled(
                format!("Problems: {problems}"),
                if problems == 0 {
                    theme.muted
                } else {
                    theme.error
                },
            ),
            Span::styled(format!("  Notes: {notes}"), theme.muted),
            Span::styled(
                format!(
                    "  Entries: {}  Libraries: {}  Roots: {}",
                    app.total(),
                    app.libraries,
                    app.roots
                ),
                theme.muted,
            ),
        ]))
        .style(theme.text),
        area,
    );
    if area.height >= 2 {
        render_if_visible(
            frame,
            Paragraph::new(search_line(app, area.width, theme, true)).style(theme.text),
            ratatui::layout::Rect::new(area.x, area.y.saturating_add(1), area.width, 1),
        );
    }
}

fn small_header_line(
    app: &App,
    problems: usize,
    notes: usize,
    width: u16,
    theme: &BubbleTheme,
) -> Line<'static> {
    let status = if app.finished { "done" } else { "scan" };
    let brand_and_status = format!("dxray {status} ");
    let diagnostics = format!("P{problems} N{notes} ");
    let results = format!("{}/{} [", app.filtered_len(), app.total());
    let query = if app.filter().is_empty() {
        "filter"
    } else {
        app.filter()
    };
    let available = usize::from(width).saturating_sub(
        Line::from(brand_and_status.as_str()).width()
            + Line::from(diagnostics.as_str()).width()
            + Line::from(results.as_str()).width()
            + 1,
    );
    Line::from(vec![
        Span::styled(brand_and_status, theme.accent),
        Span::styled(
            format!("P{problems}"),
            if problems == 0 {
                theme.muted
            } else {
                theme.error
            },
        ),
        Span::styled(format!(" N{notes} "), theme.muted),
        Span::styled(results, theme.accent),
        Span::styled(query_tail(query, available), theme.text),
        Span::styled("]", theme.muted),
    ])
}

fn query_tail(query: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let line = Line::from(query);
    if line.width() <= width {
        return query.to_owned();
    }

    let mut used = 1;
    let mut tail = Vec::new();
    let graphemes = line.styled_graphemes(Style::default()).collect::<Vec<_>>();
    for grapheme in graphemes.into_iter().rev() {
        let grapheme_width = Line::from(grapheme.symbol).width();
        if used + grapheme_width > width {
            break;
        }
        used += grapheme_width;
        tail.push(grapheme.symbol);
    }
    tail.reverse();
    format!("…{}", tail.concat())
}

fn search_line(app: &App, width: u16, theme: &BubbleTheme, counts: bool) -> Line<'static> {
    let prefix = if counts {
        format!("Search {}/{}: [", app.filtered_len(), app.total())
    } else {
        "Search: [".to_owned()
    };
    let suffix = if counts { "] · Esc clear" } else { "]" };
    let query = if app.filter().is_empty() {
        "type name or AppID"
    } else {
        app.filter()
    };
    let available = usize::from(width)
        .saturating_sub(Line::from(prefix.as_str()).width() + Line::from(suffix).width());
    Line::from(vec![
        Span::styled(prefix, theme.accent),
        Span::styled(query_tail(query, available), theme.text),
        Span::styled(suffix, theme.muted),
    ])
}

fn render_list(
    app: &App,
    theme: &BubbleTheme,
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
    terminal_width: u16,
) {
    let mut title = if app.filter().is_empty() {
        format!("Entries ({})", app.total())
    } else {
        format!("Entries ({} of {})", app.filtered_len(), app.total())
    };
    if app.focus == Focus::List {
        title.push_str(" · active");
    }
    let block =
        theme
            .titled_block(title)
            .style(theme.text)
            .border_style(if app.focus == Focus::List {
                theme.focused_border
            } else {
                theme.border
            });
    let inner = block.inner(area);
    render_if_visible(frame, block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let entries = app.visible_entries();
    if entries.is_empty() && !app.filter().is_empty() {
        render_if_visible(
            frame,
            Paragraph::new(format!(
                "No matches; Esc clears search.\n\"{}\"",
                query_tail(app.filter(), usize::from(inner.width).saturating_sub(2))
            ))
            .style(theme.muted),
            inner,
        );
        return;
    }
    let tabular = terminal_width >= crate::layout::SPLIT_WIDTH;
    let selected_position = app.selected_position();
    let boundary = app.evidence_group_len();
    let no_evidence = app.no_evidence_len();
    let mut lines = Vec::new();
    if tabular {
        lines.push(table_line(
            "Name",
            "Library",
            "Static result",
            usize::from(inner.width),
            theme.title,
            false,
        ));
    }
    for (visible_index, entry) in entries.into_iter().enumerate() {
        let position = app.offset + visible_index;
        if no_evidence > 0 && (position == boundary || (visible_index == 0 && position > boundary))
        {
            lines.push(
                Line::from(clip_text(
                    &format!("No game evidence ({no_evidence})"),
                    usize::from(inner.width),
                ))
                .style(theme.muted.add_modifier(Modifier::BOLD)),
            );
        }
        let is_selected = selected_position == Some(position);
        lines.push(if tabular {
            table_line(
                &entry.name,
                library_label(entry, true, usize::from(inner.width)),
                &static_result(entry),
                usize::from(inner.width),
                theme.text,
                is_selected,
            )
        } else {
            compact_list_line(entry, usize::from(inner.width), theme.text, is_selected)
        });
    }
    render_if_visible(frame, Paragraph::new(lines).style(theme.text), inner);
}

fn table_line(
    name: &str,
    library: &str,
    result: &str,
    width: usize,
    style: Style,
    selected: bool,
) -> Line<'static> {
    let marker = if selected { "› " } else { "  " };
    let separators = 6;
    let columns = width.saturating_sub(text_width(marker) + separators);
    let library_width = if width >= 72 { 14 } else { 7 }.min(columns);
    let result_width = if width >= 72 { 24 } else { 17 }.min(columns.saturating_sub(library_width));
    let name_width = columns.saturating_sub(library_width + result_width);
    let text = format!(
        "{marker}{} │ {} │ {}",
        padded_cell(name, name_width),
        padded_cell(library, library_width),
        padded_cell(result, result_width),
    );
    Line::from(text).style(if selected {
        style.add_modifier(Modifier::REVERSED)
    } else {
        style
    })
}

fn padded_cell(text: &str, width: usize) -> String {
    let text = clip_text(text, width);
    format!(
        "{text}{}",
        " ".repeat(width.saturating_sub(text_width(&text)))
    )
}

fn compact_list_line(entry: &Entry, width: usize, style: Style, selected: bool) -> Line<'static> {
    let marker = match (selected, width < 40) {
        (true, true) => "›",
        (false, true) => " ",
        (true, false) => "› ",
        (false, false) => "  ",
    };
    let separator = if width < 40 { " " } else { " · " };
    let available = width.saturating_sub(text_width(marker));
    let result = static_result(entry);
    let library = library_label(entry, false, width);
    let fixed = text_width(library) + text_width(&result) + 2 * text_width(separator);
    let name_width = available.saturating_sub(fixed).max(1).min(available);
    let name = clip_text(&entry.name, name_width);
    let mut text = format!("{marker}{name}{separator}{library}{separator}{result}");
    text = clip_text(&text, width);
    Line::from(text).style(if selected {
        style.add_modifier(Modifier::REVERSED)
    } else {
        style
    })
}

fn library_label(entry: &Entry, tabular: bool, width: usize) -> &str {
    let label = entry.origin.label();
    if label == "Steam" || (tabular && width >= 72) {
        label
    } else {
        label.split(" / ").next().unwrap_or(label)
    }
}

fn static_result(entry: &Entry) -> String {
    match &entry.best {
        Best::Unwalkable(_) => "not searched".to_owned(),
        Best::NoExecutable => "no executable".to_owned(),
        Best::Ranked(ranked) => match &ranked.verdict {
            Err(_) => "not searched".to_owned(),
            Ok(verdict) if verdict.renderers.is_empty() => "no API determined".to_owned(),
            Ok(verdict) => verdict
                .renderers
                .iter()
                .map(|finding| finding.name.as_str())
                .collect::<Vec<_>>()
                .join(" / "),
        },
    }
}

fn text_width(text: &str) -> usize {
    Line::from(text).width()
}

fn clip_text(text: &str, width: usize) -> String {
    if text_width(text) <= width {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let line = Line::from(text);
    let mut used = 1;
    let mut clipped = String::new();
    for grapheme in line.styled_graphemes(Style::default()) {
        let grapheme_width = Line::from(grapheme.symbol).width();
        if used + grapheme_width > width {
            break;
        }
        used += grapheme_width;
        clipped.push_str(grapheme.symbol);
    }
    clipped.push('…');
    clipped
}

/// The caveats that qualify the headline beside them, in the column that has
/// room for a word and not a sentence.
///
/// Drawn *before* the headline, because this column is narrow and the end of it
/// is what a narrow terminal drops. Between losing the tail of a verdict the
/// detail pane repeats in full and losing the line that says the verdict may be
/// about the wrong executable, the verdict is the cheaper thing to cut.
///
/// A verdict shown without them is a best executable presented as though the
/// evidence chose it, and a library view is exactly where a caveat gets
/// dropped. The words are `dxray --game`'s own — it says "tied with N others"
/// and "could not be searched in full" — because a second vocabulary for one
/// fact is the defect this project keeps finding in itself. The sentences
/// behind both are in the detail pane, under `Note:`, so a marker here is never
/// a mark with no explanation anywhere.
fn caveats(entry: &Entry) -> String {
    let mut out = match entry.carries_evidence {
        Some(true) => " · static evidence found",
        Some(false) => " · no static game evidence",
        None => " · evidence unavailable",
    }
    .to_owned();
    // Survey evidence and readability of the selected executable are distinct.
    if entry.carries_evidence.is_some()
        && (matches!(&entry.best, Best::Unwalkable(_))
            || matches!(&entry.best, Best::Ranked(ranked) if ranked.verdict.is_err()))
    {
        out.push_str(" · evidence unavailable");
    }
    if entry.tie.is_some() {
        out.push_str(" · tied");
    }
    if entry.incomplete {
        out.push_str(" · not searched in full");
    }
    out
}

fn render_detail(
    app: &App,
    theme: &BubbleTheme,
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
) {
    let mut title = "Details".to_owned();
    if app.focus == Focus::Detail {
        title.push_str(" · active");
    }
    let content_lines = detail_line_count(app);
    let viewport_rows = crate::layout::detail_rows(app.width, app.height);
    let max_offset = content_lines.saturating_sub(viewport_rows);
    if max_offset > 0 {
        let marker = match app.detail_offset {
            0 => "↓ more",
            offset if offset < max_offset => "↕ more",
            _ => "↑ more",
        };
        write!(title, " · {marker}").expect("writing to a String cannot fail");
        if area.width >= 60 {
            write!(
                title,
                " · {}/{}",
                app.detail_offset.saturating_add(1),
                content_lines
            )
            .expect("writing to a String cannot fail");
        }
    }
    let block =
        theme
            .titled_block(title)
            .style(theme.text)
            .border_style(if app.focus == Focus::Detail {
                theme.focused_border
            } else {
                theme.border
            });
    let inner = block.inner(area);
    render_if_visible(frame, block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let summary_height = crate::layout::detail_summary_rows(inner.height);
    let summary_area = ratatui::layout::Rect::new(inner.x, inner.y, inner.width, summary_height);
    render_if_visible(
        frame,
        Paragraph::new(summary_lines(
            app,
            usize::from(inner.width),
            summary_height == 1,
        ))
        .style(theme.text),
        summary_area,
    );
    let body_area = ratatui::layout::Rect::new(
        inner.x,
        inner.y.saturating_add(summary_height),
        inner.width,
        inner.height.saturating_sub(summary_height),
    );
    render_if_visible(
        frame,
        detail_body(app)
            .style(theme.text)
            .scroll((u16::try_from(app.detail_offset).unwrap_or(u16::MAX), 0)),
        body_area,
    );
}

fn summary_lines(app: &App, width: usize, compact: bool) -> Vec<Line<'static>> {
    app.selected_entry().map_or_else(
        || {
            if compact {
                return vec![Line::from(clip_text(
                    if app.filter().is_empty() {
                        "Selected: waiting for discovery"
                    } else {
                        "Selected: no filter match"
                    },
                    width,
                ))];
            }
            vec![
                Line::from("Identity / origin:").style(Modifier::BOLD),
                Line::from(if app.filter().is_empty() {
                    "Waiting for a game to be discovered."
                } else {
                    "No selected game matches the current filter."
                }),
            ]
        },
        |entry| {
            if compact {
                return vec![Line::from(compact_summary(entry, width))];
            }
            vec![
                Line::from(clip_text(
                    &format!(
                        "Identity / origin: {} ({}) · {}",
                        entry.name,
                        entry.id_label(),
                        entry.origin.label()
                    ),
                    width,
                ))
                .style(Modifier::BOLD),
                Line::from(clip_text(
                    &format!("Renderer (static): {}", entry.headline()),
                    width,
                ))
                .style(
                    Style::new()
                        .fg(ACCENT)
                        .bg(BACKGROUND)
                        .add_modifier(Modifier::BOLD),
                ),
            ]
        },
    )
}

fn compact_summary(entry: &Entry, width: usize) -> String {
    let separator = " · ";
    let available = width.saturating_sub(text_width(separator));
    let name_width = text_width(&entry.name);
    let renderer = entry.headline();
    let renderer_width = text_width(&renderer);
    let mut name_budget = available.div_ceil(2);
    let mut renderer_budget = available.saturating_sub(name_budget);
    if name_width < name_budget {
        name_budget = name_width;
        renderer_budget = available.saturating_sub(name_budget);
    } else if renderer_width < renderer_budget {
        renderer_budget = renderer_width;
        name_budget = available.saturating_sub(renderer_budget);
    }
    format!(
        "{}{}{}",
        clip_text(&entry.name, name_budget),
        separator,
        clip_text(&renderer, renderer_budget)
    )
}

#[cfg(test)]
fn panel_lines(app: &App) -> Vec<Line<'static>> {
    let mut lines = summary_lines(app, usize::MAX, false);
    lines.extend(body_lines(app));
    lines
}

// Rendering and scroll measurement share the same body.
fn body_lines(app: &App) -> Vec<Line<'static>> {
    let mut lines = app
        .selected_entry()
        .map_or_else(Vec::new, |entry| detail_lines(entry, app.evidence_expanded));
    if !app.problems.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from("Scan diagnostics:").style(Modifier::BOLD));
        lines.push(Line::from("Scope: entire scan"));
        lines.extend(app.problems.iter().map(|message| {
            let line = Line::from(format!("{}: {}", message.label(), message.text()));
            match message {
                ScanMessage::Problem(_) => line.style(Style::new().fg(ERROR).bg(BACKGROUND)),
                ScanMessage::Note(_) => line,
            }
        }));
    }
    lines
}

/// The body widget is also the source of its rendered line count. Keeping the
/// wrap configuration here makes scrolling use Ratatui's exact compositor.
fn detail_body(app: &App) -> Paragraph<'static> {
    Paragraph::new(body_lines(app)).wrap(Wrap { trim: false })
}

/// Visual rows in the current detail pane. The reducer uses this to keep a
/// scroll offset valid after selection, scan messages, and terminal resizes.
pub(crate) fn detail_line_count(app: &App) -> usize {
    detail_body(app).line_count(crate::layout::detail_width(app.width))
}

fn detail_lines(entry: &Entry, expanded: bool) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(format!("ID: {}", entry.id_label())),
        Line::from(format!("Source: {}", entry.origin.label())),
        Line::from(""),
    ];
    lines.push(
        Line::from(if expanded {
            "Evidence details [expanded]"
        } else {
            "Evidence details [collapsed]"
        })
        .style(Modifier::BOLD),
    );
    if expanded && let Best::Ranked(ranked) = &entry.best {
        render_verdict(&mut lines, ranked);
    }

    render_nvapi_policy(&mut lines, entry, expanded);
    if expanded {
        render_paths_and_ranking(&mut lines, entry);
    }
    render_advanced_diagnostics(&mut lines, entry);
    lines
}

fn render_nvapi_policy(lines: &mut Vec<Line<'static>>, entry: &Entry, expanded: bool) {
    lines.push(Line::from(""));
    lines.push(section_title("Proton / NVAPI"));
    lines.push(Line::from(format!("NVAPI: {}", entry.nvapi.verdict)));
    lines.push(muted_line("Policy does not establish runtime enablement."));
    if !expanded {
        return;
    }
    if let Some(script) = &entry.nvapi.script {
        lines.push(Line::from(format!("Proton script: {}", script.display())));
    }
    if !entry.nvapi.condition.is_empty() {
        lines.push(Line::from("NVAPI condition:"));
        lines.extend(
            entry
                .nvapi
                .condition
                .iter()
                .map(|line| Line::from(format!("  {line}"))),
        );
    }
}

fn render_paths_and_ranking(lines: &mut Vec<Line<'static>>, entry: &Entry) {
    lines.push(Line::from(""));
    lines.push(section_title("Paths / ranking"));
    lines.push(Line::from(format!(
        "Install: {}",
        entry.install_dir.display()
    )));
    lines.push(Line::from(format!("Library: {}", entry.library.display())));
    if let Best::Ranked(ranked) = &entry.best {
        lines.push(Line::from(format!(
            "Executable: {} (score {}, {} candidates)",
            ranked.path.display(),
            ranked.score,
            ranked.of
        )));
        lines.extend(
            ranked
                .reasons
                .iter()
                .map(|reason| Line::from(format!("• {reason}"))),
        );
    }
}

fn render_advanced_diagnostics(lines: &mut Vec<Line<'static>>, entry: &Entry) {
    lines.push(Line::from(""));
    lines.push(section_title("Advanced diagnostics"));
    let qualifications = caveats(entry);
    if !qualifications.is_empty() {
        lines.push(Line::from(format!("Analysis:{qualifications}")));
    }
    match &entry.best {
        Best::Ranked(ranked) => {
            if !ranked.has_evidence() {
                lines.push(Line::from(
                    "• nothing observed argues that this is the game",
                ));
            }
            if let Ok(verdict) = &ranked.verdict
                && verdict.renderers.is_empty()
            {
                lines.push(muted_line("Renderer unknown from static evidence."));
            }
            match &ranked.verdict {
                Ok(verdict) if verdict.is_empty() => lines.push(muted_line(
                    "Evidence: executable read; no recognised graphics findings.",
                )),
                Err(error) => lines.push(error_line(format!(
                    "Evidence unavailable: could not read executable: {error}"
                ))),
                Ok(_) => {}
            }
        }
        Best::NoExecutable => {
            lines.push(muted_line("No executable was found in this install."));
        }
        Best::Unwalkable(error) => {
            lines.push(error_line(format!("Could not read install: {error}")));
        }
    }
    if !entry.notes.is_empty() {
        lines.push(Line::from("Entry notes:").style(Modifier::BOLD));
        lines.extend(
            entry
                .notes
                .iter()
                .map(|note| Line::from(format!("Note: {note}"))),
        );
    }
}

fn section_title(title: &'static str) -> Line<'static> {
    Line::from(title).style(
        Style::new()
            .fg(ACCENT)
            .bg(BACKGROUND)
            .add_modifier(Modifier::BOLD),
    )
}

fn muted_line(text: impl Into<String>) -> Line<'static> {
    Line::from(text.into()).style(Style::new().fg(MUTED).bg(BACKGROUND))
}

fn error_line(text: impl Into<String>) -> Line<'static> {
    Line::from(text.into()).style(Style::new().fg(ERROR).bg(BACKGROUND))
}

/// Adds the evidence read from the selected executable without interpreting it
/// again. `dxray-core` owns the classification; the TUI only exposes every
/// finding and the observation that supports it.
fn render_verdict(lines: &mut Vec<Line<'static>>, ranked: &Ranked) {
    let Ok(verdict) = &ranked.verdict else {
        return;
    };
    if verdict.is_empty() {
        return;
    }
    lines.push(Line::from(""));
    lines.push(section_title("Renderer evidence"));
    lines.push(muted_line(
        "Imports do not prove the renderer used at runtime.",
    ));
    render_findings(lines, "Renderers", &verdict.renderers);
    render_findings(lines, "Infrastructure", &verdict.infrastructure);
    lines.push(Line::from(""));
    lines.push(section_title("Features / upscalers"));
    render_findings(lines, "Features", &verdict.features);
    lines.push(muted_line(
        "DLL presence does not prove a feature is enabled.",
    ));
    render_findings(lines, "Local overrides", &verdict.local_overrides);
}

/// Renders a complete verdict category, including empty categories. Keeping
/// them visible means a missing category is not confused with one that was
/// accidentally omitted by the detail view.
fn render_findings(lines: &mut Vec<Line<'static>>, category: &str, findings: &[Finding]) {
    lines.push(Line::from(format!("{category}:")));
    if findings.is_empty() {
        lines.push(Line::from("  none"));
        return;
    }

    for finding in findings {
        lines.push(Line::from(format!("  {}", finding.name)));
        // The same sentence `dxray --game` prints, from the same function.
        lines.extend(
            finding
                .signals
                .iter()
                .map(|signal| Line::from(format!("    {}", signal.describe()))),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::{io, path::PathBuf};

    use dxray_core::{Evidence, analyse};

    #[test]
    fn evidence_toggle_preserves_triage_and_scroll_across_sizes() {
        use crate::{Key, app::Focus};
        for width in [32, 60, 99, 100, 180] {
            let mut app = App::new(30);
            let mut entry = ranked_entry(Ok(analyse(&Evidence {
                imports: vec!["d3d12.dll".into()],
                ..Evidence::default()
            })));
            entry.notes.push("entry caveat".into());
            app.update(Msg::Resize(width, 30));
            app.update(Msg::Game(Box::new(entry)));
            app.update(Msg::test_problem("global diagnostic"));
            app.update(Msg::Key(Key::Enter));
            assert!(!app.evidence_expanded);
            app.update(Msg::Key(Key::Tab));
            assert_eq!(app.focus, Focus::Detail);
            let collapsed = super::panel_lines(&app)
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(collapsed.contains("Entry notes:"));
            assert!(collapsed.contains("Scope: entire scan"));
            assert!(!collapsed.contains("/games/tf2"));
            assert!(!collapsed.contains("d3d12.dll (import)"));
            assert!(draw(&app, width, 30).contains("Enter expand evidence"));
            app.update(Msg::Key(Key::Enter));
            let expanded = super::panel_lines(&app)
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(expanded.contains("/games/tf2"));
            assert!(expanded.contains("d3d12.dll (import)"));
            assert!(draw(&app, width, 30).contains("Enter collapse evidence"));
            app.update(Msg::Key(Key::PageDown));
            app.update(Msg::Resize(width, 13));
            app.update(Msg::Key(Key::End));
            assert!(draw(&app, width, 13).contains("global diagnostic"));
            app.update(Msg::Key(Key::Enter));
            assert!(!app.evidence_expanded);
            assert!(
                app.detail_offset
                    <= super::detail_line_count(&app)
                        .saturating_sub(crate::layout::detail_rows(width, 13))
            );
            app.update(Msg::Resize(width, 30));
            app.update(Msg::Key(Key::Home));
            assert!(draw(&app, width, 30).contains("Identity / origin:"));
        }
    }

    #[test]
    fn entry_without_pe_evidence_can_expand_paths() {
        use crate::Key;
        let mut app = App::new(50);
        app.update(Msg::Resize(180, 50));
        app.update(Msg::Game(Box::new(game())));
        app.update(Msg::Key(Key::Tab));
        assert!(draw(&app, 180, 50).contains("No executable was found"));
        app.update(Msg::Key(Key::Enter));
        let screen = draw(&app, 180, 50);
        assert!(screen.contains("Install: /games/tf2"));
        assert!(!screen.contains("score"));
        app.update(Msg::Key(Key::Enter));
        assert!(!draw(&app, 180, 50).contains("Install:"));
    }
    use ratatui::{
        Terminal,
        backend::TestBackend,
        buffer::Buffer,
        style::{Modifier, Style},
    };
    use ratatui_tea::Model;

    use super::{compact_list_line, render, text_width};
    use crate::{
        Msg,
        app::App,
        entry::{Best, Entry, Ranked},
    };
    use dxray_core::{launcher::Game, proton::Answer};

    fn game() -> Entry {
        Entry {
            identity: dxray_core::Identity::SteamApp(440),
            origin: dxray_core::steam::ORIGIN,
            name: "Team Fortress 2".to_owned(),
            install_dir: PathBuf::from("/games/tf2"),
            library: PathBuf::from("/steam"),
            best: Best::NoExecutable,
            notes: Vec::new(),
            tie: None,
            carries_evidence: None,
            incomplete: false,
            nvapi: Answer {
                script: None,
                verdict: "not configured".to_owned(),
                available: None,
                condition: Vec::new(),
            },
        }
    }

    fn draw(app: &App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(app, frame)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn assert_background(buffer: &Buffer, label: &str) {
        for y in buffer.area.y..buffer.area.bottom() {
            for x in buffer.area.x..buffer.area.right() {
                assert_eq!(
                    buffer[(x, y)].bg,
                    super::BACKGROUND,
                    "background at ({x}, {y}) in {label}"
                );
            }
        }
    }

    fn ranked_entry(verdict: Result<dxray_core::analysis::Verdict, String>) -> Entry {
        let mut entry = game();
        entry.best = Best::Ranked(Box::new(Ranked {
            path: PathBuf::from("/games/tf2/bin/game.exe"),
            score: 42,
            reasons: vec!["shipping executable".to_owned()],
            of: 2,
            verdict,
        }));
        entry
    }

    #[test]
    fn list_uses_factual_static_results_without_ids() {
        for (evidence, best, status) in [
            (Some(true), Best::NoExecutable, "no executable"),
            (
                Some(false),
                Best::Ranked(Box::new(Ranked {
                    path: PathBuf::from("game.exe"),
                    score: 0,
                    reasons: Vec::new(),
                    of: 1,
                    verdict: Ok(dxray_core::analysis::Verdict::default()),
                })),
                "no API determined",
            ),
            (
                None,
                Best::Unwalkable("directory is absent".into()),
                "not searched",
            ),
        ] {
            let mut entry = game();
            entry.name = "Example".into();
            entry.carries_evidence = evidence;
            entry.best = best;
            let mut app = App::new(24);
            app.update(Msg::Game(Box::new(entry)));
            for width in [32, 60, 80, 99, 100, 180] {
                app.update(Msg::Resize(width, 24));
                let selected = app.selected;
                let screen = draw(&app, width, 24);
                assert!(screen.contains("Entries (1)"), "{screen}");
                let row = screen.lines().find(|line| line.contains('›')).unwrap();
                let row = row.split("││").next().unwrap_or(row);
                assert!(row.contains(status), "{width}: {row}");
                assert!(!row.contains("440"), "{width}: {row}");
                assert!(!row.contains("static evidence found"), "{width}: {row}");
                if width >= 100 {
                    for heading in ["Name", "Library", "Static result"] {
                        assert!(screen.contains(heading), "{width}: {screen}");
                    }
                }
                app.set_filter("example");
                let screen = draw(&app, width, 24);
                assert!(screen.contains("Entries (1 of 1)"), "{screen}");
                assert_eq!(app.selected, selected);
                app.set_filter("absent");
                let screen = draw(&app, width, 24);
                assert!(screen.contains("Entries (0 of 1)"), "{screen}");
                assert!(screen.contains("No matches"));
                app.set_filter("");
            }
        }
    }

    #[test]
    fn responsive_list_keeps_name_library_and_result_visible() {
        let mut entry = game();
        entry.name = "Marvel's Guardians of the Galaxy".into();
        entry.identity = dxray_core::Identity::SteamApp(1_088_850);
        entry.carries_evidence = Some(true);
        let mut app = App::new(24);
        app.update(Msg::Game(Box::new(entry)));

        for width in [32, 60, 80, 99, 100, 180] {
            app.update(Msg::Resize(width, 24));
            let screen = draw(&app, width, 24);
            let row = screen.lines().find(|line| line.contains('›')).unwrap();
            let row = row.split("││").next().unwrap_or(row);
            assert!(row.contains('M'), "{width}: {row}");
            assert!(row.contains("no executable"), "{width}: {row}");
            assert!(!row.contains("1088850"), "{width}: {row}");
        }
    }

    #[test]
    fn list_clips_unicode_names_and_shortens_heroic_library() {
        let mut entry = game();
        entry.name = "Étoile 銀河 — a very long title".into();
        entry.origin = dxray_core::heroic::Store::Epic.origin();
        entry.identity = dxray_core::Identity::Native("abcdef0123456789abcdef0123456789".into());
        entry.carries_evidence = Some(false);
        let mut app = App::new(24);
        app.update(Msg::Game(Box::new(entry)));

        for width in [32, 60, 80, 99, 100, 180] {
            app.update(Msg::Resize(width, 24));
            let screen = draw(&app, width, 24);
            let row = screen.lines().find(|line| line.contains('›')).unwrap();
            let row = row.split("││").next().unwrap_or(row);
            assert!(row.contains('É'), "{width}: {row}");
            assert!(row.contains("no executable"), "{width}: {row}");
            assert!(!row.contains("abcdef"), "{width}: {row}");
            assert_eq!(
                row.contains("Heroic / Epic"),
                width >= 180,
                "{width}: {row}"
            );
        }
    }

    #[test]
    fn compact_list_line_fits_unicode_cell_budget() {
        let mut entry = game();
        entry.name = "銀河 Étoile".repeat(20);
        entry.origin = dxray_core::heroic::Store::Epic.origin();
        entry.identity = dxray_core::Identity::Native("id0123456789".repeat(8));
        for width in [30_usize, 58, 78, 97] {
            let line = compact_list_line(&entry, width, Style::default(), true);
            let text = line.to_string();
            assert!(text_width(&text) <= width, "{width}: {text}");
            assert!(text.contains('銀'), "{width}: {text}");
            assert!(text.contains("Heroic"), "{width}: {text}");
            assert!(text.contains("no executable"), "{width}: {text}");
            assert!(!text.contains("id012"), "{width}: {text}");
        }
    }

    #[test]
    fn list_preserves_found_evidence_when_selected_executable_is_unreadable() {
        let mut entry = ranked_entry(Err("permission denied".into()));
        entry.name = "Example".into();
        entry.carries_evidence = Some(true);
        entry.incomplete = true;
        let mut app = App::new(24);
        app.update(Msg::Game(Box::new(entry)));
        let screen = draw(&app, 400, 24);
        let row = screen.lines().find(|line| line.contains('›')).unwrap();
        let row = row.split("││").next().unwrap_or(row);
        assert!(row.contains("not searched"), "{row}");
        assert!(!row.contains("static evidence found"), "{row}");
        assert!(!row.contains("440"), "{row}");
    }

    #[test]
    fn no_evidence_section_preserves_order_selection_and_filtering() {
        let mut game_entry = game();
        game_entry.name = "Game".into();
        game_entry.carries_evidence = Some(true);
        let mut runtime = game();
        runtime.name = "Run one".into();
        runtime.carries_evidence = Some(false);
        let mut proton = game();
        proton.name = "Run two".into();
        proton.carries_evidence = Some(false);

        let mut app = App::new(12);
        app.update(Msg::Game(Box::new(runtime)));
        app.update(Msg::Game(Box::new(game_entry)));
        app.update(Msg::Game(Box::new(proton)));

        for width in [32, 60, 80, 99, 100, 180] {
            app.update(Msg::Resize(width, 12));
            let screen = draw(&app, width, 12);
            let game_at = screen.find("Game").unwrap();
            let section_at = screen.find("No game evidence (2)").unwrap();
            let runtime_at = screen.rfind("Run").unwrap();
            assert!(
                game_at < section_at && section_at < runtime_at,
                "{width}: {screen}"
            );
            if width >= 100 {
                assert!(screen.contains("Name"));
                assert!(screen.contains("Library"));
                assert!(screen.contains("Static result"));
            }
        }

        app.update(Msg::Key(crate::Key::End));
        assert_eq!(app.selected_entry().unwrap().name, "Run two");
        assert!(draw(&app, 100, 12).contains("No game evidence (2)"));

        for character in "run one".chars() {
            app.update(Msg::Key(crate::Key::Char(character)));
        }
        assert_eq!(app.filtered_len(), 1);
        assert_eq!(app.selected_entry().unwrap().name, "Run one");
        let filtered = draw(&app, 100, 12);
        assert!(filtered.contains("No game evidence (1)"));
        assert!(filtered.contains("Run one"));
    }

    #[test]
    fn no_evidence_selection_highlights_its_entry_when_viewport_starts_in_that_group() {
        let mut app = App::new(12);
        for name in ["Game one", "Game two"] {
            let mut entry = game();
            entry.name = name.into();
            entry.carries_evidence = Some(true);
            app.update(Msg::Game(Box::new(entry)));
        }
        for name in [
            "Runtime one",
            "Runtime two",
            "Runtime three",
            "Runtime four",
            "Runtime five",
            "Runtime six",
            "Chosen runtime",
        ] {
            let mut entry = game();
            entry.name = name.into();
            entry.carries_evidence = Some(false);
            app.update(Msg::Game(Box::new(entry)));
        }
        app.update(Msg::Resize(100, 12));
        app.update(Msg::Key(crate::Key::End));

        assert_eq!(app.selected_entry().unwrap().name, "Chosen runtime");
        assert!(
            app.offset > app.evidence_group_len(),
            "the viewport must start inside the no-evidence group"
        );

        let mut terminal = Terminal::new(TestBackend::new(100, 12)).unwrap();
        terminal.draw(|frame| render(&app, frame)).unwrap();
        let buffer = terminal.backend().buffer();
        let list = crate::layout::areas(buffer.area).list;
        let inner_x = list.x + 1;
        let inner_width = list.width.saturating_sub(2);
        let rows = (list.y + 1)..list.bottom().saturating_sub(1);
        let row_text = |y| {
            (inner_x..inner_x + inner_width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        };
        let table_row = rows
            .clone()
            .find(|&y| row_text(y).contains("Static result"))
            .expect("the table header is visible");
        let section_row = rows
            .clone()
            .find(|&y| row_text(y).contains("No game evidence (7)"))
            .expect("the group header is repeated above a viewport that starts in the group");
        let selected_row = rows
            .clone()
            .find(|&y| row_text(y).contains("Chosen runtime"))
            .expect("the selected entry is visible");
        assert!(table_row < section_row && section_row < selected_row);

        let reversed_rows = rows
            .filter(|&y| {
                (inner_x..inner_x + inner_width)
                    .any(|x| buffer[(x, y)].modifier.contains(Modifier::REVERSED))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            reversed_rows,
            [selected_row],
            "only the selected entry row may carry the selection highlight"
        );
    }

    fn missing_install_entry(kind: io::ErrorKind, message: &str) -> Entry {
        Entry::build(
            Game {
                identity: dxray_core::Identity::SteamApp(9999),
                name: "Still downloading".to_owned(),
                install_dir: PathBuf::from("/games/not-downloaded-yet"),
                origin: dxray_core::steam::ORIGIN,
            },
            PathBuf::from("/steam"),
            Err(io::Error::new(kind, message)),
            Answer {
                script: None,
                verdict: "not configured".to_owned(),
                available: None,
                condition: Vec::new(),
            },
        )
    }

    #[test]
    fn normal_view_renders_the_game_and_scan_messages() {
        let mut app = App::new(24);
        app.update(Msg::Game(Box::new(game())));
        app.update(Msg::test_problem(
            "could not read /steam/libraryfolders.vdf".to_owned(),
        ));
        app.update(Msg::test_note("library is declared twice".to_owned()));

        app.update(Msg::Resize(180, 50));
        let screen = draw(&app, 180, 50);
        assert!(screen.contains("Team Fortress 2"));
        assert!(screen.contains("Problem: could not read /steam/libraryfolders.vdf"));
        assert!(screen.contains("Note: library is declared twice"));
    }

    #[test]
    fn responsive_panels_preserve_focus_selection_and_page_geometry() {
        use crate::{app::Focus, key::Key};
        let mut app = App::new(24);
        let mut entry = game();
        entry.notes = (0..80).map(|i| format!("evidence line {i}")).collect();
        app.update(Msg::Game(Box::new(entry)));
        app.update(Msg::test_problem("unreadable library"));
        let selected = app.selected;
        for width in [32, 60, 99, 100, 120, 180, 40] {
            app.update(Msg::Resize(width, 24));
            app.focus = Focus::List;
            let list = draw(&app, width, 24);
            assert!(list.contains("Entries (1)"));
            assert_eq!(list.contains("Details"), width >= 100);
            if width < 60 {
                for text in ["dxray scan", "1/1", "[filter]", "P1 N0"] {
                    assert!(list.contains(text), "missing {text} at {width}");
                }
            } else if width < 100 {
                for text in [
                    "scanning",
                    "P:1 N:0",
                    "E:1 L:0 R:0",
                    "Search 1/1: [type name or AppID]",
                ] {
                    assert!(list.contains(text), "missing {text} at {width}");
                }
            }
            if width < 100 {
                for text in ["Tab/Shift-Tab pane", "Esc clear/quit"] {
                    assert!(list.contains(text), "missing {text} at {width}");
                }
            }
            app.update(Msg::Key(Key::Tab));
            assert_eq!(app.focus, Focus::Detail);
            app.update(Msg::Key(Key::Home));
            app.update(Msg::Key(Key::PageDown));
            assert_eq!(app.detail_offset, crate::layout::detail_rows(width, 24));
            let detail = draw(&app, width, 24);
            assert!(detail.contains("Details"));
            assert_eq!(detail.contains("Entries (1)"), width >= 100);
            app.update(Msg::Key(Key::End));
            app.update(Msg::Resize(80, 30));
            assert_eq!(app.focus, Focus::Detail);
            assert!(
                app.detail_offset
                    <= super::detail_line_count(&app)
                        .saturating_sub(crate::layout::detail_rows(80, 30))
            );
            app.update(Msg::Key(Key::BackTab));
            assert_eq!(app.selected, selected);
        }
    }

    #[test]
    fn search_field_reports_results_and_marks_overflow_at_all_supported_widths() {
        use crate::Key;
        for width in [32, 60, 99, 100, 180] {
            let mut app = App::new(24);
            app.update(Msg::Resize(width, 24));
            app.update(Msg::Game(Box::new(game())));
            assert!(draw(&app, width, 24).contains(if width < 60 {
                "[filter]"
            } else {
                "type name or AppID"
            }));
            for character in "不存在".repeat(100).chars().chain("END".chars()) {
                app.update(Msg::Key(Key::Char(character)));
            }
            let screen = draw(&app, width, 24);
            assert!(screen.contains("0/1"));
            assert!(screen.contains('…'));
            assert!(screen.contains("END]"));
            assert!(screen.contains("No matches; Esc clears search."));
            app.update(Msg::Key(Key::Escape));
            assert!(draw(&app, width, 24).contains("1/1"));
            assert!(!app.quitting);
            for character in "440".chars() {
                app.update(Msg::Key(Key::Char(character)));
            }
            assert!(draw(&app, width, 24).contains("[440]"));
            assert_eq!(app.filtered_len(), 1);
        }
    }

    #[test]
    fn clipping_keeps_unicode_graphemes_and_terminal_cell_budgets() {
        assert_eq!(super::query_tail("abcde\u{301}", 2), "…e\u{301}");
        assert_eq!(super::clip_text("e\u{301}xyz", 2), "e\u{301}…");
        assert_eq!(super::query_tail("xx👨‍👩‍👧‍👦", 3), "…👨‍👩‍👧‍👦");

        for clipped in [
            super::query_tail("銀河".repeat(20).as_str(), 5),
            super::clip_text("銀河".repeat(20).as_str(), 5),
        ] {
            assert!(super::text_width(&clipped) <= 5, "{clipped}");
        }
    }

    #[test]
    fn responsive_header_fits_and_keeps_essential_state_at_each_target_width() {
        for width in [32, 60, 80, 99, 100, 180] {
            let mut app = App::new(24);
            app.update(Msg::Resize(width, 24));
            app.update(Msg::Game(Box::new(game())));
            app.update(Msg::test_problem("unreadable library"));
            app.update(Msg::test_note("duplicate library"));
            app.set_filter(&format!("{}END", "銀河".repeat(80)));

            let screen = draw(&app, width, 24);
            let lines = screen.lines().collect::<Vec<_>>();
            let areas = crate::layout::areas(ratatui::layout::Rect::new(0, 0, width, 24));

            assert_eq!(lines.len(), 24, "renderer escaped its backend at {width}");
            assert!(lines[0].contains("dxray"), "{width}: {screen}");
            assert!(screen.contains("0/1"), "{width}: {screen}");
            assert!(screen.contains('…'), "{width}: {screen}");
            assert!(screen.contains("END]"), "{width}: {screen}");
            assert!(
                !lines[usize::from(areas.list.y)].contains("dxray")
                    && !lines[usize::from(areas.list.y)].contains("Search"),
                "header overlaps body at {width}:\n{screen}"
            );

            if width < 60 {
                assert_eq!(areas.header.height, 1);
                for text in ["scan", "P1", "N1"] {
                    assert!(
                        lines[0].contains(text),
                        "missing {text} at {width}: {screen}"
                    );
                }
            } else if width < 100 {
                assert_eq!(areas.header.height, 2);
                for text in ["scanning", "P:1", "N:1", "E:1 L:0 R:0"] {
                    assert!(
                        lines[0].contains(text),
                        "missing {text} at {width}: {screen}"
                    );
                }
                assert!(lines[1].contains("Search 0/1:"), "{width}: {screen}");
            } else {
                assert_eq!(areas.header.height, 2);
                for text in [
                    "scanning",
                    "Problems: 1",
                    "Notes: 1",
                    "Entries: 1  Libraries: 0  Roots: 0",
                ] {
                    assert!(
                        lines[0].contains(text),
                        "missing {text} at {width}: {screen}"
                    );
                }
                assert!(lines[1].contains("Search 0/1:"), "{width}: {screen}");
            }

            app.set_filter("");
            app.update(Msg::Key(crate::Key::Tab));
            app.update(Msg::Key(crate::Key::End));
            let diagnostics = draw(&app, width, 24);
            assert!(
                diagnostics.contains("Problem: unreadable library"),
                "diagnostic is not reachable with Tab then End at {width}: {diagnostics}"
            );
            assert!(
                diagnostics.contains("Note: duplicate library"),
                "note is not reachable with Tab then End at {width}: {diagnostics}"
            );
        }
    }

    #[test]
    fn scan_status_and_totals_leave_the_filter_visible() {
        let mut app = App::new(24);
        app.update(Msg::Game(Box::new(game())));
        app.update(Msg::Library(crate::msg::ScanLocation {
            origin: dxray_core::steam::ORIGIN,
            path: PathBuf::from("/steam"),
        }));
        let scanning = draw(&app, 120, 24);
        assert!(scanning.contains("scanning"));
        assert!(scanning.contains("Entries: 1  Libraries: 1  Roots: 0"));
        assert!(
            scanning
                .lines()
                .nth(1)
                .unwrap()
                .contains("Search 1/1: [type name or AppID]")
        );

        app.update(Msg::Finished);
        let screen = draw(&app, 100, 24);
        assert!(screen.contains("scan complete"));
        assert!(screen.contains("Entries: 1  Libraries: 1"));
        assert!(
            screen
                .lines()
                .nth(1)
                .unwrap()
                .contains("Search 1/1: [type name or AppID]")
        );
    }

    #[test]
    fn diagnostic_counts_are_global_and_do_not_invent_severities() {
        let mut app = App::new(24);
        app.update(Msg::Game(Box::new(game())));
        app.update(Msg::test_problem("unreadable library"));
        app.update(Msg::test_note("duplicate library"));
        app.update(Msg::test_note("missing metadata"));
        app.update(Msg::Key(crate::key::Key::Char('z')));
        let screen = draw(&app, 120, 24);
        let header = screen.lines().next().unwrap();
        assert!(header.contains("Problems: 1  Notes: 2"));
        assert!(!header.contains("Warning"));
        assert!(screen.contains("Search 0/1: [z]"));
        assert!(screen.contains("No matches; Esc clears search."));
        assert!(screen.contains("Problem: unreadable library"));
        assert!(screen.contains("Note: duplicate library"));
        assert!(!screen.contains("3 scan notes"));
    }

    #[test]
    fn focus_and_problem_color_have_textual_equivalents() {
        let mut app = App::new(24);
        app.update(Msg::test_problem("unreadable library"));
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        for focus in [crate::app::Focus::List, crate::app::Focus::Detail] {
            app.focus = focus;
            terminal.draw(|frame| render(&app, frame)).unwrap();
            let buffer = terminal.backend().buffer();
            let header: String = (0..120).map(|x| buffer[(x, 0)].symbol()).collect();
            let problem_column =
                u16::try_from(header.chars().position(|c| c == 'P').unwrap()).unwrap();
            assert_eq!(buffer[(problem_column, 0)].fg, super::ERROR);
            let areas = crate::layout::areas(buffer.area);
            assert_eq!(
                buffer[(areas.list.x, areas.list.y)].fg,
                if focus == crate::app::Focus::List {
                    super::ACCENT
                } else {
                    super::BORDER
                }
            );
            let screen = draw(&app, 180, 24);
            assert!(screen.contains(if focus == crate::app::Focus::List {
                "Entries (0) · active"
            } else {
                "Details · active"
            }));
            let footer = screen.lines().last().unwrap();
            for key in [
                "Esc",
                "Ctrl-C",
                "Tab/Shift-Tab",
                "↑↓",
                "PgUp/PgDn",
                "Home/End",
                "Backspace",
            ] {
                assert!(footer.contains(key));
            }
        }
    }

    #[test]
    fn explicit_background_covers_empty_cells_borders_and_responsive_regions() {
        use crate::app::Focus;

        let mut app = App::new(24);
        app.update(Msg::Game(Box::new(game())));
        for (width, height, focus) in [
            (32, 12, Focus::List),
            (60, 24, Focus::Detail),
            (99, 24, Focus::List),
            (100, 24, Focus::Detail),
            (180, 30, Focus::List),
        ] {
            app.update(Msg::Resize(width, height));
            app.focus = focus;
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| render(&app, frame)).unwrap();
            let buffer = terminal.backend().buffer();
            assert_background(buffer, &format!("{width}x{height} {focus:?}"));

            let areas = crate::layout::areas(buffer.area);
            for (label, point) in [
                ("header", (areas.header.right() - 1, areas.header.y)),
                ("panel border", (areas.list.x, areas.list.y)),
                (
                    "panel empty row",
                    (areas.list.x + 1, areas.list.bottom() - 2),
                ),
                ("footer", (areas.help.right() - 1, areas.help.bottom() - 1)),
            ] {
                assert_eq!(buffer[point].bg, super::BACKGROUND, "{label} at {width}");
            }
            assert_ne!(buffer[(areas.list.x, areas.list.y)].symbol(), " ");
            assert_eq!(
                buffer[(areas.list.x + 1, areas.list.bottom() - 2)].symbol(),
                " ",
                "expected an empty list cell at {width}"
            );

            if width >= crate::layout::SPLIT_WIDTH {
                assert_eq!(
                    buffer[(areas.detail.x, areas.detail.y)].bg,
                    super::BACKGROUND,
                    "detail border at {width}"
                );
                assert_eq!(
                    buffer[(areas.detail.right() - 2, areas.detail.bottom() - 2)].bg,
                    super::BACKGROUND,
                    "empty detail cell at {width}"
                );
            }
        }

        let mut terminal = Terminal::new(TestBackend::new(31, 12)).unwrap();
        terminal.draw(|frame| render(&app, frame)).unwrap();
        let buffer = terminal.backend().buffer();
        assert_background(buffer, "minimum-size warning");
        assert_eq!(buffer[(0, 0)].fg, super::WARNING);
        assert_eq!(buffer[(30, 11)].bg, super::BACKGROUND);
    }

    #[test]
    fn essential_text_structure_state_and_selection_keep_contrast() {
        let mut app = App::new(24);
        app.update(Msg::Game(Box::new(game())));
        app.update(Msg::test_problem("unreadable library"));
        app.update(Msg::Resize(120, 24));
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        terminal.draw(|frame| render(&app, frame)).unwrap();
        let buffer = terminal.backend().buffer();
        let areas = crate::layout::areas(buffer.area);

        assert_ne!(super::FOREGROUND, super::BACKGROUND);
        assert_ne!(super::MUTED, super::BACKGROUND);
        assert_ne!(super::ACCENT, super::BACKGROUND);
        assert_ne!(super::BORDER, super::BACKGROUND);
        assert_ne!(super::ERROR, super::BACKGROUND);
        assert_ne!(super::WARNING, super::BACKGROUND);
        assert_eq!(buffer[(areas.list.x, areas.list.y)].fg, super::ACCENT);
        assert_eq!(buffer[(areas.detail.x, areas.detail.y)].fg, super::BORDER);

        let row = ((areas.list.y + 1)..areas.list.bottom() - 1)
            .find(|&y| {
                (areas.list.x + 1..areas.list.right() - 1)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .contains("Team Fortress 2")
            })
            .expect("selected entry row");
        let selected = &buffer[(areas.list.x + 1, row)];
        assert_eq!(selected.fg, super::FOREGROUND);
        assert_eq!(selected.bg, super::BACKGROUND);
        assert!(selected.modifier.contains(Modifier::REVERSED));

        let header = (0..120)
            .map(|x| buffer[(x, 0)].symbol())
            .collect::<String>();
        let problem_x = u16::try_from(header.find("Problems").unwrap()).unwrap();
        assert_eq!(buffer[(problem_x, 0)].fg, super::ERROR);
        assert_eq!(buffer[(problem_x, 0)].bg, super::BACKGROUND);
    }

    #[test]
    fn entry_notes_and_scan_diagnostics_have_distinct_sections() {
        let mut app = App::new(60);
        let mut entry = game();
        entry.notes.push("selection remains uncertain".into());
        app.update(Msg::Game(Box::new(entry)));
        app.update(Msg::test_problem("unreadable library"));
        app.update(Msg::test_note("duplicate library"));
        for width in [60, 180] {
            app.update(Msg::Resize(width, 60));
            app.focus = crate::app::Focus::Detail;
            let screen = draw(&app, width, 60);
            let selected = screen.find("Identity / origin:").unwrap();
            let notes = screen.find("Entry notes:").unwrap();
            let local = screen.find("Note: selection remains uncertain").unwrap();
            let scan = screen.find("Scan diagnostics:").unwrap();
            let problem = screen.find("Problem: unreadable library").unwrap();
            let note = screen.find("Note: duplicate library").unwrap();
            assert!(selected < notes && notes < local && local < scan);
            assert!(scan < problem && problem < note);
            assert!(screen.contains("Scope: entire scan"));
            assert!(!screen.contains("Warning:"));
        }
    }

    #[test]
    fn absent_notes_and_diagnostics_do_not_render_empty_sections() {
        let mut app = App::new(50);
        app.update(Msg::Game(Box::new(game())));
        let screen = draw(&app, 180, 50);
        assert!(screen.contains("Identity / origin:"));
        assert!(!screen.contains("Entry notes:"));
        assert!(!screen.contains("Scan diagnostics:"));
        assert!(screen.contains("Problems: 0  Notes: 0"));
    }

    #[test]
    fn diagnostics_remain_reachable_after_scrolling_and_resizing() {
        let mut app = App::new(24);
        let mut entry = game();
        entry.notes = (0..60).map(|i| format!("entry caveat {i}")).collect();
        app.update(Msg::Game(Box::new(entry)));
        app.update(Msg::test_problem("scan failure"));
        app.update(Msg::test_note("scan note"));
        app.focus = crate::app::Focus::Detail;
        let selected = app.selected;
        for width in [180, 60, 32, 100] {
            app.update(Msg::Resize(width, 30));
            app.update(Msg::Key(crate::key::Key::End));
            let screen = draw(&app, width, 30);
            assert!(screen.contains("Scan diagnostics:"), "{screen}");
            assert!(screen.contains("Problem: scan failure"), "{screen}");
            assert!(screen.contains("Note: scan note"), "{screen}");
            assert_eq!(app.selected, selected);
            app.update(Msg::Key(crate::key::Key::Home));
            assert!(draw(&app, width, 30).contains("Identity / origin:"));
        }
    }

    #[test]
    fn a_tiny_viewport_draws_without_panic_or_out_of_bounds_geometry() {
        let mut app = App::new(1);
        app.update(Msg::Game(Box::new(game())));

        let screen = draw(&app, 1, 1);
        assert_eq!(screen.chars().count(), 1);
    }

    #[test]
    fn responsive_minimum_boundaries_preserve_focus_and_selection() {
        use crate::app::Focus;
        let mut app = App::new(12);
        app.update(Msg::Game(Box::new(game())));
        let selected = app.selected;
        for (width, height) in [(31, 12), (32, 11), (32, 12), (99, 11), (100, 11)] {
            app.update(Msg::Resize(width, height));
            for focus in [Focus::List, Focus::Detail] {
                app.focus = focus;
                let screen = draw(&app, width, height);
                let too_small = width < 32 || height < 12;
                assert_eq!(screen.contains("Resize: 32x12 min"), too_small);
                if !too_small {
                    assert!(screen.contains("[filter]"));
                    assert_eq!(screen.contains("Entries (1)"), focus == Focus::List);
                    assert_eq!(screen.contains("Details"), focus == Focus::Detail);
                }
                assert_eq!(app.selected, selected);
                assert_eq!(app.focus, focus);
            }
        }
    }

    #[test]
    fn detail_hierarchy_keeps_static_limits_and_long_evidence() {
        let mut entry = ranked_entry(Ok(analyse(&Evidence {
            imports: vec!["d3d12.dll".into()],
            neighbours: vec![
                "nvngx_dlss.dll".into(),
                "libxess.dll".into(),
                "sl.interposer.dll".into(),
            ],
            ..Evidence::default()
        })));
        entry.notes.push("selection remains uncertain".into());
        entry
            .nvapi
            .condition
            .push("original condition evidence".into());
        let mut app = App::new(80);
        app.update(Msg::Resize(180, 80));
        app.update(Msg::Game(Box::new(entry)));
        let collapsed = draw(&app, 180, 80);
        for title in [
            "Identity / origin:",
            "Renderer (static):",
            "Proton / NVAPI",
            "Advanced diagnostics",
        ] {
            assert_eq!(
                collapsed.matches(title).count(),
                1,
                "missing or duplicated collapsed title {title}:\n{collapsed}"
            );
        }
        for expanded_only in [
            "Renderer evidence",
            "Features / upscalers",
            "Paths / ranking",
            "nvngx_dlss.dll (neighbour)",
        ] {
            assert!(
                !collapsed.contains(expanded_only),
                "expanded evidence leaked into collapsed details: {expanded_only}:\n{collapsed}"
            );
        }

        app.evidence_expanded = true;
        let screen = draw(&app, 180, 80);
        let mut previous = 0;
        for section in [
            "Identity / origin:",
            "Team Fortress 2 (440)",
            "Renderer (static):",
            "Renderer evidence",
            "Renderers:",
            "Infrastructure:",
            "Features / upscalers",
            "Features:",
            "Proton / NVAPI",
            "NVAPI condition:",
            "Paths / ranking",
            "Executable:",
            "Advanced diagnostics",
            "Note: selection remains uncertain",
        ] {
            let position = screen
                .find(section)
                .unwrap_or_else(|| panic!("missing {section}:\n{screen}"));
            assert!(position >= previous, "out of order: {section}");
            previous = position;
        }
        for evidence in [
            "Imports do not prove the renderer used at runtime.",
            "DLL presence does not prove a feature is enabled.",
            "Policy does not establish runtime enablement.",
            "nvngx_dlss.dll (neighbour)",
            "libxess.dll (neighbour)",
            "sl.interposer.dll (neighbour)",
            "score 42, 2 candidates",
            "shipping executable",
            "original condition evidence",
        ] {
            assert_eq!(
                screen.matches(evidence).count(),
                1,
                "missing or duplicated {evidence}:\n{screen}"
            );
        }
        for title in [
            "Identity / origin:",
            "Renderer (static):",
            "Renderer evidence",
            "Features / upscalers",
            "Proton / NVAPI",
            "Paths / ranking",
            "Advanced diagnostics",
        ] {
            assert_eq!(
                screen.matches(title).count(),
                1,
                "duplicated title {title}:\n{screen}"
            );
        }
    }

    #[test]
    fn detail_shows_the_proton_script_and_verbatim_conditional_nvapi_lines() {
        let mut app = App::new(24);
        let mut entry = game();
        entry.nvapi = Answer {
            script: Some(PathBuf::from("/compatibilitytools.d/Proton/proton")),
            verdict: "not determined: NVAPI depends on a condition".to_owned(),
            available: None,
            condition: vec![
                "if not os.environ.get(\"PROTON_DISABLE_NVAPI_QUIRKS\"):".to_owned(),
                "    ret.add(\"disablenvapi\")".to_owned(),
            ],
        };
        app.update(Msg::Game(Box::new(entry)));

        app.evidence_expanded = true;
        let screen = draw(&app, 120, 40);
        assert!(screen.contains("NVAPI: not determined: NVAPI depends on a condition"));
        assert!(screen.contains("Proton script: /compatibilitytools.d/Proton/proton"));
        assert!(screen.contains("NVAPI condition:"));
        assert!(screen.contains("if not os.environ.get(\"PROTON_DISABLE_NVAPI_QUIRKS\"):"));
        assert!(screen.contains("ret.add(\"disablenvapi\")"));
    }

    #[test]
    fn detail_shows_every_verdict_category_and_signal_provenance() {
        let verdict = analyse(&Evidence {
            imports: vec!["D3D12.dll".to_owned(), "dxgi.dll".to_owned()],
            delay_imports: vec!["nvapi64.dll".to_owned()],
            neighbours: vec!["sl.interposer.dll".to_owned(), "d3d11.dll".to_owned()],
            ..Evidence::default()
        });
        let mut app = App::new(40);
        app.update(Msg::Game(Box::new(ranked_entry(Ok(verdict)))));

        app.evidence_expanded = true;
        let screen = draw(&app, 160, 80);
        assert!(screen.contains("Renderers:"));
        assert!(screen.contains("Direct3D 12"));
        assert!(screen.contains("D3D12.dll (import)"));
        assert!(screen.contains("Infrastructure:"));
        assert!(screen.contains("DXGI"));
        assert!(screen.contains("dxgi.dll (import)"));
        assert!(screen.contains("Features:"));
        assert!(screen.contains("NVAPI"));
        assert!(screen.contains("nvapi64.dll (delay-import)"));
        assert!(screen.contains("NVIDIA Streamline"));
        assert!(screen.contains("sl.interposer.dll (neighbour)"));
        assert!(screen.contains("Local overrides:"));
        assert!(screen.contains("d3d11.dll (neighbour)"));
    }

    #[test]
    fn detail_prints_the_same_versioned_signal_line_the_cli_prints() {
        // Two surfaces, one sentence. The defect this project has already fixed
        // twice is one rule with two implementations that drift, and a version
        // shown by `dxray --game` but not by the detail pane would be the third.
        use dxray_core::{FileVersion, Signal, Source, Version, VersionInfo};

        let mut verdict = analyse(&Evidence {
            neighbours: vec!["nvngx_dlss.dll".to_owned()],
            ..Evidence::default()
        });
        let stamped = Signal {
            library: "nvngx_dlss.dll".to_owned(),
            source: Source::Neighbour,
            version: FileVersion::Stamped(VersionInfo {
                file: Version {
                    major: 310,
                    minor: 2,
                    patch: 1,
                    build: 0,
                },
                product: Version {
                    major: 310,
                    minor: 2,
                    patch: 1,
                    build: 0,
                },
            }),
        };
        assert_eq!(
            stamped.describe(),
            "nvngx_dlss.dll (neighbour, 310.2.1.0)",
            "the one rendering both binaries share"
        );
        verdict.features[0].signals = vec![stamped];

        let mut app = App::new(40);
        app.update(Msg::Game(Box::new(ranked_entry(Ok(verdict)))));

        app.evidence_expanded = true;
        let screen = draw(&app, 160, 80);
        assert!(
            screen.contains("nvngx_dlss.dll (neighbour, 310.2.1.0)"),
            "the detail pane has to name the build, got:\n{screen}"
        );
    }

    #[test]
    fn detail_distinguishes_an_empty_verdict_from_an_unreadable_executable() {
        let mut empty = App::new(24);
        empty.update(Msg::Game(Box::new(ranked_entry(Ok(
            dxray_core::analysis::Verdict::default(),
        )))));
        let empty_screen = draw(&empty, 140, 30);
        assert!(
            empty_screen.contains("Evidence: executable read; no recognised graphics findings.")
        );
        assert!(!empty_screen.contains("Evidence unavailable:"));
        assert!(empty_screen.contains("Renderer unknown from static evidence."));

        let mut unreadable = App::new(24);
        unreadable.update(Msg::Game(Box::new(ranked_entry(Err(
            "invalid PE header".to_owned()
        )))));
        let unreadable_screen = draw(&unreadable, 140, 30);
        assert!(
            unreadable_screen
                .contains("Evidence unavailable: could not read executable: invalid PE header")
        );
        assert!(!unreadable_screen.contains("no recognised graphics findings"));
    }

    #[test]
    fn collapsed_and_expanded_details_keep_each_status_once() {
        let text = |entry: Entry, expanded: bool| {
            let mut app = App::new(40);
            app.update(Msg::Game(Box::new(entry)));
            app.evidence_expanded = expanded;
            super::panel_lines(&app)
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        };
        let cases = [
            (
                ranked_entry(Ok(dxray_core::analysis::Verdict::default())),
                "Evidence: executable read; no recognised graphics findings.",
            ),
            (
                ranked_entry(Err("invalid PE header".to_owned())),
                "Evidence unavailable: could not read executable: invalid PE header",
            ),
            {
                let mut entry = ranked_entry(Ok(analyse(&Evidence {
                    imports: vec!["d3d12.dll".into()],
                    ..Evidence::default()
                })));
                let Best::Ranked(ranked) = &mut entry.best else {
                    panic!("the fixture must have a ranked executable");
                };
                ranked.reasons.clear();
                (entry, "• nothing observed argues that this is the game")
            },
        ];

        for (entry, status) in cases {
            for expanded in [false, true] {
                let details = text(entry.clone(), expanded);
                assert_eq!(
                    details.matches(status).count(),
                    1,
                    "expanded={expanded}, status={status}:\n{details}"
                );
                assert!(details.contains(if expanded {
                    "Evidence details [expanded]"
                } else {
                    "Evidence details [collapsed]"
                }));
            }
        }
    }

    #[test]
    fn focused_detail_panel_renders_later_long_content_after_scrolling() {
        let mut app = App::new(12);
        let mut entry = ranked_entry(Ok(analyse(&Evidence {
            imports: vec!["d3d12.dll".into()],
            ..Evidence::default()
        })));
        entry.notes = (0..30)
            .map(|index| format!("scroll proof {index}"))
            .collect();
        app.update(Msg::Resize(120, 12));
        app.update(Msg::Game(Box::new(entry)));

        let initial = draw(&app, 120, 12);
        assert!(initial.contains("Entries (1) · active"));
        assert!(!initial.contains("scroll proof 20"));
        assert!(initial.contains("↓ more"));

        app.update(Msg::Key(crate::Key::Tab));
        app.update(Msg::Key(crate::Key::End));
        let scrolled = draw(&app, 120, 12);
        assert!(scrolled.contains("Details · active"));
        assert!(scrolled.contains("scroll proof 29"));
        assert!(scrolled.contains("Team Fortress 2 (440)"));
        assert!(scrolled.contains("Renderer (static): Direct3D 12"));
        assert!(scrolled.contains("↑ more"));
    }

    #[test]
    fn minimum_detail_end_uses_the_rendered_wrap_count() {
        let mut app = App::new(12);
        let entry = ranked_entry(Ok(analyse(&Evidence {
            imports: vec!["d3d12.dll".into()],
            ..Evidence::default()
        })));
        app.update(Msg::Resize(32, 12));
        app.update(Msg::Game(Box::new(entry)));
        // At the 30-cell inner width, Ratatui renders this diagnostic in two
        // rows. The former local approximation counted three because of the
        // four spaces, so End selected a blank row after the content.
        app.update(Msg::test_problem("aa    aaaaaaaaaaaaaaa aaaaaaaaaa FINAL"));
        app.update(Msg::Key(crate::Key::Tab));
        app.update(Msg::Key(crate::Key::Home));

        assert!(crate::layout::detail_rows(32, 12) > 0);
        let first = draw(&app, 32, 12);
        let first_rows = first.lines().collect::<Vec<_>>();
        let summary_row = first_rows
            .iter()
            .position(|line| line.contains("Renderer (static): Direct3D 12"))
            .unwrap_or_else(|| panic!("missing compact summary:\n{first}"));
        let id_row = first_rows
            .iter()
            .position(|line| line.contains("ID: 440"))
            .unwrap_or_else(|| panic!("missing complete ID:\n{first}"));
        assert_eq!(id_row, summary_row + 1, "overlapping rows:\n{first}");
        assert!(first.contains("↓ more"), "{first}");

        app.update(Msg::Key(crate::Key::End));
        assert_eq!(
            app.detail_offset,
            super::detail_line_count(&app) - crate::layout::detail_rows(32, 12)
        );
        let last = draw(&app, 32, 12);
        let last_rows = last.lines().collect::<Vec<_>>();
        let summary_row = last_rows
            .iter()
            .position(|line| line.contains("Renderer (static): Direct3D 12"))
            .unwrap_or_else(|| panic!("missing compact summary:\n{last}"));
        let problem_row = last_rows
            .iter()
            .position(|line| line.contains("aaaaaaaaaa FINAL"))
            .unwrap_or_else(|| panic!("missing final wrapped diagnostic row:\n{last}"));
        assert!(problem_row > summary_row, "overlapping rows:\n{last}");
        assert!(
            problem_row
                < usize::from(
                    crate::layout::areas(ratatui::layout::Rect::new(0, 0, 32, 12))
                        .help
                        .y
                ),
            "diagnostic escaped the detail panel:\n{last}"
        );
        assert!(last.contains("↑ more"), "{last}");
    }

    #[test]
    fn small_details_pin_identity_and_primary_result_while_the_body_scrolls() {
        for width in [32, 60, 99, 100] {
            let mut app = App::new(13);
            let mut entry = ranked_entry(Ok(analyse(&Evidence {
                imports: vec!["d3d12.dll".into()],
                ..Evidence::default()
            })));
            entry.notes = (0..30).map(|index| format!("long note {index}")).collect();
            app.update(Msg::Resize(width, 13));
            app.update(Msg::Game(Box::new(entry)));
            app.update(Msg::test_problem("last scroll row"));
            app.update(Msg::Key(crate::Key::Tab));

            let first = draw(&app, width, 13);
            assert!(
                first.contains("Identity / origin: Team"),
                "{width}:\n{first}"
            );
            assert!(
                first.contains("Renderer (static): Direct3D 12"),
                "{width}:\n{first}"
            );
            assert!(first.contains("↓ more"), "{width}:\n{first}");

            app.update(Msg::Key(crate::Key::End));
            let last = draw(&app, width, 13);
            assert!(last.contains("Identity / origin: Team"), "{width}:\n{last}");
            assert!(
                last.contains("Renderer (static): Direct3D 12"),
                "{width}:\n{last}"
            );
            assert!(
                last.contains("Problem: last scroll row"),
                "{width}:\n{last}"
            );
            assert!(last.contains("↑ more"), "{width}:\n{last}");
        }
    }

    #[test]
    fn the_list_column_carries_the_caveats_that_qualify_the_verdict_beside_it() {
        // Both are computed on the scanning thread and were drawn nowhere. A
        // browser that shows "Direct3D 12" while silently dropping "this was a
        // coin flip between two executables" is the one surface in the project
        // where a caveat goes quiet.
        let mut app = App::new(24);
        let mut entry = ranked_entry(Ok(analyse(&Evidence::default())));
        entry.tie = Some(
            "tied with 1 other at the top; the evidence does not choose between them".to_owned(),
        );
        entry.incomplete = true;
        entry.notes = vec![
            "tied with 1 other at the top; the evidence does not choose between them".to_owned(),
        ];
        app.update(Msg::Game(Box::new(entry)));

        let screen = draw(&app, 160, 40);
        assert!(
            screen.contains("· tied"),
            "the tie is drawn beside the headline, got:
{screen}"
        );
        assert!(
            screen.contains("· not searched in full"),
            "and so is the walk that did not finish, got:
{screen}"
        );
        assert!(
            screen.contains("Note: tied with 1 other at the top"),
            "with the sentence behind the marker still under it, got:
{screen}"
        );
    }

    #[test]
    fn an_install_that_argues_nothing_is_grouped_and_explained_in_detail() {
        let mut app = App::new(24);
        let entry = Entry::build(
            Game {
                identity: dxray_core::Identity::SteamApp(1_493_710),
                name: "Proton Experimental".to_owned(),
                install_dir: PathBuf::from("/steam/common/Proton"),
                origin: dxray_core::steam::ORIGIN,
            },
            PathBuf::from("/steam"),
            Ok(dxray_core::game::Survey::ranked(
                vec![dxray_core::game::Candidate {
                    path: PathBuf::from("/steam/common/Proton/proton"),
                    reasons: Vec::new(),
                }],
                Vec::new(),
            )),
            Answer {
                script: None,
                verdict: "not configured".to_owned(),
                available: None,
                condition: Vec::new(),
            },
        );
        app.update(Msg::Game(Box::new(entry)));

        let screen = draw(&app, 160, 40);
        assert!(
            screen.contains("Proton Experimental"),
            "nothing is hidden, got:
{screen}"
        );
        assert!(
            screen.contains("No game evidence (1)"),
            "the section remains visible, got:
{screen}"
        );
        assert!(
            !screen.contains("tool"),
            "and it never names a category, got:
{screen}"
        );
        assert!(
            screen.contains("nothing observed argues that this is the game"),
            "with the sentence behind the marker under it, got:
{screen}"
        );
    }

    #[test]
    fn the_markers_are_drawn_in_the_order_they_qualify_the_headline() {
        // The order is the claim `caveats` documents: `no evidence` first,
        // because it qualifies the headline hardest, then the tie, then the
        // partial walk. Every other assertion in this file asks whether a
        // marker is on the screen, which three markers in any order satisfy —
        // so this one asks where they are relative to each other.
        let mut app = App::new(24);
        let mut entry = Entry::build(
            Game {
                identity: dxray_core::Identity::SteamApp(1_493_710),
                name: "Proton Experimental".to_owned(),
                install_dir: PathBuf::from("/steam/common/Proton"),
                origin: dxray_core::steam::ORIGIN,
            },
            PathBuf::from("/steam"),
            Ok(dxray_core::game::Survey::ranked(
                vec![dxray_core::game::Candidate {
                    path: PathBuf::from("/steam/common/Proton/proton"),
                    reasons: Vec::new(),
                }],
                Vec::new(),
            )),
            Answer {
                script: None,
                verdict: "not configured".to_owned(),
                available: None,
                condition: Vec::new(),
            },
        );
        assert!(entry.lacks_evidence(), "the row has to carry all three");
        entry.tie = Some(
            "tied with 1 other at the top; the evidence does not choose between them".to_owned(),
        );
        entry.incomplete = true;
        app.update(Msg::Game(Box::new(entry)));

        let screen = draw(&app, 240, 24);
        assert!(
            screen.contains(
                " · no static game evidence · evidence unavailable · tied · not searched in full"
            ),
            "the three markers are drawn in that order and next to each other, got:\n{screen}"
        );
    }

    #[test]
    fn an_unreadable_install_is_not_marked_as_carrying_no_evidence() {
        // It was never asked. The headline already says the directory could not
        // be read, and a second marker claiming nothing was observed would make
        // an unanswered question look like an answer.
        let mut app = App::new(24);
        app.update(Msg::Game(Box::new(missing_install_entry(
            io::ErrorKind::NotFound,
            "directory is absent",
        ))));

        let screen = draw(&app, 160, 40);
        assert!(
            !screen.contains("no static game evidence"),
            "got:
{screen}"
        );
        assert!(
            screen.contains("directory unreadable"),
            "got:
{screen}"
        );
    }

    #[test]
    fn a_game_whose_caveats_are_clear_carries_no_markers() {
        // The marker has to mean something, which means it has to be absent.
        let mut app = App::new(24);
        app.update(Msg::Game(Box::new(ranked_entry(Ok(analyse(
            &Evidence::default(),
        ))))));

        let screen = draw(&app, 160, 40);
        assert!(
            !screen.contains("· tied"),
            "got:
{screen}"
        );
        assert!(
            !screen.contains("not searched in full"),
            "got:
{screen}"
        );
        assert!(
            !screen.contains("no static game evidence"),
            "got:
{screen}"
        );
    }

    #[test]
    fn a_missing_install_keeps_its_diagnosis_without_a_partial_scan_marker() {
        let mut app = App::new(24);
        app.update(Msg::Game(Box::new(missing_install_entry(
            io::ErrorKind::NotFound,
            "directory is absent",
        ))));

        let screen = draw(&app, 160, 40);
        assert!(
            screen.contains("Could not read install: directory is absent"),
            "the diagnosis must remain visible, got:\n{screen}"
        );
        assert!(
            !screen.contains("not searched in full"),
            "an absent install must not be presented as a partial scan, got:\n{screen}"
        );
    }

    #[test]
    fn a_failed_existing_install_shows_its_diagnosis_and_partial_scan_marker() {
        let mut app = App::new(24);
        app.update(Msg::Game(Box::new(missing_install_entry(
            io::ErrorKind::PermissionDenied,
            "permission denied",
        ))));

        let screen = draw(&app, 160, 40);
        assert!(
            screen.contains("Could not read install: permission denied"),
            "the diagnosis must remain visible, got:\n{screen}"
        );
        assert!(
            screen.contains("not searched in full"),
            "a failed existing install must carry the caveat, got:\n{screen}"
        );
    }

    #[test]
    fn a_best_executable_with_nothing_arguing_for_it_says_so_in_the_cli_s_words() {
        // The shape of a directory of leftover installers: something ranked
        // first because something had to. A score with no reasons under it
        // reads as truncated output instead of as the finding it is.
        let mut app = App::new(24);
        let mut entry = ranked_entry(Ok(analyse(&Evidence::default())));
        if let Best::Ranked(ranked) = &mut entry.best {
            ranked.reasons.clear();
        }
        app.update(Msg::Game(Box::new(entry)));

        let screen = draw(&app, 160, 40);
        assert!(
            screen.contains("nothing observed argues that this is the game"),
            "got:
{screen}"
        );
    }
}
