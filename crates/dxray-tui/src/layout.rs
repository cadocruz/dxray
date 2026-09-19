//! Geometry shared by the model and the renderer.

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// The parts of the screen used by the browser.
///
/// Keeping these calculations here ensures that the reducer clamps scroll
/// offsets against precisely the same rounded layout that the renderer uses.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Areas {
    pub(crate) header: Rect,
    pub(crate) list: Rect,
    pub(crate) detail: Rect,
    pub(crate) help: Rect,
}

#[must_use]
pub(crate) fn areas(area: Rect) -> Areas {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(area);
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
        .split(outer[1]);

    Areas {
        header: outer[0],
        list: body[0],
        detail: body[1],
        help: outer[2],
    }
}

/// Inner viewport of the bordered detail panel for a terminal size.
#[must_use]
pub(crate) fn detail_viewport(terminal_width: u16, terminal_height: u16) -> Rect {
    let detail = areas(Rect::new(0, 0, terminal_width, terminal_height)).detail;
    Rect::new(
        detail.x.saturating_add(1),
        detail.y.saturating_add(1),
        detail.width.saturating_sub(2),
        detail.height.saturating_sub(2),
    )
}

/// Rows available to game names after the list's border is accounted for.
///
/// The application header and help line take four terminal rows. Keeping this
/// calculation outside the renderer makes page movement agree with what is
/// actually visible.
#[must_use]
pub fn list_rows(terminal_height: u16) -> usize {
    usize::from(terminal_height.saturating_sub(5)).max(1)
}

/// Rows available inside the bordered detail panel.
///
/// Both panes live below the two-line header and above the one-line help, and
/// both have a one-cell border on their top and bottom.
#[must_use]
pub fn detail_rows(terminal_height: u16) -> usize {
    usize::from(detail_viewport(1, terminal_height).height).max(1)
}

/// Approximate the inner width of the right-hand detail panel.
#[must_use]
pub fn detail_width(terminal_width: u16) -> u16 {
    detail_viewport(terminal_width, 5).width.max(1)
}
