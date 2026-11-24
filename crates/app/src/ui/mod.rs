//! The terminal front end: a thin layer that draws engine state and turns
//! keys into [`Command`]s. All simulation logic lives in the engine crate.

pub mod flow;
pub mod overlays;
pub mod panels;
pub mod theme;
pub mod widgets;
mod worker;

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::time::{Duration, Instant};

use chainlab_engine::challenges::{ChallengeId, ChallengeRun, Outcome};
use chainlab_engine::config::{SavedParams, ron_options, to_ron};
use chainlab_engine::engine::analysis::{Payback, RunSummary};
use chainlab_engine::engine::model::LogKind;
use chainlab_engine::engine::params::{Group, ParamId};
use chainlab_engine::scenarios::{self, Loaded};
use chainlab_engine::{Command, Sim, TICKS_PER_SEC};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use theme::Theme;
use worker::{Done, Job, Worker};

/// Simulation speed multipliers; the last one means "as fast as possible".
pub const SPEEDS: [f64; 6] = [0.5, 1.0, 2.0, 5.0, 20.0, f64::INFINITY];
pub const COMPARE_MINUTES: [u64; 5] = [1, 2, 5, 10, 30];
pub const SAVE_FILE: &str = "chainlab_params.ron";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overlay {
    None,
    Help(usize),
    Shop(usize),
    Market,
    /// tab 0 = challenges, 1 = scenarios
    Menu {
        tab: usize,
        sel: usize,
    },
    Compare,
}

pub struct CompareUi {
    pub a: (String, BTreeMap<String, f64>),
    pub b: (String, BTreeMap<String, f64>),
    pub minutes: usize,
    pub result: Option<Result<(RunSummary, RunSummary), String>>,
}

pub struct App {
    pub sim: Sim,
    pub loaded: Loaded,
    pub run: ChallengeRun,
    pub paused: bool,
    pub speed: usize,
    debt: f64,
    pub theme: Theme,
    pub overlay: Overlay,
    pub group: usize,
    pub sel: usize,
    pub payback: Vec<Payback>,
    pub payback_tick: u64,
    worker: Worker,
    last_payback: Instant,
    pub compare: CompareUi,
    pub flash: Option<(String, Instant)>,
    pub best: HashMap<ChallengeId, u32>,
    /// Measured simulated ticks per real second.
    pub tps: f64,
    tps_window: (Instant, u64),
    quit: bool,
}

impl App {
    pub fn new(loaded: Loaded, sim: Sim) -> App {
        let seed = sim.seed;
        let defaults = defaults_map(&sim);
        let current = sim.params.to_human_map();
        App {
            run: ChallengeRun::sandbox(seed),
            paused: false,
            speed: 1,
            debt: 0.0,
            theme: Theme { warn_colors: false },
            overlay: Overlay::None,
            group: 0,
            sel: 0,
            payback: vec![],
            payback_tick: 0,
            worker: Worker::spawn(),
            last_payback: Instant::now() - Duration::from_secs(10),
            compare: CompareUi {
                a: ("scenario defaults".into(), defaults),
                b: ("current levers".into(), current),
                minutes: 2,
                result: None,
            },
            flash: None,
            best: HashMap::new(),
            tps: 0.0,
            tps_window: (Instant::now(), 0),
            quit: false,
            sim,
            loaded,
        }
    }

    /// Build the app, optionally jumping straight into a challenge.
    pub fn start(loaded: Loaded, sim: Sim, challenge: Option<ChallengeId>) -> App {
        let mut app = App::new(loaded, sim);
        if let Some(id) = challenge {
            let seed = app.sim.seed;
            app.start_challenge(id, seed);
        }
        app
    }

    /// Has the player asked to quit?
    pub fn quit_requested(&self) -> bool {
        self.quit
    }

    pub fn say(&mut self, msg: impl Into<String>) {
        self.flash = Some((msg.into(), Instant::now()));
    }

    /// Lever groups in panel order.
    pub fn groups(&self) -> Vec<Group> {
        self.sim.params.groups()
    }

