//! Native window front end.
//!
//! CHAINLAB in a desktop window instead of a terminal, with exactly the
//! same screens, keys and behaviour. Every frame:
//!
//! 1. egui keyboard events are translated into the same key events the
//!    terminal UI uses and fed to [`App::on_key`];
//! 2. the simulation advances by real elapsed time at the chosen speed;
//! 3. the ratatui UI is drawn into an off-screen character grid
//!    (ratatui's `TestBackend`) sized to fit the window;
//! 4. that grid is painted with egui's monospace font, green on black.
//!
//! Resize the window for more room. Cmd/Ctrl +/- zooms (egui built-in).

use std::time::Instant;

use chainlab_engine::Sim;
use chainlab_engine::challenges::ChallengeId;
use chainlab_engine::scenarios::Loaded;
use eframe::egui::{self, Align2, Color32, Event, FontId, Key, Pos2, Rect, Vec2};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Modifier};

use crate::ui::{App, panels};

/// Default font size in points.
pub const FONT_SIZE: f32 = 15.0;
const BG: Color32 = Color32::BLACK;
/// Fallback foreground for `Color::Reset` (normal phosphor green).
const FG: Color32 = Color32::from_rgb(0x00, 0xd7, 0x00);

pub struct Gui {
    app: App,
    term: Terminal<TestBackend>,
    last: Instant,
}

impl Gui {
    pub fn new(app: App) -> Gui {
        let term = Terminal::new(TestBackend::new(100, 35)).expect("in-memory backend");
        Gui {
            app,
            term,
            last: Instant::now(),
        }
    }

    fn handle_input(&mut self, ctx: &egui::Context) {
        let events = ctx.input(|i| i.events.clone());
        for e in events {
            for k in translate(&e) {
                self.app.on_key(k);
            }
        }
    }

    /// Draw the UI into the grid and paint it into `rect`.
    fn paint(&mut self, ui: &egui::Ui, rect: Rect) {
        let font = FontId::monospace(FONT_SIZE);
        let (cw, rh) = ui.fonts(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
        let cols = ((rect.width() / cw).floor() as u16).clamp(20, 400);
        let rows = ((rect.height() / rh).floor() as u16).clamp(8, 200);
        let size = self.term.backend().buffer().area;
        if size.width != cols || size.height != rows {
            self.term.backend_mut().resize(cols, rows);
        }
        let app = &self.app;
        let _ = self.term.draw(|f| panels::draw(f, app));
        // Centre the grid in the window.
        let origin = Pos2::new(
            rect.left() + ((rect.width() - cols as f32 * cw) / 2.0).max(0.0),
            rect.top() + ((rect.height() - rows as f32 * rh) / 2.0).max(0.0),
        );
        paint_buffer(
            ui.painter(),
            self.term.backend().buffer(),
            origin,
            cw,
            rh,
            &font,
        );
    }
}

impl eframe::App for Gui {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_input(ctx);
        if self.app.quit_requested() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let now = Instant::now();
        let dt = now - self.last;
        self.last = now;
        self.app.advance(dt);
        self.app.background();

        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(BG))
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                self.paint(ui, rect);
            });
        // Keep animating: the sim runs in real time.
        ctx.request_repaint();
    }
}

/// Paint a ratatui buffer as a character grid. Runs of cells with the same
/// style are drawn as one text shape, each run anchored at its own column so
/// alignment never drifts.
pub fn paint_buffer(
    painter: &egui::Painter,
    buf: &Buffer,
    origin: Pos2,
    cw: f32,
    rh: f32,
    font: &FontId,
) {
    let area = buf.area;
    painter.rect_filled(
        Rect::from_min_size(
            origin,
            Vec2::new(area.width as f32 * cw, area.height as f32 * rh),
        ),
        0.0,
        BG,
    );
    for y in 0..area.height {
        let mut x = 0;
        while x < area.width {
            let Some(cell) = buf.cell((x, y)) else { break };
            let (fg, bg, bold) = (cell.fg, cell.bg, cell.modifier.contains(Modifier::BOLD));
            let start = x;
            let mut text = String::new();
            while x < area.width {
                let Some(c) = buf.cell((x, y)) else { break };
                if c.fg != fg || c.bg != bg || c.modifier.contains(Modifier::BOLD) != bold {
                    break;
                }
                let sym = c.symbol();
                text.push_str(if sym.is_empty() { " " } else { sym });
                x += 1;
            }
            let pos = Pos2::new(origin.x + start as f32 * cw, origin.y + y as f32 * rh).round();
            let bgc = to_color32(bg, BG);
            if bgc != BG {
                painter.rect_filled(
                    Rect::from_min_size(pos, Vec2::new((x - start) as f32 * cw, rh)),
                    0.0,
                    bgc,
                );
            }
            if text.trim().is_empty() {
                continue;
            }
            let fgc = to_color32(fg, FG);
            painter.text(pos, Align2::LEFT_TOP, &text, font.clone(), fgc);
            if bold {
                // Fake bold: a second pass offset by a fraction of a pixel.
                painter.text(
                    pos + Vec2::new(0.6, 0.0),
                    Align2::LEFT_TOP,
                    &text,
                    font.clone(),
                    fgc,
                );
            }
        }
    }
}

