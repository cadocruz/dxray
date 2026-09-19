//! Rendering for the library browser.

use std::fmt::Write as _;

use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui_bubbletea_components::{ListItem, SelectList, Spinner, SpinnerFrames};
use ratatui_bubbletea_theme::BubbleTheme;

use crate::app::{App, Focus};
use crate::entry::{Best, Entry, Ranked};
use dxray_core::analysis::Finding;

pub(crate) fn render(app: &App, frame: &mut ratatui::Frame<'_>) {
    if frame.area().width == 0 || frame.area().height == 0 {
        return;
    }
    let theme = BubbleTheme::default();
    let areas = crate::layout::areas(frame.area());

    render_header(app, &theme, frame, areas.header);
    render_list(app, &theme, frame, areas.list);
    render_detail(app, &theme, frame, areas.detail);
    render_if_visible(
        frame,
        Paragraph::new(
            "type to filter name/ID  Tab/Shift-Tab switch pane  Backspace edit  Esc clear/quit  Ctrl-C quit  ↑↓ move  PgUp/PgDn page",
        )
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
    let status = if app.finished {
        format!(
            "{} games from {} libraries",
            app.entries.len(),
            app.libraries
        )
    } else {
        format!("scanning {} roots / {} libraries", app.roots, app.libraries)
    };
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
                if app.filter.is_empty() {
                    "filter: all games".to_owned()
                } else {
                    format!("filter: {}", app.filter)
                },
                theme.muted,
            ),
        ])),
        area,
    );
    if !app.finished && area.width > 0 && area.height >= 2 {
        let mut spinner = Spinner::new()
            .frames(SpinnerFrames::DOTS)
            .label(status)
            .theme(*theme);
        for _ in 0..app.spinner.frame_index() {
            spinner.tick();
        }
        render_if_visible(
            frame,
            &spinner,
            ratatui::layout::Rect::new(area.x, area.y.saturating_add(1), area.width, 1),
        );
    }
}

fn render_list(
    app: &App,
    theme: &BubbleTheme,
    frame: &mut ratatui::Frame<'_>,
    area: ratatui::layout::Rect,
) {
    let mut title = if app.filter.is_empty() {
        format!("Games ({})", app.entries.len())
    } else {
        format!("Games ({} of {})", app.filtered_len(), app.entries.len())
    };
    if app.focus == Focus::List {
        title.push_str(" · active");
    }
    let block = theme.titled_block(title);
    let inner = block.inner(area);
    render_if_visible(frame, block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let entries = app.visible_entries();
    if entries.is_empty() && !app.filter.is_empty() {
        render_if_visible(
            frame,
            Paragraph::new(format!("No games match \"{}\".", app.filter)).style(theme.muted),
            inner,
        );
        return;
    }
    let items = entries.iter().map(|entry| {
        ListItem::new(entry.name.clone()).description(format!(
            "{} · {}{} · {}",
            entry.origin.label(),
            entry.id_label(),
            caveats(entry),
            entry.headline()
        ))
    });
    let mut list = SelectList::new(items).theme(*theme);
    list.select(
        app.selected_position()
            .and_then(|selected| selected.checked_sub(app.offset)),
    );
    render_if_visible(frame, &list, inner);
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
    let mut out = String::new();
    // First, because it qualifies the headline harder than the others do: the
    // renderer beside it was read off the best of several executables that
    // between them argue nothing about being a game.
    //
    // `no evidence` and not `tool`. This says what was observed — nothing —
    // and stops there. Naming the category is the claim that `DownloadType`
    // made and that marked shipped games as tooling; the sentence under it, in
    // the detail pane, is the same one `dxray --game` prints.
    if entry.lacks_evidence() {
        out.push_str(" · no evidence");
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
    let mut lines = app.selected_entry().map_or_else(
        || {
            if app.filter.is_empty() {
                vec![Line::from("Waiting for a game to be discovered.")]
            } else {
                vec![Line::from("No selected game matches the current filter.")]
            }
        },
        detail_lines,
    );
    if !app.problems.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from("Scan messages:"));
        lines.extend(
            app.problems
                .iter()
                .map(|message| Line::from(format!("{}: {}", message.label(), message.text()))),
        );
    }
    let mut title = if app.problems.is_empty() {
        "Details".to_owned()
    } else {
        format!("Details · {} scan notes", app.problems.len())
    };
    if app.focus == Focus::Detail {
        title.push_str(" · active");
    }
    let content_lines = detail_line_count(app);
    if content_lines > crate::layout::detail_rows(app.height) {
        write!(
            title,
            " · {}/{}",
            app.detail_offset.saturating_add(1),
            content_lines
        )
        .expect("writing to a String cannot fail");
    }
    render_if_visible(
        frame,
        Paragraph::new(lines)
            .style(theme.text)
            .block(theme.titled_block(title))
            .scroll((u16::try_from(app.detail_offset).unwrap_or(u16::MAX), 0))
            .wrap(Wrap { trim: false }),
        area,
    );
}