    pub fn group_params(&self, g: Group) -> Vec<ParamId> {
        (0..self.sim.params.len())
            .filter(|&i| self.sim.params.list[i].group == g)
            .collect()
    }

    pub fn group_label(&self, g: Group) -> String {
        let c = &*self.sim.chain;
        match g {
            Group::Node(n) => c.nodes[n].name.clone(),
            Group::Link(l) => {
                let link = &c.links[l];
                format!(
                    "Haul {}>{}",
                    short(&c.nodes[link.from].name),
                    short(&c.nodes[link.to].name)
                )
            }
            Group::Market => "Market".into(),
            Group::Costs => "Costs".into(),
            Group::Global => "Global".into(),
            Group::Events => "Events".into(),
        }
    }

    pub fn selected_param(&self) -> Option<ParamId> {
        let groups = self.groups();
        let g = *groups.get(self.group)?;
        self.group_params(g).get(self.sel).copied()
    }

    pub fn show_bottleneck(&self) -> bool {
        !self.run.hide_bottleneck()
    }

    // ----- main loop -----

    pub fn main_loop(&mut self, term: &mut DefaultTerminal) -> io::Result<()> {
        let frame = Duration::from_millis(33);
        let mut last = Instant::now();
        while !self.quit {
            term.draw(|f| panels::draw(f, self))?;
            let deadline = last + frame;
            while let Some(rem) = deadline.checked_duration_since(Instant::now()) {
                if !event::poll(rem)? {
                    break;
                }
                if let Event::Key(k) = event::read()?
                    && k.kind != KeyEventKind::Release
                {
                    self.on_key(k);
                    if self.quit {
                        break;
                    }
                }
            }
            let now = Instant::now();
            let dt = now - last;
            last = now;
            self.advance(dt);
            self.background();
        }
        Ok(())
    }

    /// Advance the simulation by real time `dt` at the current speed.
    pub fn advance(&mut self, dt: Duration) {
        if !self.paused {
            let speed = SPEEDS[self.speed];
            if speed.is_infinite() {
                let start = Instant::now();
                while start.elapsed() < Duration::from_millis(22) {
                    for _ in 0..50 {
                        self.step();
                    }
                }
            } else {
                self.debt += dt.as_secs_f64() * TICKS_PER_SEC as f64 * speed;
                let n = (self.debt.floor() as u64).min(5_000);
                self.debt -= n as f64;
                if self.debt > 50.0 {
                    self.debt = 0.0; // can't keep up; drop the backlog
                }
                for _ in 0..n {
                    self.step();
                }
            }
        }
        let (t0, k0) = self.tps_window;
        let el = t0.elapsed().as_secs_f64();
        if el >= 1.0 {
            self.tps = (self.sim.state.tick - k0) as f64 / el;
            self.tps_window = (Instant::now(), self.sim.state.tick);
        }
    }

    /// One tick, with challenge scripting and scoring around it.
    fn step(&mut self) {
        let was = self.run.is_running();
        self.run.before_step(&mut self.sim);
        self.sim.step();
        self.run.after_step(&self.sim);
        if was && !self.run.is_running() {
            match self.run.outcome.clone() {
                Outcome::Passed { score, detail, .. } => {
                    let best = self.best.entry(self.run.id).or_insert(0);
                    *best = (*best).max(score);
                    self.sim.state.log(
                        LogKind::Challenge,
                        format!("PASSED {} score {score}: {detail}", self.run.id.name()),
                    );
                    self.say(format!(
                        "PASSED {}! score {score}/100 (x to retry)",
                        self.run.id.name()
                    ));
                }
                Outcome::Failed { reason, .. } => {
                    self.sim.state.log(
                        LogKind::Challenge,
                        format!("FAILED {}: {reason}", self.run.id.name()),
                    );
                    let s = self.run.status.clone();
                    self.say(format!("{s} (x to retry)"));
                }
                Outcome::Running => {}
            }
        }
    }

