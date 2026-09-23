//! Terminal setup and restoration.

use std::{
    io::{self, Stdout},
    panic,
    sync::Once,
};

use ratatui::crossterm::{
    cursor::{Hide, Show},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

static PANIC_HOOK: Once = Once::new();

/// Enters raw mode and returns an alternate-screen terminal.
///
/// # Errors
///
/// Returns an error if crossterm cannot configure the current terminal or if
/// ratatui cannot construct its terminal backend.
pub fn enter() -> io::Result<Tui> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    if let Err(error) = execute!(stdout, EnterAlternateScreen) {
        let _ = restore();
        return Err(error);
    }
    if let Err(error) = execute!(stdout, Hide) {
        // Undo every step: a failed `Hide` can follow a successful switch.
        let _ = restore();
        return Err(error);
    }
    match Terminal::new(CrosstermBackend::new(stdout)) {
        Ok(terminal) => Ok(terminal),
        Err(error) => {
            // `Terminal::new` owns (and drops) the backend before returning,
            // so a fresh stdout handle is safe for the rollback.
            let _ = restore();
            Err(error)
        }
    }
}

/// Installs the panic hook once; it restores the terminal, then calls the
/// previous hook.
pub fn install_panic_hook() {
    PANIC_HOOK.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let _ = restore();
            previous(info);
        }));
    });
}

/// Restores each terminal-wide setting independently, including the cursor.
///
/// # Errors
///
/// Returns the first restoration error after attempting all three operations.
pub fn restore() -> io::Result<()> {
    restore_with(
        disable_raw_mode,
        || {
            let mut stdout = io::stdout();
            execute!(stdout, LeaveAlternateScreen)
        },
        || {
            let mut stdout = io::stdout();
            execute!(stdout, Show)
        },
    )
}

fn restore_with<Raw, Alternate, Cursor>(
    raw: Raw,
    alternate: Alternate,
    cursor: Cursor,
) -> io::Result<()>
where
    Raw: FnOnce() -> io::Result<()>,
    Alternate: FnOnce() -> io::Result<()>,
    Cursor: FnOnce() -> io::Result<()>,
{
    let raw = raw();
    let alternate = alternate();
    let cursor = cursor();
    raw.and(alternate).and(cursor)
}

#[cfg(test)]
mod tests {
    use std::{io, sync::Mutex};

    use super::restore_with;

    #[test]
    fn restoration_attempts_every_step_after_an_error() {
        let steps = Mutex::new(Vec::new());
        let error = restore_with(
            || {
                steps.lock().unwrap().push("raw");
                Err(io::Error::other("raw"))
            },
            || {
                steps.lock().unwrap().push("alternate");
                Ok(())
            },
            || {
                steps.lock().unwrap().push("cursor");
                Ok(())
            },
        )
        .unwrap_err();

        assert_eq!(error.to_string(), "raw");
        assert_eq!(*steps.lock().unwrap(), ["raw", "alternate", "cursor"]);
    }
}
