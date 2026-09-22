//! The event loop and the background scan that fills it, over every launcher.

use std::{
    io,
    ops::ControlFlow,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use ratatui::crossterm::event;
use ratatui_tea::{Program, ProgramHandle, channel};

use crate::app::App;
use crate::entry::Entry;
use crate::key;
use crate::msg::Msg;
use crate::screen;
use dxray_core::proton::Builds;

/// Runs the terminal UI until the user quits or its input channel closes.
///
/// # Errors
///
/// Returns an error when the terminal cannot enter or leave its interactive
/// mode, cannot be queried for its size, or cannot draw a frame.
pub fn run() -> io::Result<()> {
    screen::install_panic_hook();
    let mut terminal = screen::enter()?;
    let (result, workers) = run_terminal(&mut terminal);
    let restored = hand_back(screen::restore, workers);
    result.and(restored)
}

/// Gives the terminal back, and only then waits for the workers.
///
/// Cancellation is cooperative and is polled between filesystem operations, so
/// a scan inside `dxray_core::candidates` on a stale mount can take as long as
/// the mount does. Joining first meant the user pressed Esc and then watched a
/// frozen alternate screen until the walk returned. Restoring first hands the
/// shell back immediately; the join still happens, and no worker draws.
fn hand_back<R>(restore: R, workers: Option<Workers>) -> io::Result<()>
where
    R: FnOnce() -> io::Result<()>,
{
    let restored = restore();
    if let Some(workers) = workers {
        workers.shutdown();
    }
    restored
}

/// The workers come back out so that [`run`] can restore the terminal before
/// joining them. `None` is a failure early enough that none were started.
fn run_terminal(terminal: &mut screen::Tui) -> (io::Result<()>, Option<Workers>) {
    let size = match terminal.size() {
        Ok(size) => size,
        Err(error) => return (Err(error), None),
    };
    let mut program = Program::new(App::new(size.height));
    program.init();
    program.send(Msg::Resize(size.width, size.height));

    // This must happen before the scan thread exists: no directory walk can
    // delay the first frame.
    if let Err(error) = first_frame(&program, terminal) {
        return (Err(error), None);
    }

    let (handle, receiver) = channel();
    let cancellation = Cancellation::default();
    let scanning = Scanning::new();
    let workers = Workers::new(&cancellation, &scanning, handle);

    let result = loop {
        if program.model().should_quit() {
            break Ok(());
        }
        let Ok(message) = receiver.recv() else {
            break Ok(());
        };
        program.send(message);
        if let Err(error) = program.draw(terminal) {
            break Err(error);
        }
    };
    (result, Some(workers))
}

/// Draws without access to background work, enforcing a prompt first frame.
fn first_frame(program: &Program<App>, terminal: &mut screen::Tui) -> io::Result<()> {
    program.draw(terminal).map(|_| ())
}

#[derive(Clone, Default)]
struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
}

struct Workers {
    cancellation: Cancellation,
    handles: Vec<JoinHandle<()>>,
}

impl Workers {
    fn new(cancellation: &Cancellation, scanning: &Scanning, handle: ProgramHandle<Msg>) -> Self {
        Self {
            cancellation: cancellation.clone(),
            handles: vec![
                spawn_input(cancellation.clone(), handle.clone()),
                spawn_ticks(cancellation.clone(), scanning.clone(), handle.clone()),
                spawn_scan(cancellation.clone(), scanning.clone(), handle),
            ],
        }
    }

    fn shutdown(self) {
        self.cancellation.cancel();
        for handle in self.handles {
            let _ = handle.join();
        }
    }
}

fn spawn_input(cancellation: Cancellation, handle: ProgramHandle<Msg>) -> JoinHandle<()> {
    thread::spawn(move || {
        while !cancellation.is_cancelled() {
            // `read` can block forever; bounded polling lets shutdown wake it.
            let Ok(ready) = event::poll(Duration::from_millis(80)) else {
                return;
            };
            if !ready {
                continue;
            }
            let Ok(event) = event::read() else {
                return;
            };
            let message = match &event {
                event::Event::Resize(width, height) => Some(Msg::Resize(*width, *height)),
                _ => key::from_crossterm(&event).map(Msg::Key),
            };
            if message.is_some_and(|message| handle.send(message).is_err()) {
                return;
            }
        }
    })
}

/// A separate completion signal stops the ticker while keeping input alive
/// after a normal scan. It is intentionally distinct from cancellation, which
/// shuts down every worker when the application exits.
#[derive(Clone)]
struct Scanning(Arc<AtomicBool>);

