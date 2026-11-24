//! Phosphor-green terminal palette.
//!
//! Everything is green on black: dim green for secondary text, normal green
//! for content, bright and white-green for emphasis. The optional *warning
//! colours* flag (`t`) adds amber for "starved"-type warnings and red for
//! "blocked"/alerts. Nothing else ever gets a colour.

use chainlab_engine::engine::model::{LogKind, Status};
use ratatui::style::{Color, Modifier, Style};

pub const BG: Color = Color::Black;
pub const DIM: Color = Color::Indexed(28);
pub const FG: Color = Color::Indexed(40);
pub const BRIGHT: Color = Color::Indexed(46);
pub const EMPH: Color = Color::Indexed(157);
pub const AMBER: Color = Color::Indexed(214);
pub const RED: Color = Color::Indexed(196);

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub warn_colors: bool,
}

impl Theme {
    pub fn base(&self) -> Style {
        Style::new().fg(FG).bg(BG)
    }
    pub fn dim(&self) -> Style {
        Style::new().fg(DIM).bg(BG)
    }
    pub fn bright(&self) -> Style {
        Style::new().fg(BRIGHT).bg(BG).add_modifier(Modifier::BOLD)
    }
    pub fn emph(&self) -> Style {
        Style::new().fg(EMPH).bg(BG).add_modifier(Modifier::BOLD)
    }
    /// Soft warning (starved, low).
    pub fn warn(&self) -> Style {
        if self.warn_colors {
            Style::new().fg(AMBER).bg(BG).add_modifier(Modifier::BOLD)
        } else {
            self.bright()
        }
    }
    /// Hard alert (blocked, debt, failure).
    pub fn alert(&self) -> Style {
        if self.warn_colors {
            Style::new().fg(RED).bg(BG).add_modifier(Modifier::BOLD)
        } else {
            self.emph()
        }
    }
    /// Selected row / inverse video.
    pub fn sel(&self) -> Style {
        Style::new().fg(BG).bg(FG).add_modifier(Modifier::BOLD)
    }
    pub fn status(&self, s: Status) -> Style {
        match s {
            Status::Working => self.base(),
            Status::Starved | Status::Unstaffed => self.warn(),
            Status::Blocked => self.alert(),
            Status::Off => self.dim(),
        }
    }
    pub fn log(&self, k: LogKind) -> Style {
        match k {
            LogKind::Info | LogKind::Lever => self.dim(),
            LogKind::Purchase => self.base(),
            LogKind::Event => self.warn(),
            LogKind::Unlock | LogKind::Challenge => self.emph(),
            LogKind::Warn => self.alert(),
        }
    }
}
