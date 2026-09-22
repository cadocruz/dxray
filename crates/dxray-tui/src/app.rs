//! The TUI's state and Bubble Tea-style message reducer.

use ratatui_bubbletea_components::SpinnerState;
use ratatui_tea::{Cmd, Model};

use crate::entry::Entry;
use crate::key::Key;
use crate::layout;
use crate::msg::{Msg, ScanDiagnostic};

/// The pane whose navigation keys currently have ownership.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    List,
    Detail,
}

/// State that is safe to mutate on the terminal thread.
pub struct App {
    pub(crate) entries: Vec<Entry>,
    /// Text entered by the user to narrow the library by game name or `AppID`.
    pub(crate) filter: String,
    /// An index into `entries`, never into the transient filtered list.
    pub(crate) selected: Option<usize>,
    pub(crate) offset: usize,
    /// First logical line shown in the detail panel.
    pub(crate) detail_offset: usize,
    pub(crate) evidence_expanded: bool,
    pub(crate) focus: Focus,
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) roots: usize,
    pub(crate) libraries: usize,
    pub(crate) problems: Vec<ScanMessage>,
    pub(crate) finished: bool,
    pub(crate) quitting: bool,
    pub(crate) spinner: SpinnerState,
}

/// A discovery diagnostic retaining whether it is a problem or a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ScanMessage {
    Problem(ScanDiagnostic),
    Note(ScanDiagnostic),
}

impl ScanMessage {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::Problem(_) => "Problem",
            Self::Note(_) => "Note",
        }
    }

    pub(crate) fn text(&self) -> String {
        match self {
            Self::Problem(diagnostic) | Self::Note(diagnostic) => {
                if diagnostic.origin.label().is_empty() {
                    diagnostic.message.clone()
                } else {
                    format!("{}: {}", diagnostic.origin, diagnostic.message)
                }
            }
        }
    }
}

impl App {
    #[must_use]
    pub const fn new(height: u16) -> Self {
        Self {
            entries: Vec::new(),
            filter: String::new(),
            selected: None,
            offset: 0,
            detail_offset: 0,
            evidence_expanded: false,
            focus: Focus::List,
            width: 80,
            height,
            roots: 0,
            libraries: 0,
            problems: Vec::new(),
            finished: false,
            quitting: false,
            spinner: SpinnerState::new(),
        }
    }

    #[must_use]
    pub const fn should_quit(&self) -> bool {
        self.quitting
    }

    #[must_use]
    pub(crate) fn selected_entry(&self) -> Option<&Entry> {
        self.selected.and_then(|index| self.entries.get(index))
    }

    #[must_use]
    pub(crate) fn filtered_len(&self) -> usize {
        self.filtered_indices().count()
    }

    #[must_use]
    pub(crate) fn visible_entries(&self) -> Vec<&Entry> {
        self.filtered_indices()
            .skip(self.offset)
            .take(self.visible_entry_count_from(self.offset))
            .map(|index| &self.entries[index])
            .collect()
    }

    pub(crate) fn evidence_group_len(&self) -> usize {
        self.filtered_indices()
            .take_while(|&index| !self.entries[index].lacks_evidence())
            .count()
    }

    pub(crate) fn no_evidence_len(&self) -> usize {
        self.filtered_len()
            .saturating_sub(self.evidence_group_len())
    }

    fn visible_entry_count_from(&self, offset: usize) -> usize {
        let length = self.filtered_len();
        if offset >= length {
            return 0;
        }
        let mut rows = layout::list_rows(self.width, self.height);
        if self.width >= layout::SPLIT_WIDTH {
            rows = rows.saturating_sub(1);
        }
        let boundary = self.evidence_group_len();
        let has_section = boundary < length;
        let mut used = usize::from(has_section && offset >= boundary);
        let mut count = 0;
        for position in offset..length {
            if has_section && position == boundary && position != offset {
                used = used.saturating_add(1);
            }
            if used >= rows {
                break;
            }
            used = used.saturating_add(1);
            count += 1;
        }
        count
    }

    fn maximum_list_offset(&self) -> usize {
        let length = self.filtered_len();
        if length == 0 {
            return 0;
        }
        let mut offset = length - 1;
        while offset > 0 && self.visible_entry_count_from(offset - 1) > length - offset {
            offset -= 1;
        }
        offset
    }