    /// Collect background analysis results and schedule new ones.
    pub fn background(&mut self) {
        for d in self.worker.poll() {
            match d {
                Done::Payback { tick, list } => {
                    self.payback = list;
                    self.payback_tick = tick;
                }
                Done::Compare(r) => self.compare.result = Some(*r),
            }
        }
        let due = self.last_payback.elapsed() > Duration::from_millis(1500);
        let moved = self.sim.state.tick != self.payback_tick || self.payback.is_empty();
        if due && moved && !self.worker.payback_busy && !self.sim.chain.upgrades.is_empty() {
            self.worker.send(Job::Payback(Box::new(self.sim.clone())));
            self.last_payback = Instant::now();
        }
    }

    // ----- actions -----

    fn adjust(&mut self, steps: i64, coarse: bool) {
        let Some(id) = self.selected_param() else {
            return;
        };
        let key = self.sim.params.list[id].key.clone();
        if !self.run.lever_allowed(&key) {
            self.say("That lever is locked during this challenge");
            return;
        }
        self.sim.command(Command::Adjust { id, steps, coarse });
    }

    fn reset_selected(&mut self) {
        let Some(id) = self.selected_param() else {
            return;
        };
        if !self.run.lever_allowed(&self.sim.params.list[id].key) {
            self.say("That lever is locked during this challenge");
            return;
        }
        self.sim.command(Command::Reset(id));
    }

    fn reset_all(&mut self) {
        self.sim.command(Command::ResetAll);
        self.say("All levers reset to scenario defaults");
    }

    fn buy(&mut self, u: usize) {
        match self.sim.buy(u) {
            Ok(_) => {}
            Err(e) => {
                use chainlab_engine::engine::economy::BuyBlock;
                let msg = match e {
                    BuyBlock::Locked(t) => format!("Locked until tier {t}"),
                    BuyBlock::MaxLevel => "Already at max level".into(),
                    BuyBlock::TooExpensive(short) => {
                        format!(
                            "Need {:.1} more coin",
                            chainlab_engine::fixed::to_f64(short)
                        )
                    }
                };
                self.say(msg);
            }
        }
    }

    fn new_sim(&mut self, sim: Sim, run: ChallengeRun) {
        self.sim = sim;
        self.run = run;
        self.payback.clear();
        self.payback_tick = 0;
        self.debt = 0.0;
        self.group = self.group.min(self.groups().len().saturating_sub(1));
        self.sel = 0;
        self.compare.a = ("scenario defaults".into(), defaults_map(&self.sim));
        self.compare.b = ("current levers".into(), self.sim.params.to_human_map());
        self.compare.result = None;
    }

    /// Retry the current challenge, or restart the sandbox keeping levers.
    fn restart(&mut self) {
        if self.run.id == ChallengeId::Sandbox {
            self.sim.restart();
            self.payback.clear();
            self.run = ChallengeRun::sandbox(self.sim.seed);
            self.say("Restarted (levers kept)");
        } else {
            self.start_challenge(self.run.id, self.run.seed());
        }
    }

    fn start_challenge(&mut self, id: ChallengeId, seed: u64) {
        if id == ChallengeId::Sandbox {
            match self.loaded.build(Some(seed)) {
                Ok(sim) => {
                    let run = ChallengeRun::sandbox(seed);
                    self.new_sim(sim, run);
                    self.say(format!("Sandbox: {}", self.loaded.scenario.name));
                }
                Err(e) => self.say(e),
            }
            return;
        }
        match ChallengeRun::start(id, seed) {
            Ok((sim, run)) => {
                if let Ok(l) = scenarios::load("balanced") {
                    self.loaded = l;
                }
                self.new_sim(sim, run);
                self.paused = false;
                self.say(format!("Challenge: {}", id.name()));
            }
            Err(e) => self.say(e),
        }
    }

    fn load_scenario(&mut self, id: &str) {
        match scenarios::load(id) {
            Ok(l) => match l.build(Some(self.sim.seed)) {
                Ok(sim) => {
                    self.loaded = l;
                    let seed = sim.seed;
                    self.new_sim(sim, ChallengeRun::sandbox(seed));
                    self.say(format!("Scenario: {}", self.loaded.scenario.name));
                }
                Err(e) => self.say(e),
            },
            Err(e) => self.say(e),
        }
    }