/// ratatui colour -> egui colour (xterm-256 palette for indexed colours).
pub fn to_color32(c: Color, default: Color32) -> Color32 {
    const BASE: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (205, 0, 0),
        (0, 205, 0),
        (205, 205, 0),
        (0, 0, 238),
        (205, 0, 205),
        (0, 205, 205),
        (229, 229, 229),
        (127, 127, 127),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (92, 92, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    let idx = |i: u8| -> Color32 {
        match i {
            0..=15 => {
                let (r, g, b) = BASE[i as usize];
                Color32::from_rgb(r, g, b)
            }
            16..=231 => {
                let i = i - 16;
                let lv = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
                Color32::from_rgb(lv(i / 36), lv((i / 6) % 6), lv(i % 6))
            }
            _ => {
                let v = 8 + (i - 232) * 10;
                Color32::from_rgb(v, v, v)
            }
        }
    };
    match c {
        Color::Reset => default,
        Color::Black => idx(0),
        Color::Red => idx(1),
        Color::Green => idx(2),
        Color::Yellow => idx(3),
        Color::Blue => idx(4),
        Color::Magenta => idx(5),
        Color::Cyan => idx(6),
        Color::Gray => idx(7),
        Color::DarkGray => idx(8),
        Color::LightRed => idx(9),
        Color::LightGreen => idx(10),
        Color::LightYellow => idx(11),
        Color::LightBlue => idx(12),
        Color::LightMagenta => idx(13),
        Color::LightCyan => idx(14),
        Color::White => idx(15),
        Color::Indexed(i) => idx(i),
        Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
    }
}

/// egui event -> the key events the shared UI understands.
pub fn translate(e: &Event) -> Vec<KeyEvent> {
    match e {
        // Printable characters arrive as text (already shifted: 'R', '>', '{').
        Event::Text(t) => t
            .chars()
            .filter(|c| !c.is_control() && *c != ' ')
            .map(|c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
            .collect(),
        Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } => {
            if modifiers.ctrl && *key == Key::C {
                return vec![KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)];
            }
            if modifiers.command {
                return vec![]; // leave Cmd/Ctrl shortcuts (zoom) to egui
            }
            let shift = if modifiers.shift {
                KeyModifiers::SHIFT
            } else {
                KeyModifiers::NONE
            };
            let code = match key {
                Key::ArrowUp => KeyCode::Up,
                Key::ArrowDown => KeyCode::Down,
                Key::ArrowLeft => KeyCode::Left,
                Key::ArrowRight => KeyCode::Right,
                Key::Tab if modifiers.shift => KeyCode::BackTab,
                Key::Tab => KeyCode::Tab,
                Key::Enter => KeyCode::Enter,
                Key::Escape => KeyCode::Esc,
                Key::PageUp => KeyCode::PageUp,
                Key::PageDown => KeyCode::PageDown,
                Key::F1 => KeyCode::F(1),
                Key::Backspace => KeyCode::Backspace,
                Key::Space => KeyCode::Char(' '),
                _ => return vec![],
            };
            vec![KeyEvent::new(code, shift)]
        }
        _ => vec![],
    }
}