impl Scanning {
    fn new() -> Self {
        Self(Arc::new(AtomicBool::new(true)))
    }

    fn is_active(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    fn finish(&self) {
        self.0.store(false, Ordering::Release);
    }
}

fn spawn_ticks(
    cancellation: Cancellation,
    scanning: Scanning,
    handle: ProgramHandle<Msg>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        while !cancellation.is_cancelled() && scanning.is_active() {
            thread::sleep(Duration::from_millis(80));
            if cancellation.is_cancelled()
                || !scanning.is_active()
                || handle.send(Msg::Tick).is_err()
            {
                return;
            }
        }
    })
}

fn spawn_scan(
    cancellation: Cancellation,
    scanning: Scanning,
    handle: ProgramHandle<Msg>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        if !scan_launchers(dxray_core::launcher::all(), &cancellation, &handle) {
            return;
        }
        if !cancellation.is_cancelled() {
            scanning.finish();
            let _ = handle.send(Msg::Finished);
        }
    })
}

/// Streams every launcher's games down the channel. `false` means the UI has
/// gone away or cancellation was requested, so the worker must stop silently.
///
/// The traversal itself is [`dxray_core::walk`], shared with `dxray --installed`.
/// What is here is this consumer's reaction to it: each thing found goes down
/// the channel as it is found, so the list fills while the scan is still
/// running. The command line's reaction to the same walk is to accumulate rows
/// and print when it returns. Before the walk moved into the core crate these
/// were two loops with a copy each of the library dedup rule, free to drift.
///
/// One loop for every launcher, which it can only be because
/// [`heroic::roots`](dxray_core::heroic::roots) answers "is Heroic installed
/// here" rather than "did a scan of it find anything". This scan used to carry
/// a guard that announced a Heroic root only once its scan had produced a game,
/// and that guard reported two identical situations differently: an empty Steam
/// library was counted and an installed Heroic with nothing installed
/// disappeared. Both are now counted, because both are somewhere games would be
/// if the user had any.
///
/// Takes the launchers rather than calling [`launcher::all`](dxray_core::launcher::all)
/// itself, so a test can hand it a launcher whose libraries hold no games —
/// the case the removed guard silenced, and the one no real machine can be
/// relied upon to have.
fn scan_launchers(
    launchers: &[&dyn dxray_core::Launcher],
    cancellation: &Cancellation,
    handle: &ProgramHandle<Msg>,
) -> bool {
    let mut stream = Stream {
        cancellation,
        handle,
        builds: Builds::default(),
        root: None,
    };
    dxray_core::walk(launchers, &mut stream).is_continue()
}

/// Sends each thing the walk finds to the running UI.
///
/// Every method stops the walk — [`ControlFlow::Break`] — when cancellation has
/// been requested or the channel has gone away, which is the same "return
/// false" the hand-written loop used to do at every one of these points. The
/// flag is polled between filesystem operations, because that is where these
/// methods are called from, so pressing Esc during a walk on a stale mount
/// still stops it at the next boundary.
struct Stream<'a> {
    cancellation: &'a Cancellation,
    handle: &'a ProgramHandle<Msg>,
    builds: Builds,
    root: Option<PathBuf>,
}