    fn save(&mut self) {
        let saved = SavedParams {
            scenario: self.loaded.id.clone(),
            seed: self.sim.seed,
            values: self.sim.params.to_human_map(),
        };
        let r =
            to_ron(&saved).and_then(|t| std::fs::write(SAVE_FILE, t).map_err(|e| e.to_string()));
        match r {
            Ok(()) => self.say(format!("Saved levers to {SAVE_FILE}")),
            Err(e) => self.say(format!("Save failed: {e}")),
        }
    }

    fn load(&mut self) {
        let r = std::fs::read_to_string(SAVE_FILE)
            .map_err(|e| e.to_string())
            .and_then(|t| {
                ron_options()
                    .from_str::<SavedParams>(&t)
                    .map_err(|e| e.to_string())
            });
        match r {
            Ok(saved) => {
                if saved.scenario != self.loaded.id || self.run.id != ChallengeId::Sandbox {
                    self.load_scenario(&saved.scenario.clone());
                }
                let unknown = self.sim.apply_values(&saved.values, false);
                self.say(if unknown.is_empty() {
                    format!("Loaded levers from {SAVE_FILE}")
                } else {
                    format!("Loaded; ignored {} unknown keys", unknown.len())
                });
            }
            Err(e) => self.say(format!("Load failed: {e}")),
        }
    }

    fn run_compare(&mut self) {
        if self.worker.compare_busy {
            return;
        }
        self.compare.result = None;
        let ticks = COMPARE_MINUTES[self.compare.minutes] * 60 * TICKS_PER_SEC;
        self.worker.send(Job::Compare {
            chain: Box::new(self.loaded.chain.clone()),
            a: self.compare.a.clone(),
            b: self.compare.b.clone(),
            seed: self.sim.seed,
            ticks,
        });
    }

    pub fn compare_busy(&self) -> bool {
        self.worker.compare_busy
    }

    // ----- input -----

    /// Handle one key press (shared by the terminal and window front ends).
    pub fn on_key(&mut self, k: KeyEvent) {
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        match self.overlay {
            Overlay::None => self.key_main(k),
            Overlay::Help(p) => self.key_help(k, p),
            Overlay::Shop(sel) => self.key_shop(k, sel),
            Overlay::Market => self.key_simple(k),
            Overlay::Menu { tab, sel } => self.key_menu(k, tab, sel),
            Overlay::Compare => self.key_compare(k),
        }
    }

    /// Keys that work everywhere.
    fn key_global(&mut self, k: KeyEvent) -> bool {
        match k.code {
            KeyCode::Char(' ') => self.paused = !self.paused,
            KeyCode::Char('.') => {
                self.paused = true;
                self.step();
            }
            KeyCode::Char('>') => {
                self.paused = true;
                for _ in 0..TICKS_PER_SEC {
                    self.step();
                }
            }
            KeyCode::Char('-') => self.speed = self.speed.saturating_sub(1),
            KeyCode::Char('=') | KeyCode::Char('+') => {
                self.speed = (self.speed + 1).min(SPEEDS.len() - 1)
            }
            KeyCode::Char(c @ '1'..='6') if self.overlay == Overlay::None => {
                self.speed = c as usize - '1' as usize;
            }
            KeyCode::Char('t') => self.theme.warn_colors = !self.theme.warn_colors,
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') | KeyCode::F(1) => self.overlay = Overlay::Help(0),
            _ => return false,
        }
        true
    }