/// Visual rows in the current detail pane. The reducer uses this to keep a
/// scroll offset valid after selection, scan messages, and terminal resizes.
pub(crate) fn detail_line_count(app: &App) -> usize {
    let mut lines = app.selected_entry().map_or_else(
        || vec![Line::from("Waiting for a game to be discovered.")],
        detail_lines,
    );
    if !app.problems.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from("Scan messages:"));
        lines.extend(
            app.problems
                .iter()
                .map(|message| Line::from(format!("{}: {}", message.label(), message.text()))),
        );
    }
    let width = usize::from(crate::layout::detail_width(app.width));
    lines
        .iter()
        .map(|line| wrapped_line_count(line, width))
        .sum()
}

/// Count the rows produced by Ratatui's word wrapping for the plain lines we
/// render here. A width-only division undercounts strings made of several
/// words that cannot share a row (for example, three six-character paths in a
/// ten-cell panel), which made `End` stop before the actual last visual row.
fn wrapped_line_count(line: &Line<'_>, width: usize) -> usize {
    let width = width.max(1);
    let text = line.to_string();
    if text.is_empty() {
        return 1;
    }

    let mut rows = 1_usize;
    let mut row_width = 0_usize;
    for token in text.split_inclusive(char::is_whitespace) {
        let token_width = Line::from(token).width();
        if token_width == 0 {
            continue;
        }
        if !token.chars().next().is_some_and(char::is_whitespace)
            && row_width > 0
            && row_width.saturating_add(token_width) > width
        {
            rows += 1;
            row_width = 0;
        }
        for character in token.chars() {
            let character_width = Line::from(character.to_string()).width();
            if row_width > 0 && row_width.saturating_add(character_width) > width {
                rows += 1;
                row_width = 0;
            }
            row_width = row_width.saturating_add(character_width);
        }
    }
    rows
}

fn detail_lines(entry: &Entry) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(format!("{} ({})", entry.name, entry.id_label())),
        Line::from(format!("Source: {}", entry.origin.label())),
        Line::from(format!("Install: {}", entry.install_dir.display())),
        Line::from(format!("Library: {}", entry.library.display())),
        Line::from(""),
        Line::from(format!("Renderer: {}", entry.headline())),
        Line::from(format!("NVAPI: {}", entry.nvapi.verdict)),
    ];
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
    match &entry.best {
        Best::Ranked(ranked) => {
            lines.push(Line::from(format!(
                "Executable: {} (score {}, {} candidates)",
                ranked.path.display(),
                ranked.score,
                ranked.of
            )));
            if ranked.has_evidence() {
                lines.extend(
                    ranked
                        .reasons
                        .iter()
                        .map(|reason| Line::from(format!("• {reason}"))),
                );
            } else {
                // `dxray --game`'s own sentence. A score with nothing under it
                // reads as truncated output rather than as the finding it is:
                // this is what a Visual C++ redistributable looks like when it
                // is the best thing in the directory.
                lines.push(Line::from(
                    "• nothing observed argues that this is the game",
                ));
            }
            render_verdict(&mut lines, ranked);
        }
        Best::NoExecutable => lines.push(Line::from("No executable was found in this install.")),
        Best::Unwalkable(error) => {
            lines.push(Line::from(format!("Could not read install: {error}")));
        }
    }
    lines.extend(
        entry
            .notes
            .iter()
            .map(|note| Line::from(format!("Note: {note}"))),
    );
    lines
}