/// Open the window and run until it's closed (or `q`).
pub fn run(loaded: Loaded, sim: Sim, challenge: Option<ChallengeId>) -> Result<(), String> {
    let app = App::start(loaded, sim, challenge);
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("CHAINLAB")
        .with_inner_size([1100.0, 740.0])
        .with_min_inner_size([760.0, 460.0]);
    // Hidden: CHAINLAB_GUI_POS="x,y" places the window (for screenshots).
    if let Some((x, y)) = std::env::var("CHAINLAB_GUI_POS")
        .ok()
        .and_then(|v| {
            v.split_once(',')
                .map(|(a, b)| (a.trim().parse::<f32>(), b.trim().parse::<f32>()))
        })
        .and_then(|(a, b)| Some((a.ok()?, b.ok()?)))
    {
        viewport = viewport.with_position([x, y]);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "CHAINLAB",
        options,
        Box::new(move |_cc| Ok(Box::new(Gui::new(app)))),
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chainlab_engine::scenarios;
    use std::collections::BTreeSet;

    /// Every character any screen can draw must exist in egui's monospace
    /// font, or the grid would show tofu boxes and lose alignment.
    #[test]
    fn monospace_font_has_every_glyph_we_draw() {
        let mut chars: BTreeSet<char> = BTreeSet::new();
        for overlay in ["none", "help", "shop", "market", "menu", "compare"] {
            let loaded = scenarios::load("balanced").unwrap();
            let mut sim = loaded.build(Some(42)).unwrap();
            sim.run(2400);
            let s = crate::ui::snapshot(loaded, sim, 100, 35, overlay, None);
            chars.extend(s.chars().filter(|c| *c != '\n'));
        }
        // Glyphs that only appear in some states.
        chars.extend("╔╗╚╝═║┌┐└┘─│├┤┬┴┼█▓▒░·▁▂▃▄▅▆▇▲▼oO>".chars());
        let ctx = egui::Context::default();
        let mut missing = vec![];
        let mut uneven = vec![];
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            let font = FontId::monospace(FONT_SIZE);
            ctx.fonts(|f| {
                let w = f.glyph_width(&font, 'M');
                for &c in &chars {
                    if !f.has_glyph(&font, c) {
                        missing.push(c);
                    } else if (f.glyph_width(&font, c) - w).abs() > 0.01 {
                        uneven.push((c, f.glyph_width(&font, c), w));
                    }
                }
            });
        });
        assert!(missing.is_empty(), "missing glyphs: {missing:?}");
        assert!(
            uneven.is_empty(),
            "glyphs wider/narrower than a cell: {uneven:?}"
        );
    }

    #[test]
    fn keys_translate_like_a_terminal() {
        let text = translate(&Event::Text("R>".into()));
        assert_eq!(text[0].code, KeyCode::Char('R'));
        assert_eq!(text[1].code, KeyCode::Char('>'));
        let key = |k: Key, m: egui::Modifiers| Event::Key {
            key: k,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: m,
        };
        let t = translate(&key(Key::Tab, egui::Modifiers::SHIFT));
        assert_eq!(t[0].code, KeyCode::BackTab);
        let r = translate(&key(Key::ArrowRight, egui::Modifiers::SHIFT));
        assert_eq!(
            (r[0].code, r[0].modifiers),
            (KeyCode::Right, KeyModifiers::SHIFT)
        );
        let sp = translate(&key(Key::Space, egui::Modifiers::NONE));
        assert_eq!(sp[0].code, KeyCode::Char(' '));
        assert!(
            translate(&Event::Text(" ".into())).is_empty(),
            "space comes via Key only"
        );
        assert!(translate(&key(Key::Plus, egui::Modifiers::COMMAND)).is_empty());
    }

    /// Render the real window contents off-screen to a PNG (only when
    /// CHAINLAB_GUI_SNAPSHOT is set, since it needs a GPU).
    #[test]
    fn render_window_to_png() {
        let Ok(path) = std::env::var("CHAINLAB_GUI_SNAPSHOT") else {
            return;
        };
        let loaded = scenarios::load("balanced").unwrap();
        let mut sim = loaded.build(Some(42)).unwrap();
        sim.run(3000);
        let mut gui = Gui::new(App::start(loaded, sim, None));
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(1100.0, 740.0))
            .wgpu()
            .build_ui(move |ui| {
                let rect = ui.max_rect();
                gui.paint(ui, rect);
            });
        harness.run();
        let img = harness.render().expect("render");
        img.save(&path).expect("save png");
    }

    #[test]
    fn indexed_colours_map_to_xterm() {
        assert_eq!(
            to_color32(Color::Indexed(46), FG),
            Color32::from_rgb(0, 255, 0)
        );
        assert_eq!(
            to_color32(Color::Indexed(28), FG),
            Color32::from_rgb(0, 135, 0)
        );
        assert_eq!(to_color32(Color::Reset, FG), FG);
    }
}