    fn key_main(&mut self, k: KeyEvent) {
        if self.key_global(k) {
            return;
        }
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        let n_groups = self.groups().len().max(1);
        let n_items = self
            .groups()
            .get(self.group)
            .map_or(0, |&g| self.group_params(g).len());
        match k.code {
            KeyCode::Up => self.sel = self.sel.saturating_sub(1),
            KeyCode::Down => self.sel = (self.sel + 1).min(n_items.saturating_sub(1)),
            KeyCode::Tab => {
                self.group = (self.group + 1) % n_groups;
                self.sel = 0;
            }
            KeyCode::BackTab => {
                self.group = (self.group + n_groups - 1) % n_groups;
                self.sel = 0;
            }
            KeyCode::Left => self.adjust(-1, shift),
            KeyCode::Right => self.adjust(1, shift),
            KeyCode::Char('[') => self.adjust(-1, false),
            KeyCode::Char(']') => self.adjust(1, false),
            KeyCode::Char('{') => self.adjust(-1, true),
            KeyCode::Char('}') => self.adjust(1, true),
            KeyCode::Char('r') => self.reset_selected(),
            KeyCode::Char('R') => self.reset_all(),
            KeyCode::Char('s') | KeyCode::Char('b') => self.overlay = Overlay::Shop(0),
            KeyCode::Char('m') => self.overlay = Overlay::Market,
            KeyCode::Char('c') | KeyCode::Char('g') => {
                self.overlay = Overlay::Menu { tab: 0, sel: 0 }
            }
            KeyCode::Char('a') => self.overlay = Overlay::Compare,
            KeyCode::Char('x') => self.restart(),
            KeyCode::Char('w') => self.save(),
            KeyCode::Char('o') => self.load(),
            KeyCode::Char('k') => {
                let s = self.sim.seed.wrapping_add(1);
                self.sim.command(Command::Reseed(s));
                self.say(format!("Seed {s} (random streams reseeded from now)"));
            }
            KeyCode::Char('e') => {
                if self.run.lever_allowed("events.enabled") {
                    let id = self.sim.chain.g.ev_enabled;
                    let v = 1 - self.sim.params.get(id);
                    self.sim.command(Command::Set(id, v));
                    self.say(if v == 1 {
                        "Random events ON"
                    } else {
                        "Random events OFF"
                    });
                } else {
                    self.say("Events are locked during this challenge");
                }
            }
            KeyCode::Char('n') => {
                // Jump to the bottleneck's lever group.
                if self.show_bottleneck()
                    && let Some(b) = chainlab_engine::engine::scoring::bottleneck(
                        &self.sim.chain,
                        self.sim.eff_all(),
                        &self.sim.state,
                    )
                {
                    use chainlab_engine::engine::scoring::Limiter;
                    let g = match b.at {
                        Limiter::Node(n) => Group::Node(n),
                        Limiter::Link(l) => Group::Link(l),
                    };
                    if let Some(i) = self.groups().iter().position(|&x| x == g) {
                        self.group = i;
                        self.sel = 0;
                    }
                }
            }
            _ => {}
        }
    }

    fn key_simple(&mut self, k: KeyEvent) {
        if matches!(k.code, KeyCode::Esc | KeyCode::Enter) || matches!(k.code, KeyCode::Char('m')) {
            self.overlay = Overlay::None;
            return;
        }
        self.key_global(k);
    }

    fn key_help(&mut self, k: KeyEvent, p: usize) {
        let pages = overlays::HELP.len();
        match k.code {
            KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => self.overlay = Overlay::None,
            KeyCode::Right | KeyCode::Down | KeyCode::PageDown | KeyCode::Tab => {
                self.overlay = Overlay::Help((p + 1) % pages)
            }
            KeyCode::Left | KeyCode::Up | KeyCode::PageUp | KeyCode::BackTab => {
                self.overlay = Overlay::Help((p + pages - 1) % pages)
            }
            KeyCode::Char(c @ '1'..='9') => {
                let i = c as usize - '1' as usize;
                if i < pages {
                    self.overlay = Overlay::Help(i);
                }
            }
            KeyCode::Char('0') if pages >= 10 => self.overlay = Overlay::Help(9),
            _ => {
                self.key_global(k);
            }
        }
    }