/// Adds the evidence read from the selected executable without interpreting it
/// again. `dxray-core` owns the classification; the TUI only exposes every
/// finding and the observation that supports it.
fn render_verdict(lines: &mut Vec<Line<'static>>, ranked: &Ranked) {
    match &ranked.verdict {
        Ok(verdict) if verdict.is_empty() => {
            lines.push(Line::from(
                "Evidence: executable read; no recognised graphics findings.",
            ));
        }
        Ok(verdict) => {
            lines.push(Line::from("Evidence:"));
            render_findings(lines, "Renderers", &verdict.renderers);
            render_findings(lines, "Infrastructure", &verdict.infrastructure);
            render_findings(lines, "Features", &verdict.features);
            render_findings(lines, "Local overrides", &verdict.local_overrides);
        }
        Err(error) => {
            lines.push(Line::from(format!(
                "Evidence unavailable: could not read executable: {error}"
            )));
        }
    }
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
    use ratatui::{Terminal, backend::TestBackend, text::Line};
    use ratatui_tea::Model;

    use super::{render, wrapped_line_count};
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
        app.update(Msg::Problem(
            "could not read /steam/libraryfolders.vdf".to_owned(),
        ));
        app.update(Msg::Note("library is declared twice".to_owned()));

        let screen = draw(&app, 100, 24);
        assert!(screen.contains("Team Fortress 2"));
        assert!(screen.contains("Problem: could not read /steam/libraryfolders.vdf"));
        assert!(screen.contains("Note: library is declared twice"));
    }

    #[test]
    fn completed_scan_removes_the_spinner_row() {
        let mut app = App::new(24);
        app.update(Msg::Game(Box::new(game())));
        app.update(Msg::Library(PathBuf::from("/steam")));
        assert!(draw(&app, 100, 24).contains("scanning 0 roots / 1 libraries"));

        app.update(Msg::Finished);
        let screen = draw(&app, 100, 24);
        assert!(screen.contains("scan complete"));
        assert!(
            !screen.contains("1 games from 1 libraries"),
            "the spinner and its changing label must disappear once the scan finishes"
        );
    }

    #[test]
    fn a_tiny_viewport_draws_without_panic_or_out_of_bounds_geometry() {
        let mut app = App::new(1);
        app.update(Msg::Game(Box::new(game())));

        let screen = draw(&app, 1, 1);
        assert_eq!(screen.chars().count(), 1);
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

        let screen = draw(&app, 120, 30);
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

        let screen = draw(&app, 160, 50);
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

        let screen = draw(&app, 160, 50);
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
    fn focused_detail_panel_renders_later_long_content_after_scrolling() {
        let mut app = App::new(12);
        let mut entry = game();
        entry.notes = (0..30)
            .map(|index| format!("scroll proof {index}"))
            .collect();
        app.update(Msg::Game(Box::new(entry)));

        let initial = draw(&app, 120, 12);
        assert!(initial.contains("Games (1) · active"));
        assert!(!initial.contains("scroll proof 20"));

        app.update(Msg::Key(crate::Key::Tab));
        app.update(Msg::Key(crate::Key::End));
        let scrolled = draw(&app, 120, 12);
        assert!(scrolled.contains("Details · active"));
        assert!(scrolled.contains("scroll proof 29"));
        assert!(!scrolled.contains("Team Fortress 2 (440)"));
    }

    #[test]
    fn wrapped_line_count_keeps_words_that_cannot_share_a_row_separate() {
        // The text is 20 cells wide, so a simple `width.div_ceil()` would
        // report two rows. A word wrapper needs three 6-cell rows instead.
        assert_eq!(
            wrapped_line_count(&Line::from("abcdef abcdef abcdef"), 10),
            3
        );
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
    fn an_install_that_argues_nothing_is_marked_in_the_list_and_explained_below_it() {
        // The row stays. Only the marker and the order say that nothing in this
        // directory argues it is a game — the word is `no evidence`, because
        // what was observed is nothing, and calling it a tool is the claim this
        // project retired.
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
            screen.contains("· no evidence"),
            "the marker is drawn in the list column, got:
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
            screen.contains(" · no evidence · tied · not searched in full"),
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
            !screen.contains("no evidence"),
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
            !screen.contains("no evidence"),
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