    /// The entries the list shows, in the order it shows them.
    ///
    /// Two groups, never fewer rows: everything the scan found is here, and the
    /// installs that were read and carry no evidence of being a game are moved
    /// below the rest rather than hidden. On a real library that is roughly
    /// four rows in nine — Proton builds and Steam runtimes — and the answer
    /// that they argue nothing was already computed and thrown away. Erring by
    /// showing something extra beats erring by hiding a game, so this demotes
    /// and marks; it never filters.
    ///
    /// An install that could not be read stays with the games. It was not
    /// asked, and demoting it would put an unanswered question where an answer
    /// of "nothing" belongs.
    ///
    /// Within each group the arrival order is kept. There is no second sort
    /// key: reordering games against each other would be this browser claiming
    /// a ranking between games that no evidence supports.
    ///
    /// `entries` itself stays in arrival order, so [`App::selected`] — an index
    /// into it — keeps pointing at the same game when an arrival changes the
    /// order on screen.
    fn filtered_indices(&self) -> impl Iterator<Item = usize> + '_ {
        let needle = self.filter.to_lowercase();
        let second = needle.clone();
        let with_evidence = self
            .entries
            .iter()
            .enumerate()
            .filter(move |(_, entry)| entry.matches(&needle) && !entry.lacks_evidence())
            .map(|(index, _)| index);
        let without_evidence = self
            .entries
            .iter()
            .enumerate()
            .filter(move |(_, entry)| entry.matches(&second) && entry.lacks_evidence())
            .map(|(index, _)| index);
        with_evidence.chain(without_evidence)
    }

    pub(crate) fn selected_position(&self) -> Option<usize> {
        self.selected
            .and_then(|selected| self.filtered_indices().position(|index| index == selected))
    }

    fn select(&mut self, position: usize) {
        let Some(last) = self.filtered_len().checked_sub(1) else {
            self.selected = None;
            self.offset = 0;
            self.detail_offset = 0;
            return;
        };
        let index = self
            .filtered_indices()
            .nth(position.min(last))
            .expect("a filtered position below its measured length must exist");
        if self.selected != Some(index) {
            self.evidence_expanded = false;
        }
        self.selected = Some(index);
        self.keep_selection_visible();
        self.detail_offset = 0;
    }

    fn keep_selection_visible(&mut self) {
        let Some(selected) = self.selected_position() else {
            self.selected = None;
            self.offset = 0;
            return;
        };
        if selected < self.offset {
            self.offset = selected;
        } else {
            while selected
                >= self
                    .offset
                    .saturating_add(self.visible_entry_count_from(self.offset))
                && self.offset < selected
            {
                self.offset += 1;
            }
        }
        self.offset = self.offset.min(self.maximum_list_offset());
    }

    fn filter_changed(&mut self) {
        self.evidence_expanded = false;
        self.detail_offset = 0;
        if self.selected_position().is_none() {
            self.select(0);
        } else {
            self.keep_selection_visible();
        }
        self.clamp_detail_offset();
    }

    fn detail_page(&mut self, delta: isize) {
        let page = layout::detail_rows(self.width, self.height);
        self.detail_offset = if delta.is_negative() {
            self.detail_offset.saturating_sub(page)
        } else {
            self.detail_offset.saturating_add(page)
        };
        self.clamp_detail_offset();
    }

    fn detail_line_count(&self) -> usize {
        crate::view::detail_line_count(self)
    }

    fn clamp_detail_offset(&mut self) {
        let max = self
            .detail_line_count()
            .saturating_sub(layout::detail_rows(self.width, self.height));
        self.detail_offset = self.detail_offset.min(max);
    }

    fn key(&mut self, key: Key) {
        match key {
            Key::Interrupt => self.quitting = true,
            Key::Tab | Key::BackTab => {
                self.focus = match self.focus {
                    Focus::List => Focus::Detail,
                    Focus::Detail => Focus::List,
                }
            }
            // Every printable character, including `q`, is searchable text.
            // Esc has the conventional two-stage meaning: clear an active
            // filter, then leave the browser once there is nothing to clear.
            Key::Escape if self.filter.is_empty() => self.quitting = true,
            Key::Escape => {
                self.filter.clear();
                self.filter_changed();
            }
            Key::Char(character) => {
                self.filter.push(character);
                self.filter_changed();
            }
            Key::Backspace => {
                self.filter.pop();
                self.filter_changed();
            }
            Key::Up if self.focus == Focus::Detail => {
                self.detail_offset = self.detail_offset.saturating_sub(1);
            }
            Key::Down if self.focus == Focus::Detail => {
                self.detail_offset = self.detail_offset.saturating_add(1);
                self.clamp_detail_offset();
            }
            Key::PageUp if self.focus == Focus::Detail => self.detail_page(-1),
            Key::PageDown if self.focus == Focus::Detail => self.detail_page(1),
            Key::Home if self.focus == Focus::Detail => self.detail_offset = 0,
            Key::End if self.focus == Focus::Detail => {
                self.detail_offset = usize::MAX;
                self.clamp_detail_offset();
            }
            Key::Up => self.select(self.selected_position().unwrap_or(0).saturating_sub(1)),
            Key::Down => self.select(self.selected_position().map_or(0, |index| index + 1)),
            Key::PageUp => self.select(
                self.selected_position()
                    .unwrap_or(0)
                    .saturating_sub(self.visible_entry_count_from(self.offset).max(1)),
            ),
            Key::PageDown => self.select(self.selected_position().map_or(0, |index| {
                index.saturating_add(self.visible_entry_count_from(self.offset).max(1))
            })),
            Key::Home => self.select(0),
            Key::End => self.select(usize::MAX),
            Key::Enter if self.focus == Focus::Detail && self.selected_entry().is_some() => {
                self.evidence_expanded = !self.evidence_expanded;
                self.clamp_detail_offset();
            }
            Key::Enter => {}
        }
    }
}