    fn key_shop(&mut self, k: KeyEvent, sel: usize) {
        let n = self.sim.chain.upgrades.len();
        match k.code {
            KeyCode::Esc | KeyCode::Char('s') => self.overlay = Overlay::None,
            KeyCode::Up => self.overlay = Overlay::Shop(sel.saturating_sub(1)),
            KeyCode::Down => self.overlay = Overlay::Shop((sel + 1).min(n.saturating_sub(1))),
            KeyCode::Enter | KeyCode::Char('b') => {
                if sel < n {
                    self.buy(sel);
                }
            }
            _ => {
                self.key_global(k);
            }
        }
    }

    fn key_menu(&mut self, k: KeyEvent, tab: usize, sel: usize) {
        let n = if tab == 0 {
            ChallengeId::ALL.len()
        } else {
            scenarios::list().len()
        };
        match k.code {
            KeyCode::Esc | KeyCode::Char('c') => self.overlay = Overlay::None,
            KeyCode::Tab | KeyCode::Left | KeyCode::Right | KeyCode::BackTab => {
                self.overlay = Overlay::Menu {
                    tab: 1 - tab,
                    sel: 0,
                }
            }
            KeyCode::Up => {
                self.overlay = Overlay::Menu {
                    tab,
                    sel: sel.saturating_sub(1),
                }
            }
            KeyCode::Down => {
                self.overlay = Overlay::Menu {
                    tab,
                    sel: (sel + 1).min(n.saturating_sub(1)),
                }
            }
            KeyCode::Enter => {
                self.overlay = Overlay::None;
                if tab == 0 {
                    let id = ChallengeId::ALL[sel];
                    let seed = self.sim.seed;
                    self.start_challenge(id, seed);
                } else if let Some((id, _, _)) = scenarios::list().get(sel) {
                    let id = id.clone();
                    self.load_scenario(&id);
                }
            }
            _ => {
                self.key_global(k);
            }
        }
    }

    fn key_compare(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc | KeyCode::Char('a') => self.overlay = Overlay::None,
            KeyCode::Char('A') => {
                self.compare.a = (
                    format!("levers @{}", widgets::clock(self.sim.time_secs())),
                    self.sim.params.to_human_map(),
                );
                self.say("Current levers stored in slot A");
            }
            KeyCode::Char('B') => {
                self.compare.b = (
                    format!("levers @{}", widgets::clock(self.sim.time_secs())),
                    self.sim.params.to_human_map(),
                );
                self.say("Current levers stored in slot B");
            }
            KeyCode::Char('d') => {
                self.compare.a = ("scenario defaults".into(), defaults_map(&self.sim));
            }
            KeyCode::Char('f') => {
                let r = std::fs::read_to_string(SAVE_FILE)
                    .map_err(|e| e.to_string())
                    .and_then(|t| {
                        ron_options()
                            .from_str::<SavedParams>(&t)
                            .map_err(|e| e.to_string())
                    });
                match r {
                    Ok(s) => {
                        self.compare.b = (format!("file {SAVE_FILE}"), s.values);
                    }
                    Err(e) => self.say(format!("Load failed: {e}")),
                }
            }
            KeyCode::Left | KeyCode::Char('[') => {
                self.compare.minutes = self.compare.minutes.saturating_sub(1)
            }
            KeyCode::Right | KeyCode::Char(']') => {
                self.compare.minutes = (self.compare.minutes + 1).min(COMPARE_MINUTES.len() - 1)
            }
            KeyCode::Enter => self.run_compare(),
            _ => {
                self.key_global(k);
            }
        }
    }
}

fn short(name: &str) -> String {
    name.split_whitespace()
        .next()
        .unwrap_or(name)
        .chars()
        .take(7)
        .collect()
}

/// Lever defaults as a human-unit map (slot A of the compare view).
fn defaults_map(sim: &Sim) -> BTreeMap<String, f64> {
    sim.params
        .list
        .iter()
        .map(|p| (p.key.clone(), p.unit.to_human(p.default)))
        .collect()
}

/// Run the TUI.
pub fn run(loaded: Loaded, sim: Sim, challenge: Option<ChallengeId>) -> io::Result<()> {
    let mut terminal = ratatui::init();
    let mut app = App::start(loaded, sim, challenge);
    let r = app.main_loop(&mut terminal);
    ratatui::restore();
    r
}

