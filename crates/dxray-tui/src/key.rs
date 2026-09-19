//! The keys this program has an opinion about, and nothing else.
//!
//! A small enum of its own rather than `crossterm::event::KeyEvent`, for the
//! same reason [`analysis`](dxray_core::analysis) is not allowed to open a
//! file: it puts every judgement on the side of the line that can be tested
//! without the thing being judged. A test that drives this program types
//! [`Key::Char`], not a struct with a modifier bitfield and a kind and a state.
//!
//! [`from_crossterm`] is the whole of the other side, and it is the only
//! function in this crate that knows what a terminal event looks like.

use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// A keypress, reduced to what the program does with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Backspace,
    Enter,
    /// Switches keyboard navigation between the game list and its details.
    Tab,
    /// Switches keyboard navigation in the reverse direction.
    BackTab,
    Escape,
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    /// `Ctrl-C`. Kept apart from [`Key::Char`] because a `c` typed into the
    /// filter box is a letter and this never is.
    Interrupt,
}

/// What a terminal event means here, if it means anything.
///
/// Returns `None` for every event this program ignores, which is most of them:
/// mouse movement, focus changes, bracketed paste, and the key *release* half
/// of every press on terminals that report both. That last one matters more
/// than it sounds — on a terminal with the Kitty keyboard protocol enabled,
/// treating releases as presses moves the cursor two rows per keypress.
#[must_use]
pub fn from_crossterm(event: &Event) -> Option<Key> {
    let Event::Key(KeyEvent {
        code,
        modifiers,
        kind,
        ..
    }) = event
    else {
        return None;
    };
    // Repeats are distinct from presses in enhanced terminal protocols. They
    // are deliberate input, unlike releases, and make held navigation keys
    // behave like they do in every other terminal program.
    if !matches!(kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return None;
    }
    if *modifiers == KeyModifiers::CONTROL {
        // Only one control chord is claimed. Everything else is left alone
        // rather than being flattened into its letter, so `Ctrl-L` does not
        // silently type an `l` into the filter box.
        return match code {
            KeyCode::Char('c') => Some(Key::Interrupt),
            _ => None,
        };
    }
    if (matches!(code, KeyCode::BackTab)
        && matches!(*modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT))
        || (matches!(code, KeyCode::Tab) && *modifiers == KeyModifiers::SHIFT)
    {
        // Crossterm represents Shift-Tab as `BackTab` on terminals that can
        // distinguish it. Some terminal protocols instead retain `Tab` and
        // put Shift in the modifiers, so support both encodings.
        return Some(Key::BackTab);
    }
    let key = match code {
        // Crossterm reports a shifted printable key as its resolved character
        // (for example `Q`), so Shift remains valid text input. Alt, Super and
        // other modifiers are not text: swallowing them prevents shortcut
        // chords from unexpectedly changing the filter.
        KeyCode::Char(c) if matches!(*modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            Key::Char(*c)
        }
        // Commands intentionally require an exact, unmodified key. In
        // particular Alt-Down and Shift-Up must not become navigation.
        KeyCode::Backspace
        | KeyCode::Enter
        | KeyCode::Tab
        | KeyCode::Esc
        | KeyCode::Up
        | KeyCode::Down
        | KeyCode::PageUp
        | KeyCode::PageDown
        | KeyCode::Home
        | KeyCode::End
            if *modifiers != KeyModifiers::NONE =>
        {
            return None;
        }
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Enter => Key::Enter,
        KeyCode::Tab => Key::Tab,
        KeyCode::Esc => Key::Escape,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        _ => return None,
    };
    Some(key)
}

#[cfg(test)]
mod tests {
    use super::{Key, from_crossterm};
    use ratatui::crossterm::event::{
        Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers,
    };

    fn key(code: KeyCode, modifiers: KeyModifiers, kind: KeyEventKind) -> Event {
        Event::Key(KeyEvent {
            code,
            modifiers,
            kind,
            state: KeyEventState::NONE,
        })
    }

    fn press(code: KeyCode, modifiers: KeyModifiers) -> Event {
        key(code, modifiers, KeyEventKind::Press)
    }

    #[test]
    fn a_key_release_is_not_a_second_keypress() {
        // On a terminal that reports releases — anything speaking the Kitty
        // keyboard protocol — counting them moves the cursor two rows for every
        // one press, which reads as a broken program rather than as a terminal
        // difference.
        let release = Event::Key(KeyEvent {
            code: KeyCode::Down,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Release,
            state: KeyEventState::NONE,
        });

        assert_eq!(from_crossterm(&release), None);
        assert_eq!(
            from_crossterm(&press(KeyCode::Down, KeyModifiers::NONE)),
            Some(Key::Down)
        );
    }

    #[test]
    fn a_key_repeat_is_the_same_supported_command_as_a_press() {
        assert_eq!(
            from_crossterm(&key(
                KeyCode::Down,
                KeyModifiers::NONE,
                KeyEventKind::Repeat
            )),
            Some(Key::Down)
        );
        assert_eq!(
            from_crossterm(&key(
                KeyCode::Char('a'),
                KeyModifiers::NONE,
                KeyEventKind::Repeat
            )),
            Some(Key::Char('a'))
        );
        assert_eq!(
            from_crossterm(&key(
                KeyCode::Tab,
                KeyModifiers::SHIFT,
                KeyEventKind::Repeat
            )),
            Some(Key::BackTab)
        );
    }

    #[test]
    fn a_control_chord_is_never_flattened_into_its_letter() {
        // `Ctrl-L` reaching the filter box as an `l` is a filter the user did
        // not type, narrowing a list for a reason they cannot see.
        assert_eq!(
            from_crossterm(&press(KeyCode::Char('l'), KeyModifiers::CONTROL)),
            None
        );
        assert_eq!(
            from_crossterm(&press(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(Key::Interrupt),
            "the one chord that is claimed, because it must not type a letter"
        );
    }

    #[test]
    fn a_shifted_letter_is_the_letter_the_terminal_reports() {
        // Crossterm resolves the shift itself, so an upper-case name typed into
        // the filter arrives upper-cased. The filter lower-cases both sides, and
        // this is the half that must not swallow the key.
        assert_eq!(
            from_crossterm(&press(KeyCode::Char('Q'), KeyModifiers::SHIFT)),
            Some(Key::Char('Q'))
        );
    }

    #[test]
    fn tab_is_reserved_for_pane_focus() {
        assert_eq!(
            from_crossterm(&press(KeyCode::Tab, KeyModifiers::NONE)),
            Some(Key::Tab)
        );
    }

    #[test]
    fn backtab_and_shift_tab_reverse_pane_focus() {
        assert_eq!(
            from_crossterm(&press(KeyCode::BackTab, KeyModifiers::NONE)),
            Some(Key::BackTab)
        );
        assert_eq!(
            from_crossterm(&press(KeyCode::Tab, KeyModifiers::SHIFT)),
            Some(Key::BackTab)
        );
    }

    #[test]
    fn unsupported_modifiers_do_not_trigger_commands_or_filtering() {
        assert_eq!(
            from_crossterm(&press(KeyCode::Down, KeyModifiers::ALT)),
            None
        );
        assert_eq!(
            from_crossterm(&press(KeyCode::Up, KeyModifiers::SHIFT)),
            None
        );
        assert_eq!(
            from_crossterm(&press(KeyCode::Tab, KeyModifiers::ALT)),
            None
        );
        assert_eq!(
            from_crossterm(&press(KeyCode::Char('f'), KeyModifiers::ALT)),
            None
        );
        assert_eq!(
            from_crossterm(&press(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            None
        );
    }
}
