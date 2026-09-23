//! The keys this program has an opinion about. An enum of its own, so tests
//! type [`Key::Char`]; [`from_crossterm`] is the only function that knows what
//! a terminal event looks like.

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

/// What a terminal event means here, if anything. Releases are ignored: under
/// the Kitty protocol they would move the cursor twice per keypress.
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
    // Repeats are deliberate input, so held navigation keys work.
    if !matches!(kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return None;
    }
    if *modifiers == KeyModifiers::CONTROL {
        // Only Ctrl-C is claimed, so `Ctrl-L` does not type an `l`.
        return match code {
            KeyCode::Char('c') => Some(Key::Interrupt),
            _ => None,
        };
    }
    if (matches!(code, KeyCode::BackTab)
        && matches!(*modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT))
        || (matches!(code, KeyCode::Tab) && *modifiers == KeyModifiers::SHIFT)
    {
        // Shift-Tab arrives as `BackTab` or as `Tab` plus Shift.
        return Some(Key::BackTab);
    }
    let key = match code {
        // Shift is text (`Q` arrives resolved); Alt, Super and the rest are not,
        // so chords never reach the filter.
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
        // Counting releases would move two rows per press.
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
        // An upper-case letter reaches the filter as typed.
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