/// Render one frame to plain text (for checking layouts without a TTY).
/// `overlay`: none | help | shop | market | menu | compare.
pub fn snapshot(
    loaded: Loaded,
    sim: Sim,
    w: u16,
    h: u16,
    overlay: &str,
    challenge: Option<ChallengeId>,
) -> String {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let mut app = App::new(loaded, sim);
    if let Some(id) = challenge {
        let seed = app.sim.seed;
        let ticks = app.sim.state.tick;
        app.start_challenge(id, seed);
        for _ in 0..ticks {
            app.step();
        }
    }
    app.payback = chainlab_engine::engine::analysis::payback(
        &app.sim,
        chainlab_engine::engine::analysis::HORIZON,
    );
    app.payback_tick = app.sim.state.tick;
    app.overlay = match overlay {
        "help" => Overlay::Help(1),
        "shop" => Overlay::Shop(1),
        "market" => Overlay::Market,
        "menu" => Overlay::Menu { tab: 0, sel: 1 },
        "compare" => Overlay::Compare,
        _ => Overlay::None,
    };
    let mut term = Terminal::new(TestBackend::new(w, h)).expect("test backend");
    let _ = term.draw(|f| panels::draw(f, &app));
    let buf = term.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..h {
        for x in 0..w {
            out.push_str(buf.cell((x, y)).map_or(" ", |c| c.symbol()));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: u16, h: u16, overlay: &str, ticks: u64) -> String {
        let loaded = scenarios::load("balanced").unwrap();
        let mut sim = loaded.build(Some(42)).unwrap();
        sim.run(ticks);
        snapshot(loaded, sim, w, h, overlay, None)
    }

    #[test]
    fn every_screen_renders_at_both_sizes() {
        for (w, h) in [(100, 35), (80, 24), (160, 50)] {
            for o in ["none", "help", "shop", "market", "menu", "compare"] {
                let s = frame(w, h, o, 1200);
                assert!(s.contains("CHAINLAB"), "{w}x{h} {o}");
                assert_eq!(s.lines().count(), h as usize);
            }
        }
    }

    #[test]
    fn main_screen_shows_the_mechanics() {
        let s = frame(100, 35, "none", 2400);
        for needle in [
            "FLOW",
            "STATS",
            "LEVERS",
            "LOG",
            "Kiln BN",
            "BOTTLENECK",
            "[WORK]",
            "[LOCKED t2]",
            "COIN",
        ] {
            assert!(s.contains(needle), "missing {needle}\n{s}");
        }
    }

    #[test]
    fn too_small_terminal_says_so() {
        let s = frame(60, 20, "none", 10);
        assert!(s.contains("needs at least 80x24"));
    }

    #[test]
    fn challenge_hides_bottleneck_marker() {
        let loaded = scenarios::load("balanced").unwrap();
        let mut sim = loaded.build(Some(1)).unwrap();
        sim.state.tick = 1500;
        let s = snapshot(
            loaded,
            sim,
            100,
            35,
            "none",
            Some(ChallengeId::FindBottleneck),
        );
        assert!(s.contains("find it yourself"));
        assert!(!s.contains(" BN"));
    }

    #[test]
    fn saved_params_roundtrip() {
        let loaded = scenarios::load("balanced").unwrap();
        let mut sim = loaded.build(Some(9)).unwrap();
        let id = sim.params.id("node.kiln.cycle").unwrap();
        sim.command(Command::Set(id, 25));
        let saved = SavedParams {
            scenario: "balanced".into(),
            seed: 9,
            values: sim.params.to_human_map(),
        };
        let text = to_ron(&saved).unwrap();
        let back: SavedParams = ron_options().from_str(&text).unwrap();
        let mut fresh = loaded.build(Some(9)).unwrap();
        assert!(fresh.apply_values(&back.values, false).is_empty());
        assert_eq!(fresh.params.values(), sim.params.values());
    }
}
