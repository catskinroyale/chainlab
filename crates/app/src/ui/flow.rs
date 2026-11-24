//! The live flow diagram.
//!
//! Nodes are laid out in columns by depth (sources on the left, the Tower
//! and Market on the right). Links are routed through the gaps between
//! columns, with each bent link getting its own vertical "lane". Lines are
//! drawn on a small canvas that merges crossings into proper box-drawing
//! junctions. Carriers ride along each path: `o` loaded and heading
//! downstream, `·` empty and returning. A link whose cart is waiting to
//! unload shows `O` next to the arrowhead.

use chainlab_engine::Sim;
use chainlab_engine::engine::chain::{Kind, StockSpec};
use chainlab_engine::engine::model::Status;
use chainlab_engine::engine::scoring::Limiter;
use chainlab_engine::fixed::{Fx, to_f64};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use super::theme::Theme;
use super::widgets::{fit, gauge, num, rfit};

const N: u8 = 1;
const E: u8 = 2;
const S: u8 = 4;
const W: u8 = 8;

struct Canvas {
    w: i32,
    h: i32,
    bits: Vec<u8>,
    over: Vec<Option<char>>,
    style: Vec<Style>,
}

impl Canvas {
    fn new(w: u16, h: u16, base: Style) -> Canvas {
        let n = w as usize * h as usize;
        Canvas {
            w: w as i32,
            h: h as i32,
            bits: vec![0; n],
            over: vec![None; n],
            style: vec![base; n],
        }
    }
    fn idx(&self, x: i32, y: i32) -> Option<usize> {
        (x >= 0 && y >= 0 && x < self.w && y < self.h).then(|| (y * self.w + x) as usize)
    }
    fn bits(&mut self, x: i32, y: i32, b: u8, st: Style) {
        if let Some(i) = self.idx(x, y) {
            self.bits[i] |= b;
            self.style[i] = st;
        }
    }
    fn hline(&mut self, x0: i32, x1: i32, y: i32, st: Style) {
        let (a, b) = (x0.min(x1), x0.max(x1));
        for x in a..=b {
            let mut m = 0;
            if x > a {
                m |= W;
            }
            if x < b {
                m |= E;
            }
            self.bits(x, y, m, st);
        }
    }
    fn vline(&mut self, x: i32, y0: i32, y1: i32, st: Style) {
        let (a, b) = (y0.min(y1), y0.max(y1));
        for y in a..=b {
            let mut m = 0;
            if y > a {
                m |= N;
            }
            if y < b {
                m |= S;
            }
            self.bits(x, y, m, st);
        }
    }
    fn put(&mut self, x: i32, y: i32, c: char, st: Style) {
        if let Some(i) = self.idx(x, y) {
            self.over[i] = Some(c);
            self.style[i] = st;
        }
    }
    fn text(&mut self, x: i32, y: i32, s: &str, st: Style) {
        for (k, c) in s.chars().enumerate() {
            self.put(x + k as i32, y, c, st);
        }
    }
    fn blit(&self, buf: &mut Buffer, area: Rect) {
        for y in 0..self.h {
            for x in 0..self.w {
                let i = (y * self.w + x) as usize;
                let c = self.over[i].unwrap_or_else(|| line_char(self.bits[i]));
                if c == ' ' {
                    continue;
                }
                let (bx, by) = (area.x + x as u16, area.y + y as u16);
                if bx < area.right() && by < area.bottom() {
                    buf.set_string(bx, by, c.to_string(), self.style[i]);
                }
            }
        }
    }
}

fn line_char(b: u8) -> char {
    match b {
        0 => ' ',
        x if x == E | W || x == E || x == W => '─',
        x if x == N | S || x == N || x == S => '│',
        x if x == E | S => '┌',
        x if x == W | S => '┐',
        x if x == N | E => '└',
        x if x == N | W => '┘',
        x if x == N | E | S => '├',
        x if x == N | W | S => '┤',
        x if x == E | W | S => '┬',
        x if x == N | E | W => '┴',
        _ => '┼',
    }
}