impl Model for App {
    type Msg = Msg;

    fn update(&mut self, msg: Msg) -> Cmd<Msg> {
        match msg {
            Msg::Key(key) => self.key(key),
            Msg::Resize(width, height) => {
                self.width = width;
                self.height = height;
                self.keep_selection_visible();
                self.clamp_detail_offset();
            }
            Msg::Root(crate::msg::ScanLocation { .. }) => {
                self.roots = self.roots.saturating_add(1);
            }
            Msg::Library(crate::msg::ScanLocation { .. }) => {
                self.libraries = self.libraries.saturating_add(1);
            }
            Msg::Game(entry) => {
                self.entries.push(*entry);
                if self.selected.is_none() && self.selected_position().is_none() {
                    self.select(0);
                } else {
                    // Entries stream in while the scan runs, and one that
                    // carries evidence lands above every install that carries
                    // none. That moves the selected game's row without moving
                    // the selection, so the viewport is re-aimed at it here
                    // rather than at whatever slid into its old position.
                    self.keep_selection_visible();
                }
            }
            Msg::Problem(problem) => self.problems.push(ScanMessage::Problem(problem)),
            Msg::Note(note) => self.problems.push(ScanMessage::Note(note)),
            Msg::Finished => self.finished = true,
            // Ticks can already be queued when the scan completes. Ignoring
            // them makes the completed header stable even before the ticker
            // worker observes the completion signal and exits.
            Msg::Tick if !self.finished => self.spinner.tick(),
            Msg::Tick => {}
        }
        Cmd::none()
    }