impl Stream<'_> {
    /// `Break` when the scan must stop: nobody is listening, or the user asked.
    fn live(&self) -> ControlFlow<()> {
        if self.cancellation.is_cancelled() {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }

    /// A message whose arrival matters: losing the channel ends the scan.
    fn send(&self, message: Msg) -> ControlFlow<()> {
        self.live()?;
        if self.handle.send(message).is_err() {
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    }

    /// A message that is worth sending and not worth stopping for, which is
    /// what a note or a problem is: the scan carries on either way.
    fn tell(&self, message: Msg) -> ControlFlow<()> {
        self.live()?;
        let _ = self.handle.send(message);
        ControlFlow::Continue(())
    }
}

impl dxray_core::Visitor for Stream<'_> {
    fn root(&mut self, origin: dxray_core::Origin, root: &Path) -> ControlFlow<()> {
        self.root = Some(root.to_path_buf());
        self.send(Msg::Root(crate::msg::ScanLocation {
            origin,
            path: root.to_path_buf(),
        }))
    }

    fn note(&mut self, origin: dxray_core::Origin, note: &str) -> ControlFlow<()> {
        self.tell(Msg::Note(crate::msg::ScanDiagnostic {
            origin,
            root: self.root.clone(),
            library: None,
            message: note.to_owned(),
        }))
    }

    fn problem(&mut self, origin: dxray_core::Origin, problem: &str) -> ControlFlow<()> {
        self.tell(Msg::Problem(crate::msg::ScanDiagnostic {
            origin,
            root: self.root.clone(),
            library: None,
            message: problem.to_owned(),
        }))
    }

    fn library(&mut self, origin: dxray_core::Origin, library: &Path) -> ControlFlow<()> {
        self.send(Msg::Library(crate::msg::ScanLocation {
            origin,
            path: library.to_path_buf(),
        }))
    }

    fn catalogue(
        &mut self,
        launcher: &dyn dxray_core::Launcher,
        library: &Path,
        catalogue: dxray_core::Catalogue,
    ) -> ControlFlow<()> {
        let origin = launcher.origin();
        for note in catalogue.notes {
            self.tell(Msg::Note(crate::msg::ScanDiagnostic {
                origin,
                root: self.root.clone(),
                library: Some(library.to_path_buf()),
                message: note,
            }))?;
        }
        for problem in catalogue.problems {
            self.tell(Msg::Problem(crate::msg::ScanDiagnostic {
                origin,
                root: self.root.clone(),
                library: Some(library.to_path_buf()),
                message: problem,
            }))?;
        }
        for game in catalogue.games {
            self.live()?;
            // `walk` supplies the same inventory to CLI and TUI. Do not
            // classify or deduplicate it again here: Steam's DownloadType
            // does not reliably distinguish games from tools.
            // One call, shared with `dxray --installed`, so that the facts
            // established about a game cannot depend on which surface asked.
            // The launchers' capability difference is spent inside it rather
            // than here: a Steam appid names a compatibility prefix, and a
            // native identity is told so in a sentence naming its own launcher.
            let facts = dxray_core::inspect(&game, library, &mut self.builds);
            self.live()?;
            let entry = Entry::build(game, library.to_path_buf(), facts.survey, facts.nvapi);
            self.send(Msg::Game(Box::new(entry)))?;
        }
        ControlFlow::Continue(())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashSet,
        fs,
        path::PathBuf,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        thread,
        time::Duration,
    };

    use super::{Cancellation, Workers, hand_back};
    use crate::msg::Msg;
    use dxray_core::launcher::identity;

    type Diagnostic = (dxray_core::Origin, Option<PathBuf>, Option<PathBuf>, String);

    #[test]
    fn cancellation_is_shared_between_worker_clones() {
        let cancellation = Cancellation::default();
        let worker = cancellation.clone();
        cancellation.cancel();
        assert!(worker.is_cancelled());
    }

    #[test]
    fn library_identity_deduplicates_literal_paths_and_symlinked_libraries() {
        let mut listed = HashSet::new();
        let missing = PathBuf::from("/dxray-tui-no-such-library");
        assert!(listed.insert(identity(&missing)));
        assert!(!listed.insert(identity(&missing)));

        #[cfg(unix)]
        {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let root = std::env::temp_dir().join(format!(
                "dxray-tui-library-identity-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let target = root.join("library");
            let link = root.join("library-link");
            fs::create_dir_all(&target).expect("test library directory");
            std::os::unix::fs::symlink(&target, &link).expect("test library symlink");

            assert!(listed.insert(identity(&target)));
            assert!(
                !listed.insert(identity(&link)),
                "a library and its symlink must be scanned once"
            );

            fs::remove_dir_all(root).expect("remove test library directory");
        }
    }

    #[test]
    fn the_terminal_comes_back_before_the_workers_are_waited_for() {
        // The order is the whole fix. Cancellation is only polled between
        // filesystem operations, so a worker inside a walk on a stale mount
        // holds the join for as long as the mount takes; doing that first left
        // the user looking at a frozen alternate screen after pressing Esc.
        let order = Arc::new(Mutex::new(Vec::new()));
        let worker = Arc::clone(&order);
        let workers = Workers {
            cancellation: Cancellation::default(),
            handles: vec![thread::spawn(move || {
                thread::sleep(Duration::from_millis(50));
                worker.lock().expect("test mutex").push("joined");
            })],
        };

        let restoring = Arc::clone(&order);
        hand_back(
            move || {
                restoring.lock().expect("test mutex").push("restored");
                Ok(())
            },
            Some(workers),
        )
        .expect("restoration succeeds in this test");

        assert_eq!(
            *order.lock().expect("test mutex"),
            ["restored", "joined"],
            "the shell is handed back first, and the join still happens"
        );
    }

    #[test]
    fn a_failure_before_any_worker_started_still_restores_the_terminal() {
        // `run_terminal` can fail on its first frame, before `Workers::new`.
        // The restoration is not conditional on there being anything to join.
        let restored = Arc::new(Mutex::new(false));
        let flag = Arc::clone(&restored);

        hand_back(
            move || {
                *flag.lock().expect("test mutex") = true;
                Ok(())
            },
            None,
        )
        .expect("restoration succeeds in this test");

        assert!(*restored.lock().expect("test mutex"));
    }

    /// A launcher with exactly the shape the removed guard used to hide: it is
    /// installed, it has a library, and the library holds no games.
    struct Fake {
        origin: dxray_core::Origin,
        roots: Vec<PathBuf>,
        games: Vec<dxray_core::Game>,
        notes: Vec<String>,
        /// What this launcher's catalogue reports as costing nothing — a stale
        /// Heroic record for a game another cache supplied, in the real case.
        /// Held apart from `notes`, which are the *library* notes: the two
        /// arrive by different routes and only one of them had a test.
        catalogue_notes: Vec<String>,
    }

    impl dxray_core::Launcher for Fake {
        fn origin(&self) -> dxray_core::Origin {
            self.origin
        }

        fn roots(&self) -> Vec<PathBuf> {
            self.roots.clone()
        }

        /// Nowhere: this launcher is handed its roots and looks for none.
        fn candidate_roots(&self) -> Vec<PathBuf> {
            Vec::new()
        }

        fn libraries(&self, root: &std::path::Path) -> dxray_core::Libraries {
            dxray_core::Libraries {
                paths: vec![root.to_path_buf()],
                notes: self.notes.clone(),
                problems: Vec::new(),
            }
        }

        fn games(&self, _library: &std::path::Path) -> dxray_core::Catalogue {
            dxray_core::Catalogue {
                games: self.games.clone(),
                problems: Vec::new(),
                notes: self.catalogue_notes.clone(),
            }
        }
    }

    struct ReportingFixture {
        origin: dxray_core::Origin,
        root: PathBuf,
    }

    impl dxray_core::Launcher for ReportingFixture {
        fn origin(&self) -> dxray_core::Origin {
            self.origin
        }

        fn roots(&self) -> Vec<PathBuf> {
            vec![self.root.clone()]
        }

        fn candidate_roots(&self) -> Vec<PathBuf> {
            vec![self.root.clone()]
        }

        fn libraries(&self, root: &std::path::Path) -> dxray_core::Libraries {
            dxray_core::Libraries {
                paths: vec![root.join("library")],
                notes: vec!["index caveat".to_owned()],
                problems: vec!["index problem".to_owned()],
            }
        }

        fn games(&self, _library: &std::path::Path) -> dxray_core::Catalogue {
            dxray_core::Catalogue {
                notes: vec!["catalogue caveat".to_owned()],
                problems: vec!["catalogue problem".to_owned()],
                ..dxray_core::Catalogue::default()
            }
        }
    }

    fn drain(launchers: &[&dyn dxray_core::Launcher]) -> Vec<Msg> {
        let (handle, receiver) = ratatui_tea::channel();
        assert!(
            super::scan_launchers(launchers, &Cancellation::default(), &handle),
            "a scan nobody cancelled and whose receiver is alive must run to the end"
        );
        drop(handle);
        receiver.try_iter().collect()
    }

    #[test]
    fn an_installed_launcher_whose_library_holds_no_games_is_still_counted() {
        // The whole point of the slice. An empty Steam library has always been
        // announced; an installed Heroic with nothing installed used to vanish
        // from the header, and those are the same situation. The counter the
        // user reads says "libraries games could be in", not "libraries that
        // happened to have one".
        let empty = Fake {
            origin: dxray_core::Origin::new("empty", "Empty"),
            roots: vec![PathBuf::from("/dxray-tui-empty-install")],
            games: Vec::new(),
            notes: Vec::new(),
            catalogue_notes: Vec::new(),
        };
        let messages = drain(&[&empty]);

        assert!(
            messages.iter().any(|msg| matches!(
                msg,
                Msg::Root(location)
                    if location.origin == empty.origin && location.path == empty.roots[0]
            )),
            "the install must be announced: {messages:?}"
        );
        assert!(
            messages.iter().any(|msg| matches!(
                msg,
                Msg::Library(location)
                    if location.origin == empty.origin && location.path == empty.roots[0]
            )),
            "its library must be counted even with no game in it: {messages:?}"
        );
        assert!(
            !messages.iter().any(|msg| matches!(msg, Msg::Game(_))),
            "and no game may be invented to justify the count: {messages:?}"
        );
    }

    #[test]
    fn a_catalogue_note_reaches_the_browser_under_its_launcher_s_name() {
        // A note computed and never drawn is the failure this project names
        // explicitly, and catalogue notes had exactly that shape here: the
        // loop that forwards them could be deleted whole and every test still
        // passed, because the launcher in these tests reported none.
        //
        // They are notes and not problems on purpose — a stale record for a
        // game another cache supplied cost nobody anything — so this asserts
        // the channel they arrive on as well as that they arrive.
        let launcher = Fake {
            origin: dxray_core::Origin::new("noted", "Noted"),
            roots: vec![PathBuf::from("/dxray-tui-noted")],
            games: Vec::new(),
            notes: Vec::new(),
            catalogue_notes: vec!["a record with no install_path, found elsewhere".to_owned()],
        };
        let messages = drain(&[&launcher]);

        assert!(
            messages.iter().any(|msg| matches!(
                msg,
                Msg::Note(note)
                    if note.origin.label() == "Noted"
                        && note.message == "a record with no install_path, found elsewhere"
            )),
            "the catalogue note must be shown, with the launcher that made it: {messages:?}"
        );
        assert!(
            !messages.iter().any(|msg| matches!(msg, Msg::Problem(_))),
            "and as a note, not a problem: {messages:?}"
        );
    }

    #[test]
    fn tui_emits_the_core_diagnostics_with_their_origins() {
        let fixture = ReportingFixture {
            origin: dxray_core::Origin::new("fixture", "Fixture"),
            root: PathBuf::from("/dxray-tui-diagnostics"),
        };
        let inventory = dxray_core::Inventory::collect(&[&fixture]);
        let expected_notes: Vec<_> = inventory
            .notes
            .iter()
            .map(|diagnostic| {
                (
                    diagnostic.origin,
                    diagnostic.root.clone(),
                    diagnostic.library.clone(),
                    diagnostic.message.clone(),
                )
            })
            .collect();
        let expected_problems: Vec<_> = inventory
            .problems
            .iter()
            .map(|diagnostic| {
                (
                    diagnostic.origin,
                    diagnostic.root.clone(),
                    diagnostic.library.clone(),
                    diagnostic.message.clone(),
                )
            })
            .collect();
        let messages = drain(&[&fixture]);
        let notes: Vec<_> = messages
            .iter()
            .filter_map(|message| match message {
                Msg::Note(note) => Some((
                    note.origin,
                    note.root.clone(),
                    note.library.clone(),
                    note.message.clone(),
                )),
                _ => None,
            })
            .collect();
        let problems: Vec<_> = messages
            .into_iter()
            .filter_map(|message| match message {
                Msg::Problem(problem) => Some((
                    problem.origin,
                    problem.root,
                    problem.library,
                    problem.message,
                )),
                _ => None,
            })
            .collect();

        assert_eq!(notes, expected_notes);
        assert_eq!(problems, expected_problems);
    }

    #[test]
    fn every_launcher_is_scanned_and_its_messages_carry_its_own_name() {
        // The loop is written once and run over the registry, so a launcher
        // added to `launcher::all` is scanned without editing this file. The
        // prefix comes from the launcher rather than a literal, which is what
        // stops a third launcher inheriting a sentence about Steam.
        let first = Fake {
            origin: dxray_core::Origin::new("first", "First"),
            roots: vec![PathBuf::from("/dxray-tui-first")],
            games: vec![dxray_core::Game {
                identity: dxray_core::Identity::Native("only-game".to_owned()),
                name: "Only Game".to_owned(),
                install_dir: PathBuf::from("/dxray-tui-first/only-game"),
                origin: dxray_core::Origin::new("first", "First / Shop"),
            }],
            notes: Vec::new(),
            catalogue_notes: Vec::new(),
        };
        let second = Fake {
            origin: dxray_core::Origin::new("second", "Second"),
            roots: vec![PathBuf::from("/dxray-tui-second")],
            games: Vec::new(),
            notes: vec!["its index declared no library".to_owned()],
            catalogue_notes: Vec::new(),
        };
        let messages = drain(&[&first, &second]);

        let roots: Vec<_> = messages
            .iter()
            .filter_map(|msg| match msg {
                Msg::Root(location) => Some(location.path.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            roots,
            vec![
                PathBuf::from("/dxray-tui-first"),
                PathBuf::from("/dxray-tui-second")
            ],
            "both launchers are visited, in registry order"
        );
        assert_eq!(
            messages
                .iter()
                .filter(|msg| matches!(msg, Msg::Game(_)))
                .count(),
            1,
            "the launcher that has a game reports it: {messages:?}"
        );
        assert!(
            messages.iter().any(|msg| matches!(
                msg,
                Msg::Note(note)
                    if note.origin.label() == "Second"
                        && note.message == "its index declared no library"
            )),
            "a note is prefixed with the launcher that raised it: {messages:?}"
        );
    }

    #[test]
    fn a_cancelled_scan_stops_without_finishing_the_launcher_list() {
        // `false` is the worker's instruction to stop silently, and it must be
        // reachable from inside the uniform loop rather than only from the Steam
        // half of the old one.
        let launcher = Fake {
            origin: dxray_core::Origin::new("first", "First"),
            roots: vec![PathBuf::from("/dxray-tui-cancelled")],
            games: Vec::new(),
            notes: Vec::new(),
            catalogue_notes: Vec::new(),
        };
        let cancellation = Cancellation::default();
        cancellation.cancel();
        let (handle, receiver) = ratatui_tea::channel();

        assert!(
            !super::scan_launchers(&[&launcher], &cancellation, &handle),
            "a cancelled scan reports that the worker must stop"
        );
        drop(handle);
        assert_eq!(
            receiver.try_iter().count(),
            0,
            "and says nothing to a UI that is going away"
        );
    }

    #[test]
    fn tui_stream_matches_the_shared_steam_and_heroic_fixture() {
        let fixture = dxray_core::test_support::SteamHeroicFixture::new();
        let launchers = fixture.launchers();
        let inventory = fixture.inventory();
        let messages = drain(&launchers);

        let entries: Vec<_> = messages
            .iter()
            .filter_map(|message| match message {
                Msg::Game(entry) => {
                    Some((entry.identity.clone(), entry.origin, entry.library.clone()))
                }
                _ => None,
            })
            .collect();
        let roots: Vec<_> = messages
            .iter()
            .filter_map(|message| match message {
                Msg::Root(location) => Some((location.origin, location.path.clone())),
                _ => None,
            })
            .collect();
        let libraries: Vec<_> = messages
            .iter()
            .filter_map(|message| match message {
                Msg::Library(location) => Some((location.origin, location.path.clone())),
                _ => None,
            })
            .collect();
        let notes: Vec<_> = messages
            .iter()
            .filter_map(|message| match message {
                Msg::Note(note) => Some((
                    note.origin,
                    note.root.clone(),
                    note.library.clone(),
                    note.message.clone(),
                )),
                _ => None,
            })
            .collect();
        let problems: Vec<_> = messages
            .iter()
            .filter_map(|message| match message {
                Msg::Problem(problem) => Some((
                    problem.origin,
                    problem.root.clone(),
                    problem.library.clone(),
                    problem.message.clone(),
                )),
                _ => None,
            })
            .collect();

        assert_eq!(
            entries,
            inventory
                .entries
                .iter()
                .map(|entry| (
                    entry.game.identity.clone(),
                    entry.game.origin,
                    entry.library.clone(),
                ))
                .collect::<Vec<_>>(),
        );
        assert_eq!(
            roots,
            inventory
                .roots
                .iter()
                .map(|root| (root.origin, root.path.clone()))
                .collect::<Vec<_>>(),
        );
        assert_eq!(
            libraries,
            inventory
                .libraries
                .iter()
                .map(|library| (library.origin, library.path.clone()))
                .collect::<Vec<_>>(),
        );
        assert_eq!(notes, diagnostics(&inventory.notes));
        assert_eq!(problems, diagnostics(&inventory.problems));
    }

    fn diagnostics(diagnostics: &[dxray_core::InventoryDiagnostic]) -> Vec<Diagnostic> {
        diagnostics
            .iter()
            .map(|diagnostic| {
                (
                    diagnostic.origin,
                    diagnostic.root.clone(),
                    diagnostic.library.clone(),
                    diagnostic.message.clone(),
                )
            })
            .collect()
    }
}