#[derive(Clone, Copy, Debug)]
struct Pos {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

/// Compute box positions. Returns positions and the box height used.
fn layout(sim: &Sim, w: u16, h: u16) -> (Vec<Pos>, i32) {
    let c = &*sim.chain;
    let cols = c.depth.iter().copied().max().unwrap_or(0) + 1;
    let mut col_nodes: Vec<Vec<usize>> = vec![vec![]; cols];
    for (i, &d) in c.depth.iter().enumerate() {
        col_nodes[d].push(i);
    }
    let rows = col_nodes.iter().map(|v| v.len()).max().unwrap_or(1).max(1) as i32;
    let (w, h) = (w as i32, h as i32);
    let (box_h, vgap) = if rows * 5 + (rows - 1) <= h {
        (5, 1)
    } else if rows * 5 <= h {
        (5, 0)
    } else if rows * 4 <= h {
        (4, 0)
    } else {
        (3, 0)
    };
    let cols_i = cols as i32;
    let min_gap = 6;
    let mut box_w = if cols_i > 1 {
        ((w - (cols_i - 1) * min_gap) / cols_i).clamp(10, 20)
    } else {
        w.min(20)
    };
    if cols_i * box_w + (cols_i - 1) * 4 > w {
        box_w = ((w - (cols_i - 1) * 4) / cols_i).max(8);
    }
    let gap = if cols_i > 1 {
        ((w - cols_i * box_w) / (cols_i - 1)).max(2)
    } else {
        0
    };
    let block_h = rows * box_h + (rows - 1) * vgap;
    let top = ((h - block_h) / 2).max(0);
    let mut pos = vec![
        Pos {
            x: 0,
            y: 0,
            w: box_w,
            h: box_h
        };
        c.nodes.len()
    ];
    for (col, nodes) in col_nodes.iter().enumerate() {
        for (row, &n) in nodes.iter().enumerate() {
            pos[n] = Pos {
                x: col as i32 * (box_w + gap),
                y: top + row as i32 * (box_h + vgap),
                w: box_w,
                h: box_h,
            };
        }
    }
    (pos, box_h)
}

/// Draw the diagram into `area`.
pub fn draw(buf: &mut Buffer, area: Rect, sim: &Sim, theme: &Theme, show_bottleneck: bool) {
    if area.width < 20 || area.height < 3 {
        return;
    }
    let c = &*sim.chain;
    let st = &sim.state;
    let (pos, box_h) = layout(sim, area.width, area.height);
    let mut cv = Canvas::new(area.width, area.height, theme.dim());
    let interior = (box_h - 2).max(1);

    let bottleneck = if show_bottleneck {
        chainlab_engine::engine::scoring::bottleneck(c, sim.eff_all(), st).map(|b| b.at)
    } else {
        None
    };

    // Port rows. Straight links use the middle interior row; links heading
    // down leave and enter on the bottom row, links heading up on the top
    // row. Two links that swap rows then never share a horizontal segment.
    let port = |li: usize| -> (i32, i32) {
        let l = &c.links[li];
        let (a, b) = (pos[l.from], pos[l.to]);
        let off = if a.y == b.y {
            1 + (interior - 1) / 2
        } else if b.y > a.y {
            interior
        } else {
            1
        };
        (a.y + off, b.y + off)
    };

    // Route links.
    struct Route {
        path: Vec<(i32, i32)>,
        arrow: (i32, i32),
    }
    let mut routes: Vec<Route> = vec![];
    // Lane counters per gap (keyed by gap start x).
    let mut lane_users: Vec<(i32, Vec<usize>)> = vec![];
    let ends = |li: usize| {
        let l = &c.links[li];
        let (a, b) = (pos[l.from], pos[l.to]);
        let (sy, dy) = port(li);
        (a.x + a.w, sy, b.x - 1, dy)
    };
    for li in 0..c.links.len() {
        let (sx, sy, dx, dy) = ends(li);
        if sy != dy && dx - sx >= 3 {
            match lane_users.iter_mut().find(|(x, _)| *x == sx) {
                Some((_, v)) => v.push(li),
                None => lane_users.push((sx, vec![li])),
            }
        }
    }
    // Pick the lane order that minimises segments running on top of each
    // other (crossings are fine, overlaps are ambiguous).
    for (_, users) in lane_users.iter_mut() {
        if users.len() < 2 || users.len() > 5 {
            continue;
        }
        let mut best = (usize::MAX, users.clone());
        for perm in permutations(users) {
            let n = perm.len() as i32;
            let mut h: Vec<(i32, i32, usize)> = vec![];
            let mut v: Vec<(i32, i32, usize)> = vec![];
            for (k, &li) in perm.iter().enumerate() {
                let (sx, sy, dx, dy) = ends(li);
                let span = (dx - 2 - (sx + 1)).max(0);
                let lane = sx + 1 + (k as i32 + 1) * span / (n + 1);
                h.extend((sx..=lane).map(|x| (x, sy, li)));
                h.extend((lane..dx).map(|x| (x, dy, li)));
                v.extend((sy.min(dy)..=sy.max(dy)).map(|y| (lane, y, li)));
            }
            let overlaps = |cells: &Vec<(i32, i32, usize)>| {
                let mut n = 0;
                for (i, a) in cells.iter().enumerate() {
                    n += cells[i + 1..]
                        .iter()
                        .filter(|b| b.0 == a.0 && b.1 == a.1 && b.2 != a.2)
                        .count();
                }
                n
            };
            let cost = overlaps(&h) * 10 + overlaps(&v) * 10;
            if cost < best.0 {
                best = (cost, perm);
            }
        }
        *users = best.1;
    }
    for (li, l) in c.links.iter().enumerate() {
        let (a, b) = (pos[l.from], pos[l.to]);
        let sx = a.x + a.w;
        let dx = b.x - 1;
        let (sy, dy) = port(li);
        let hot = bottleneck == Some(Limiter::Link(li));
        let style = if hot { theme.emph() } else { theme.dim() };
        let mut path = vec![];
        if sy == dy || dx - sx < 3 {
            cv.hline(sx, dx - 1, sy, style);
            cv.bits(sx, sy, W, style);
            path.extend((sx..dx).map(|x| (x, sy)));
        } else {
            let users = lane_users
                .iter()
                .find(|(x, _)| *x == sx)
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            let k = users.iter().position(|&u| u == li).unwrap_or(0) as i32;
            let n = users.len().max(1) as i32;
            let span = (dx - 2 - (sx + 1)).max(0);
            let lane = sx + 1 + (k + 1) * span / (n + 1);
            cv.hline(sx, lane, sy, style);
            cv.bits(sx, sy, W, style);
            cv.vline(lane, sy, dy, style);
            cv.hline(lane, dx - 1, dy, style);
            path.extend((sx..lane).map(|x| (x, sy)));
            let step = if dy > sy { 1 } else { -1 };
            let mut y = sy;
            while y != dy {
                path.push((lane, y));
                y += step;
            }
            path.extend((lane..dx).map(|x| (x, dy)));
        }
        routes.push(Route {
            path,
            arrow: (dx, dy),
        });
    }

    // Carriers.
    let now = st.tick;
    for (li, l) in c.links.iter().enumerate() {
        let r = &routes[li];
        let len = r.path.len();
        if len == 0 {
            continue;
        }
        let travel = sim.eff(l.p.distance).max(1) as f64;
        let mut waiting = false;
        for s in &st.links[li].shipments {
            let (frac, glyph, style) = match s.return_at {
                None => {
                    let f =
                        (now.saturating_sub(s.depart)) as f64 / (s.arrive - s.depart).max(1) as f64;
                    if f >= 1.0 {
                        waiting = true;
                    }
                    (f.min(1.0), 'o', theme.bright())
                }
                Some(ret) => {
                    let f = 1.0 - (ret.saturating_sub(now)) as f64 / travel;
                    (1.0 - f.clamp(0.0, 1.0), '·', theme.base())
                }
            };
            let i = ((frac * (len - 1) as f64).round() as usize).min(len - 1);
            let (x, y) = r.path[i];
            cv.put(x, y, glyph, style);
        }
        let (ax, ay) = r.arrow;
        let hot = bottleneck == Some(Limiter::Link(li));
        cv.put(ax, ay, '>', if hot { theme.emph() } else { theme.base() });
        if waiting && len >= 2 {
            let (x, y) = r.path[len - 1];
            cv.put(x, y, 'O', theme.warn());
        }
    }

    // Boxes on top.
    for (n, p) in pos.iter().enumerate() {
        draw_box(
            &mut cv,
            sim,
            theme,
            n,
            *p,
            bottleneck == Some(Limiter::Node(n)),
        );
    }
    cv.blit(buf, area);
}

fn draw_box(cv: &mut Canvas, sim: &Sim, theme: &Theme, n: usize, p: Pos, hot: bool) {
    let c = &*sim.chain;
    let ns = &sim.state.nodes[n];
    let locked = sim.is_locked(n);
    let broken = sim.is_broken(n);
    let (tl, tr, bl, br, hz, vt) = if hot {
        ('╔', '╗', '╚', '╝', '═', '║')
    } else {
        ('┌', '┐', '└', '┘', '─', '│')
    };
    let border = if hot {
        theme.emph()
    } else if locked || ns.status == Status::Off {
        theme.dim()
    } else {
        theme.status(ns.status)
    };
    let iw = (p.w - 2).max(1) as usize;
    // Clear interior and draw border.
    for y in p.y..p.y + p.h {
        for x in p.x..p.x + p.w {
            cv.put(x, y, ' ', theme.base());
        }
    }
    let mut title = c.nodes[n].name.clone();
    if hot {
        title = format!("{title} BN");
    }
    let title = fit(&title, iw.min(title.chars().count()));
    let top: String = format!(
        "{tl}{title}{}{tr}",
        hz.to_string().repeat(iw - title.chars().count())
    );
    cv.text(p.x, p.y, &top, border);
    for y in p.y + 1..p.y + p.h - 1 {
        cv.put(p.x, y, vt, border);
        cv.put(p.x + p.w - 1, y, vt, border);
    }
    cv.text(
        p.x,
        p.y + p.h - 1,
        &format!("{bl}{}{br}", hz.to_string().repeat(iw)),
        border,
    );

    let lines = box_lines(sim, theme, n, iw, (p.h - 2) as usize, locked, broken);
    for (row, segs) in lines.into_iter().enumerate() {
        let mut x = p.x + 1;
        let end = p.x + 1 + iw as i32;
        for (text, st) in segs {
            let room = (end - x).max(0) as usize;
            let t: String = text.chars().take(room).collect();
            cv.text(x, p.y + 1 + row as i32, &t, st);
            x += t.chars().count() as i32;
        }
    }
}

type Segs = Vec<(String, Style)>;

/// "lbl████░░ 12" in exactly `w` cells.
fn gauge_line(theme: &Theme, label: &str, v: Fx, cap: Fx, w: usize) -> Segs {
    let val = rfit(&num(v), 4);
    let lw = label.chars().count();
    let gw = w.saturating_sub(lw + 4).max(1);
    let full = cap > 0 && v >= cap;
    let empty = v <= 0;
    let gstyle = if full {
        theme.alert()
    } else if empty {
        theme.warn()
    } else {
        theme.base()
    };
    vec![
        (label.to_string(), theme.dim()),
        (gauge(v, cap, gw), gstyle),
        (val, theme.base()),
    ]
}

fn box_lines(
    sim: &Sim,
    theme: &Theme,
    n: usize,
    iw: usize,
    rows: usize,
    locked: bool,
    broken: bool,
) -> Vec<Segs> {
    let c = &*sim.chain;
    let node = &c.nodes[n];
    let ns = &sim.state.nodes[n];
    let mut out: Vec<Segs> = vec![];

    // Line 1: status tag, crew, cycle progress.
    let (tag, tstyle) = if locked {
        (format!("[LOCKED t{}]", node.tier), theme.dim())
    } else if broken {
        ("[BROKEN]".to_string(), theme.alert())
    } else if matches!(node.kind, Kind::Market) && ns.status == Status::Blocked {
        ("[HOLDING]".to_string(), theme.alert())
    } else {
        (ns.status.tag().to_string(), theme.status(ns.status))
    };
    let crew = node.p.workers.map(|_| {
        let units = node.p.units.map_or(1, |p| sim.eff(p));
        let opt = node.optimal_workers * units;
        format!("{}/{}w", sim.workers(node), opt)
    });
    let crew = crew.unwrap_or_default();
    let mut l1: Segs = vec![(tag.clone(), tstyle)];
    let used = tag.chars().count() + crew.chars().count();
    if used < iw {
        let room = iw - used;
        let mid = match (&node.kind, node.p.cycle) {
            (Kind::Processor(_), Some(cy)) if room >= 4 && ns.in_cycle => {
                let total = sim.eff(cy).max(1) * 1000;
                let g = gauge(ns.progress, total, room - 2);
                format!(" {g} ")
            }
            _ => " ".repeat(room),
        };
        l1.push((mid, theme.dim()));
    }
    l1.push((crew, theme.dim()));
    out.push(l1);

    let in_cap = node.p.in_cap.map_or(0, |p| sim.eff(p));
    let out_cap = node.p.out_cap.map_or(0, |p| sim.eff(p));
    let mut detail: Vec<Segs> = vec![];
    match &node.kind {
        Kind::Source(spec) => {
            detail.push(gauge_line(theme, "out", ns.outputs[0], out_cap, iw));
            match spec.stock {
                StockSpec::Regrowing { max, .. } => {
                    detail.push(gauge_line(theme, "std", ns.stock, max, iw));
                }
                StockSpec::Depleting { initial } => {
                    detail.push(gauge_line(theme, "dep", ns.stock, initial, iw));
                }
                StockSpec::Infinite => {
                    let s = &sim.state.stats.nodes[n];
                    detail.push(vec![(
                        fit(&format!("{}/min", num(s.per_min())), iw),
                        theme.dim(),
                    )]);
                }
            }
        }
        Kind::Processor(_) => {
            let lowest = ns.inputs.iter().copied().min().unwrap_or(0);
            detail.push(gauge_line(theme, "in ", lowest, in_cap, iw));
            let first = ns.outputs.first().copied().unwrap_or(0);
            detail.push(gauge_line(theme, "out", first, out_cap, iw));
        }
        Kind::Sink(_) => {
            let k = node.in_res.len().max(1);
            let each = iw / k;
            let mut segs: Segs = vec![];
            for (i, &r) in node.in_res.iter().enumerate() {
                let initial: String = c.res_name(r).chars().take(1).collect();
                let gw = each.saturating_sub(2).max(1);
                let v = ns.inputs[i];
                let st = if v <= 0 {
                    theme.warn()
                } else if v >= in_cap {
                    theme.alert()
                } else {
                    theme.base()
                };
                segs.push((initial, theme.dim()));
                segs.push((gauge(v, in_cap, gw), st));
                segs.push((" ".to_string(), theme.dim()));
            }
            let used: usize = segs.iter().map(|s| s.0.chars().count()).sum();
            if used < iw {
                segs.push((" ".repeat(iw - used), theme.dim()));
            }
            detail.push(segs);
            let s = &sim.state.stats;
            detail.push(vec![(
                fit(
                    &format!(
                        "P {} {}/m",
                        num(sim.state.progress),
                        num(s.progress_per_min())
                    ),
                    iw,
                ),
                theme.bright(),
            )]);
        }
        Kind::Market => {
            if let Some(m) = &c.market {
                for (k, g) in m.goods.iter().enumerate() {
                    let gs = &sim.state.market[k];
                    let arrow = super::widgets::trend(sim.state.stats.history.prices[k].iter());
                    let name: String = c.res_name(g.res).chars().take(4).collect();
                    let price = format!("{:.2}", to_f64(gs.price));
                    let stock = num(ns.inputs[k]);
                    let txt = format!("{name} {price}{arrow}");
                    let pad = iw.saturating_sub(txt.chars().count() + stock.chars().count());
                    let st = if gs.price < sim.eff(g.base_price) * 7 / 10 {
                        theme.warn()
                    } else {
                        theme.base()
                    };
                    detail.push(vec![
                        (txt, st),
                        (" ".repeat(pad), theme.dim()),
                        (stock, theme.dim()),
                    ]);
                }
            }
        }
    }
    out.extend(detail);
    out.truncate(rows);
    out
}

/// All orderings of a small list.
fn permutations(v: &[usize]) -> Vec<Vec<usize>> {
    if v.len() <= 1 {
        return vec![v.to_vec()];
    }
    let mut out = vec![];
    for i in 0..v.len() {
        let mut rest = v.to_vec();
        let x = rest.remove(i);
        for mut p in permutations(&rest) {
            p.insert(0, x);
            out.push(p);
        }
    }
    out
}