    fn view(&self, frame: &mut ratatui::Frame<'_>) {
        crate::view::render(self, frame);
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ratatui_tea::Model;

    use super::{App, Focus};
    use crate::{
        Key, Msg,
        entry::{Best, Entry},
        msg::ScanLocation,
    };
    use dxray_core::game::{Candidate, Reason, Survey};
    use dxray_core::proton::Answer;

    fn game(appid: u32, name: &str) -> Entry {
        Entry {
            identity: dxray_core::Identity::SteamApp(appid),
            origin: dxray_core::steam::ORIGIN,
            name: name.to_owned(),
            install_dir: PathBuf::new(),
            library: PathBuf::new(),
            best: Best::NoExecutable,
            notes: Vec::new(),
            tie: None,
            // Hand-built rows say nothing about evidence; the tests that care
            // go through `Entry::build` with a real survey.
            carries_evidence: None,
            incomplete: false,
            nvapi: Answer {
                script: None,
                verdict: String::new(),
                available: None,
                condition: Vec::new(),
            },
        }
    }

    fn add(app: &mut App, appid: u32, name: &str) {
        app.update(Msg::Game(Box::new(game(appid, name))));
    }

    #[test]
    fn expansion_belongs_to_current_selection_and_filter() {
        let mut app = App::new(30);
        app.update(Msg::Key(Key::Tab));
        app.update(Msg::Key(Key::Enter));
        assert!(!app.evidence_expanded);
        add(&mut app, 1, "Alpha");
        add(&mut app, 2, "Beta");
        app.update(Msg::Key(Key::Enter));
        assert!(app.evidence_expanded);
        app.update(Msg::Resize(60, 12));
        add(&mut app, 3, "Gamma");
        assert!(app.evidence_expanded);
        app.update(Msg::Key(Key::Tab));
        app.update(Msg::Key(Key::Enter));
        assert!(app.evidence_expanded);
        app.update(Msg::Key(Key::Down));
        assert!(!app.evidence_expanded);
        app.update(Msg::Key(Key::Tab));
        app.update(Msg::Key(Key::Enter));
        app.update(Msg::Key(Key::Char('B')));
        assert!(!app.evidence_expanded);
        assert_eq!(app.detail_offset, 0);
        assert_eq!(app.selected, Some(1));
    }

    /// An entry built the way the scanning thread builds it, from a survey, so
    /// that the evidence question is answered by `dxray-core` and not by the
    /// test.
    fn surveyed(appid: u32, name: &str, reasons: Vec<Reason>) -> Entry {
        let survey = Survey::ranked(
            vec![Candidate {
                path: PathBuf::from(format!("/games/{appid}/game.exe")),
                reasons,
            }],
            Vec::new(),
        );
        Entry::build(
            dxray_core::launcher::Game {
                identity: dxray_core::Identity::SteamApp(appid),
                name: name.to_owned(),
                install_dir: PathBuf::from(format!("/games/{appid}")),
                origin: dxray_core::steam::ORIGIN,
            },
            PathBuf::from("/steam"),
            Ok(survey),
            Answer {
                script: None,
                verdict: String::new(),
                available: None,
                condition: Vec::new(),
            },
        )
    }

    fn add_surveyed(app: &mut App, appid: u32, name: &str, reasons: Vec<Reason>) {
        app.update(Msg::Game(Box::new(surveyed(appid, name, reasons))));
    }

    fn shown(app: &App) -> Vec<String> {
        app.visible_entries()
            .iter()
            .map(|entry| entry.name.clone())
            .collect()
    }

    #[test]
    fn installs_that_argue_nothing_sort_last_and_none_of_them_is_dropped() {
        // A Proton build and a Steam runtime are ranked exactly like this: an
        // executable is found, and nothing about it argues it is a game. They
        // are moved down, never out — hiding one would hide a game the day the
        // ranking is wrong about it.
        let mut app = App::new(24);
        add_surveyed(&mut app, 10, "Alpha", vec![Reason::ShippingSuffix]);
        add_surveyed(&mut app, 20, "Runtime", Vec::new());
        add_surveyed(&mut app, 30, "Charlie", vec![Reason::ShippingSuffix]);
        add_surveyed(&mut app, 40, "Proton", Vec::new());

        assert_eq!(app.filtered_len(), 4, "nothing is filtered out");
        assert_eq!(shown(&app), ["Alpha", "Charlie", "Runtime", "Proton"]);
    }

    #[test]
    fn an_install_that_could_not_be_read_keeps_its_place_among_the_games() {
        // It was never asked the question. Sorting it with the directories
        // that were read and found empty would make an unanswered question
        // look like an answer of "nothing".
        let mut app = App::new(24);
        add_surveyed(&mut app, 10, "Runtime", Vec::new());
        app.update(Msg::Game(Box::new(Entry::build(
            dxray_core::launcher::Game {
                identity: dxray_core::Identity::SteamApp(20),
                name: "Unreadable".to_owned(),
                install_dir: PathBuf::from("/games/20"),
                origin: dxray_core::steam::ORIGIN,
            },
            PathBuf::from("/steam"),
            Err(std::io::Error::other("stale mount")),
            Answer {
                script: None,
                verdict: String::new(),
                available: None,
                condition: Vec::new(),
            },
        ))));

        assert_eq!(shown(&app), ["Unreadable", "Runtime"]);
    }

    #[test]
    fn the_selection_follows_the_game_when_a_later_arrival_reorders_the_list() {
        // `selected` is an index into `entries`, which stays in arrival order
        // precisely so this holds. Entries stream in while the scan runs, and
        // one that carries evidence lands above every one that carries none.
        let mut app = App::new(24);
        add_surveyed(&mut app, 10, "Runtime", Vec::new());
        add_surveyed(&mut app, 20, "Redistributable", Vec::new());
        app.update(Msg::Key(Key::Down));
        assert_eq!(
            app.selected_entry().map(|entry| entry.name.as_str()),
            Some("Redistributable")
        );
        assert_eq!(app.selected_position(), Some(1));

        add_surveyed(&mut app, 30, "Alpha", vec![Reason::ShippingSuffix]);
        assert_eq!(
            app.selected_entry().map(|entry| entry.name.as_str()),
            Some("Redistributable"),
            "the selected game is the same game, not the same row"
        );
        assert_eq!(
            app.selected_position(),
            Some(2),
            "and it moved down by exactly the one entry that overtook it"
        );
        assert_eq!(shown(&app), ["Alpha", "Runtime", "Redistributable"]);
    }

    #[test]
    fn a_reordering_arrival_re_aims_the_viewport_at_the_selected_game() {
        let mut app = App::new(12);
        app.update(Msg::Resize(120, 12));
        for id in 10..16 {
            add_surveyed(&mut app, id, &format!("Runtime {id}"), Vec::new());
        }
        app.update(Msg::Key(Key::End));
        assert_eq!(app.selected_position(), Some(5));
        assert!(app.offset > 0, "the viewport is scrolled");

        // Lands above the runtimes, moving the selected entry down one row.
        add_surveyed(&mut app, 40, "Alpha", vec![Reason::ShippingSuffix]);

        let position = app
            .selected_position()
            .expect("the selected game is still in the list");
        assert_eq!(position, 6, "the selected game was pushed down by one");
        assert!(
            app.visible_entries()
                .iter()
                .any(|entry| entry.name == "Runtime 15"),
            "the selected entry remains in the viewport at {position} with offset {}",
            app.offset,
        );
    }

    #[test]
    fn a_filter_narrows_within_the_evidence_order_rather_than_undoing_it() {
        let mut app = App::new(24);
        add_surveyed(&mut app, 10, "Alpha runtime", Vec::new());
        add_surveyed(&mut app, 20, "Alpha game", vec![Reason::ShippingSuffix]);
        add_surveyed(&mut app, 30, "Beta game", vec![Reason::ShippingSuffix]);

        for character in "alpha".chars() {
            app.update(Msg::Key(Key::Char(character)));
        }
        assert_eq!(shown(&app), ["Alpha game", "Alpha runtime"]);
        assert_eq!(
            app.selected_entry().map(|entry| entry.name.as_str()),
            Some("Alpha runtime"),
            "the demoted row is still selectable, and a filter that keeps it              does not move the selection off it"
        );
    }

    #[test]
    fn escape_and_interrupt_quit_when_no_filter_is_active() {
        let mut app = App::new(24);
        app.update(Msg::Key(Key::Escape));
        assert!(app.should_quit());

        let mut app = App::new(24);
        app.update(Msg::Key(Key::Interrupt));
        assert!(app.should_quit());
    }

    #[test]
    fn q_is_searchable_text_and_escape_clears_it_before_quitting() {
        let mut app = App::new(24);
        add(&mut app, 440, "Quake");
        app.update(Msg::Key(Key::Char('q')));
        assert!(!app.should_quit());
        assert_eq!(app.filter, "q");
        assert_eq!(
            app.selected_entry().map(|entry| entry.name.as_str()),
            Some("Quake")
        );

        app.update(Msg::Key(Key::Escape));
        assert_eq!(app.filter, "");
        assert!(!app.should_quit());
        app.update(Msg::Key(Key::Escape));
        assert!(app.should_quit());
    }

    #[test]
    fn empty_navigation_does_not_underflow() {
        let mut app = App::new(1);
        app.update(Msg::Key(Key::End));
        app.update(Msg::Key(Key::PageDown));
        assert_eq!(app.selected, None);
        assert_eq!(app.offset, 0);
    }

    #[test]
    fn a_heroic_configuration_counts_as_a_discovered_library() {
        let mut app = App::new(24);
        let root = PathBuf::from("/config/heroic");
        app.update(Msg::Root(ScanLocation {
            origin: dxray_core::heroic::ORIGIN,
            path: root.clone(),
        }));
        app.update(Msg::Library(ScanLocation {
            origin: dxray_core::heroic::ORIGIN,
            path: root,
        }));
        assert_eq!(app.roots, 1);
        assert_eq!(app.libraries, 1);
    }

    #[test]
    fn typed_text_filters_by_name_and_appid_without_losing_the_selected_game() {
        let mut app = App::new(8);
        add(&mut app, 440, "Team Fortress 2");
        add(&mut app, 730, "Counter-Strike 2");
        add(&mut app, 570, "Dota 2");

        app.update(Msg::Key(Key::Down));
        app.update(Msg::Key(Key::Char('c')));
        app.update(Msg::Key(Key::Char('O')));
        assert_eq!(app.filter, "cO");
        assert_eq!(app.filtered_len(), 1);
        assert_eq!(
            app.selected_entry()
                .map(|entry| entry.identity.steam_appid().unwrap()),
            Some(730)
        );

        app.update(Msg::Key(Key::Backspace));
        app.update(Msg::Key(Key::Backspace));
        app.update(Msg::Key(Key::Char('5')));
        app.update(Msg::Key(Key::Char('7')));
        app.update(Msg::Key(Key::Char('0')));
        assert_eq!(
            app.selected_entry().map(|entry| entry.name.as_str()),
            Some("Dota 2")
        );
    }

    #[test]
    fn changing_filter_reselects_a_visible_entry_and_keeps_its_scroll_position_valid() {
        let mut app = App::new(7);
        for (appid, name) in [(10, "Alpha"), (20, "Bravo"), (30, "Charlie"), (40, "Delta")] {
            add(&mut app, appid, name);
        }
        app.update(Msg::Key(Key::End));
        assert_eq!(
            app.selected_entry().map(|entry| entry.name.as_str()),
            Some("Delta")
        );

        app.update(Msg::Key(Key::Char('a')));
        app.update(Msg::Key(Key::Char('l')));
        app.update(Msg::Key(Key::Char('p')));
        assert_eq!(
            app.selected_entry().map(|entry| entry.name.as_str()),
            Some("Alpha")
        );
        assert_eq!(app.offset, 0);

        app.update(Msg::Key(Key::Escape));
        assert_eq!(app.filter, "");
        assert_eq!(
            app.selected_entry().map(|entry| entry.name.as_str()),
            Some("Alpha")
        );
    }

    #[test]
    fn search_survives_arrivals_resize_and_scroll_without_hiding_empty_entries() {
        let mut app = App::new(12);
        app.update(Msg::Resize(32, 12));
        for character in "MiXeD/".chars() {
            app.update(Msg::Key(Key::Char(character)));
        }
        add(&mut app, 1, "unrelated");
        assert_eq!(app.selected, None);
        let mut empty = game(42, "mixed/empty");
        empty.carries_evidence = Some(false);
        app.update(Msg::Game(Box::new(empty)));
        let selected = app.selected;
        for id in 100..110 {
            let mut entry = game(id, "MIXED/evidence");
            entry.carries_evidence = Some(true);
            app.update(Msg::Game(Box::new(entry)));
        }
        assert_eq!(app.filtered_len(), 11);
        assert_eq!(app.selected, selected);
        assert!(
            app.visible_entries()
                .iter()
                .any(|entry| entry.name == "mixed/empty")
        );
        app.update(Msg::Resize(100, 24));
        assert_eq!(app.selected, selected);
        app.update(Msg::Key(Key::Home));
        assert_eq!(app.selected_entry().unwrap().name, "MIXED/evidence");
        app.update(Msg::Key(Key::Escape));
        assert!(!app.quitting);
        for character in "42".chars() {
            app.update(Msg::Key(Key::Char(character)));
        }
        assert_eq!(app.filtered_len(), 1);
        assert_eq!(app.selected, selected);
        app.update(Msg::Key(Key::Backspace));
        assert_eq!(app.filter, "4");
        assert_eq!(app.selected, selected);
    }

    #[test]
    fn a_filter_with_no_results_has_no_selection_or_visible_rows() {
        let mut app = App::new(24);
        add(&mut app, 440, "Team Fortress 2");
        for character in "missing".chars() {
            app.update(Msg::Key(Key::Char(character)));
        }

        assert_eq!(app.filtered_len(), 0);
        assert_eq!(app.selected, None);
        assert_eq!(app.offset, 0);
        assert!(app.visible_entries().is_empty());
    }

    #[test]
    fn tab_routes_navigation_to_details_and_keeps_list_selection_stable() {
        let mut app = App::new(10);
        let mut first = game(10, "First");
        first.notes = (0..30).map(|index| format!("detail {index}")).collect();
        app.update(Msg::Game(Box::new(first)));
        add(&mut app, 20, "Second");

        app.update(Msg::Key(Key::Tab));
        assert_eq!(app.focus, Focus::Detail);
        app.update(Msg::Key(Key::Down));
        assert_eq!(
            app.selected_entry()
                .map(|entry| entry.identity.steam_appid().unwrap()),
            Some(10)
        );
        assert_eq!(app.detail_offset, 1);

        app.update(Msg::Key(Key::PageDown));
        assert_eq!(
            app.detail_offset,
            1 + crate::layout::detail_rows(app.width, app.height)
        );
        app.update(Msg::Key(Key::Tab));
        assert_eq!(app.focus, Focus::List);
        app.update(Msg::Key(Key::Down));
        assert_eq!(
            app.selected_entry()
                .map(|entry| entry.identity.steam_appid().unwrap()),
            Some(20)
        );
        assert_eq!(app.detail_offset, 0, "selection resets detail scroll");
    }

    #[test]
    fn detail_scroll_is_bounded_and_reclamped_after_resize() {
        let mut app = App::new(10);
        let mut entry = game(10, "Long");
        entry.notes = (0..30)
            .map(|index| format!("abcdef abcdef abcdef detail {index}"))
            .collect();
        app.update(Msg::Game(Box::new(entry)));
        app.update(Msg::Resize(32, 10));
        app.update(Msg::Key(Key::Tab));
        app.update(Msg::Key(Key::End));
        let narrow_offset = app.detail_offset;
        assert_eq!(
            narrow_offset,
            app.detail_line_count()
                .saturating_sub(crate::layout::detail_rows(app.width, app.height))
        );
        app.update(Msg::Resize(120, 30));
        assert!(app.detail_offset < narrow_offset);
        assert_eq!(
            app.detail_offset,
            app.detail_line_count()
                .saturating_sub(crate::layout::detail_rows(app.width, app.height))
        );
        app.update(Msg::Key(Key::PageDown));
        assert_eq!(
            app.detail_offset,
            app.detail_line_count()
                .saturating_sub(crate::layout::detail_rows(app.width, app.height))
        );
        app.update(Msg::Key(Key::Home));
        assert_eq!(app.detail_offset, 0);
    }

    #[test]
    fn backtab_reverses_focus_without_changing_selection() {
        let mut app = App::new(24);
        add(&mut app, 10, "First");
        add(&mut app, 20, "Second");
        app.update(Msg::Key(Key::Tab));
        assert_eq!(app.focus, Focus::Detail);

        app.update(Msg::Key(Key::BackTab));
        assert_eq!(app.focus, Focus::List);
        assert_eq!(
            app.selected_entry()
                .map(|entry| entry.identity.steam_appid().unwrap()),
            Some(10)
        );
    }

    #[test]
    fn scan_completion_freezes_the_spinner_state() {
        let mut app = App::new(24);
        app.update(Msg::Tick);
        let before_completion = app.spinner.frame_index();
        app.update(Msg::Finished);
        assert!(app.finished);
        // A tick can already be queued by the ticker when the scanner sends
        // Finished. Every subsequent tick must be inert, not just the first.
        app.update(Msg::Tick);
        app.update(Msg::Tick);
        assert_eq!(app.spinner.frame_index(), before_completion);
    }
}
