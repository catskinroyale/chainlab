//! Overlay screens: help, shop (cost curves + payback), market, challenge
//! and scenario menu, and the A/B compare view.

use chainlab_engine::challenges::{ChallengeId, Outcome};
use chainlab_engine::engine::chain::{Effect, POLICIES};
use chainlab_engine::engine::economy;
use chainlab_engine::engine::market;
use chainlab_engine::fixed::{Fx, fx, to_f64};
use chainlab_engine::scenarios;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};

use super::panels::wrap;
use super::widgets::{SPARK, fit, num, rfit, signed, sparkline};
use super::{App, COMPARE_MINUTES, Overlay};

/// Help pages: (title, body). Each mechanic page names the levers that
/// demonstrate it.
pub const HELP: &[(&str, &str)] = &[
    (
        "CHAINLAB: a mechanics lab",
        r#"A live production chain: gather -> process -> haul -> spend -> expand.
Change any lever while it runs and watch throughput, bottleneck, profit
and utilisation respond.

KEYS
space pause      .  step 1 tick    >  step 1 second
- / = speed      1-6 speed 0.5x 1x 2x 5x 20x max
up/dn lever      <-/-> adjust (shift = coarse)  [ ] { } also adjust
tab   lever group (one per node, link, market, costs, global, events)
r     reset lever     R  reset all levers     n  jump to bottleneck
s     shop: upgrades, cost curves, payback times
m     market detail   c  challenges & scenarios    a  A/B compare
x     retry challenge / restart sandbox    k  new seed
w / o save / load levers (chainlab_params.ron)
e     random events on/off    t  warning colours    q  quit

LEVER PANEL: '*' changed from default, 'L' locked by the challenge,
'effective' shows the value after upgrades/events/worker pool.

Pages: <-/-> or 1-9,0. Each page explains one mechanic."#,
    ),
    (
        "1. Raw sources",
        r#"A source pulls raw material out of the world each tick:

  yield = rate * buildings * crew factor * efficiency * (1 +/- variance)

If its output buffer is full it is BLOCKED: it could work, but nothing
downstream takes its output. Infinite sources never run out; Depleting
ones (Deep Quarry) run dry and go STARVED for good.

The Forest REGROWS logistically:  growth = r * water * S * (1 - S/K)
Growth is fastest at half capacity and slowest when nearly empty, so
over-harvesting is punished twice: less to cut, slower recovery.
Max sustainable harvest is r*K/4 per second.

TRY: Forest harvest rate up past 1.25/s and watch 'std' drain.
     Global > Water supply down (drought); Global > Regrowth rate.
     Scenario 'Starved Forest'."#,
    ),
    (
        "2. Processors (recipes)",
        r#"A processor turns inputs into outputs over a cycle:

  2 Stone --[4.0s]--> 1 Brick   (x batch x buildings x efficiency)

It starts a cycle only if every input has a full batch AND the output
has room. Short input = STARVED. No room = BLOCKED (inputs untouched).
Crew factor sets speed: half crew, half speed.

Capacity/min = output * batch * buildings * eff% * crew * 60/cycle.
The lever panel shows actual vs capacity for the selected node.

TRY: Kiln > cycle time down: watch the bottleneck move to the Tower.
     Kiln > efficiency (waste). Kiln > buildings (+ more workers)."#,
    ),
    (
        "3. Logistics",
        r#"Goods ride carts along links. A cart loads, travels the distance,
unloads, and travels back empty:

  link max/min = carts * load * 60 / (2 * distance)

Doubling distance halves throughput unless you double the carts.
In the diagram: o = loaded cart heading downstream, · = empty cart
returning, O = cart waiting to unload because the receiver is full.
The dispatcher only ships what will fit at the destination, so a full
destination backs up into the sender (backpressure).

TRY: a Haul group: carts 1, distance 10s. The link becomes the
     bottleneck (highlighted line). Scenario 'Expensive Logistics'."#,
    ),
    (
        "4. Storage and caps",
        r#"Every node has input and output buffers with a capacity (tanks in the
diagram: ████░░). Buffers decouple stages: a big buffer absorbs
bursts and breakdowns, but it can't raise long-run throughput.

  full output buffer  -> node BLOCKED (and soon its feeders too)
  empty input buffer  -> node STARVED

Gauges turn bright/red when full and amber when empty (with colours on).

TRY: Kiln > output cap down to 2 and watch blocking ripple upstream.
     Turn events on (e) and compare small vs large buffers through a
     breakdown."#,
    ),
    (
        "5. Workers",
        r#"The worker pool is shared. Each node asks for workers; the pool is
handed out upstream-first, so if you over-assign, downstream nodes get
NOCREW. Every worker in the pool costs upkeep per minute, assigned or
idle.

Diminishing returns: below the optimal crew, speed is linear. Above it,
extra worker k adds d^k of a worker (d = 'over-crew returns'). With d=50%
crews of 2/3/4 on an optimal-2 node run at 100/125/137.5%.

TRY: Global > Worker pool down to 6: watch NOCREW appear downstream.
     Pile 6 workers on the Kiln and compare with buying a building.
     Challenge 'Lean Chain'."#,
    ),
    (
        "6. Market",
        r#"Surplus goes to market for Coin. Each good's price:

  price = base * drift / (1 + elasticity * pressure)

Every unit sold adds pressure; pressure fades by 'recovery' per second.
Dumping lots at once tanks the price; steady selling earns more. Drift
is a random walk (volatility) pulled back towards 1.0.

Sell policy: sell all | above threshold (only while price >= X% of base;
market shows HOLDING otherwise) | keep reserve (never sell the last X).

TRY: market (m) view; elasticity up; policy 'above thresh' in
     scenario 'Volatile Market'. Challenge 'Market Shock'."#,
    ),
    (
        "7. Escalating cost and upgrades",
        r#"Each upgrade costs more the more you own:

  cost(n) = base * growth^n * (1 - discount)

At growth 1.15 the 10th level costs 4x the first. Exponential costs vs
linear gains means every upgrade's payback time keeps rising until a
different upgrade, or a new tier, becomes the better buy.

The shop (s) shows each upgrade's next-12-levels cost curve and its
payback: the sim is forked, the upgrade applied for free, and both
futures run 2 minutes. An upgrade on a non-bottleneck shows ~0 gain.

TRY: Costs > growth up to 2.0 (the cost wall), Costs > bulk discount.
     Challenge 'Cost Wall'."#,
    ),
    (
        "8. Gates, events and time",
        r#"GATES: milestones passed in order raise the tier, unlocking upgrades
and nodes (tier 1 at 30 progress: lift + stall upgrades; tier 2 at 600
profit: Deep Quarry + glazing). New nodes need workers assigned!

EVENTS (e to toggle): drought (water cut), price crash (drift knocked
down, recovers), breakdown (a processor stops), windfall (coin).
Frequency and severity are levers in the Events group.

TIME: fixed 10 ticks per simulated second. Integer fixed-point state,
per-subsystem random streams: the same seed + levers + actions give
the exact same run, and different speeds give identical results."#,
    ),
    (
        "9. Bottleneck, payback and the efficiency score",
        r#"BOTTLENECK: each stage's load = actual/min / capacity/min. Starting at
the Tower, follow its scarcest input upstream; the highest-load stage on
that path is the bottleneck (ties go to the stage nearest the Tower).
It gets a double border; the flow panel footer gives the evidence:
feeders BLOCKED behind it, consumers STARVED after it.

EFFICIENCY SCORE (0-100):
  100 * (0.4 * utilisation + 0.3 * workforce + 0.3 * margin)
utilisation = mean Working share of production nodes
workforce   = share of the pool on nodes that are actually working
margin      = (income - upkeep) / income

A balanced, fully staffed chain with low upkeep scores near 100."#,
    ),
];

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(2));
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

fn frame<'a>(app: &App, title: &'a str, hint: &'a str) -> Block<'a> {
    Block::bordered()
        .border_type(BorderType::Double)
        .border_style(app.theme.bright())
        .title(Span::styled(format!(" {title} "), app.theme.emph()))
        .title_bottom(Span::styled(format!(" {hint} "), app.theme.dim()))
        .style(app.theme.base())
}

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    match app.overlay {
        Overlay::None => {}
        Overlay::Help(p) => help(f, area, app, p),
        Overlay::Shop(sel) => shop(f, area, app, sel),
        Overlay::Market => market_view(f, area, app),
        Overlay::Menu { tab, sel } => menu(f, area, app, tab, sel),
        Overlay::Compare => compare(f, area, app),
    }
}

fn help(f: &mut Frame, area: Rect, app: &App, page: usize) {
    let th = app.theme;
    let r = centered(area, 82, 26);
    let (title, body) = HELP[page.min(HELP.len() - 1)];
    let hint = format!("page {}/{}  <-/-> pages  esc close", page + 1, HELP.len());
    let block = frame(app, title, &hint);
    let inner = block.inner(r);
    f.render_widget(Clear, r);
    f.render_widget(block, r);
    let lines: Vec<Line> = body
        .lines()
        .map(|l| {
            let st = if l.starts_with("TRY")
                || l.starts_with("     ") && l.trim_start().ends_with('.')
            {
                th.bright()
            } else if l.chars().all(|c| c.is_uppercase() || c.is_whitespace())
                && !l.trim().is_empty()
            {
                th.emph()
            } else if l.starts_with("  ") {
                th.bright()
            } else {
                th.base()
            };
            Line::from(Span::styled(l.to_string(), st))
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

fn effect_text(app: &App, u: usize) -> Vec<String> {
    let sim = &app.sim;
    let up = &sim.chain.upgrades[u];
    up.effects
        .iter()
        .map(|(pid, e)| {
            let p = &sim.params.list[*pid];
            let now = sim.eff(*pid);
            let (what, next) = match e {
                Effect::Add(a) => (format!("+{}", p.unit.format(*a)), now + a),
                Effect::Mul(m) => (format!("x{m:.2}"), (now as f64 * m).round() as i64),
            };
            format!(
                "{what} {}  ({} -> {})",
                p.label,
                p.unit.format(now),
                p.unit.format(next.clamp(p.min, p.max))
            )
        })
        .collect()
}

fn shop(f: &mut Frame, area: Rect, app: &App, sel: usize) {
    let th = app.theme;
    let r = centered(area, 98, 32);
    let block = frame(
        app,
        "SHOP: upgrades, cost curves, payback",
        "up/dn select  enter/b buy  esc close",
    );
    let inner = block.inner(r);
    f.render_widget(Clear, r);
    f.render_widget(block, r);
    let sim = &app.sim;
    let c = &*sim.chain;
    let iw = inner.width as usize;
    let mut lines: Vec<Line> = vec![];
    lines.push(Line::from(vec![
        Span::styled(format!("coin {}  ", num(sim.state.coin)), th.emph()),
        Span::styled(
            format!(
                "payback = cost / extra profit, measured by forking the sim for 2 min (as of {})",
                super::widgets::clock(app.payback_tick as f64 / 10.0)
            ),
            th.dim(),
        ),
    ]));
    let nw = iw.saturating_sub(4 + 7 + 9 + 10 + 10 + 10 + 1).max(12);
    lines.push(Line::from(Span::styled(
        format!(
            "  {}{}{}{}{}{}",
            fit("upgrade", nw),
            rfit("lvl", 4),
            rfit("cost", 9),
            rfit("+prog/m", 10),
            rfit("+profit/m", 10),
            rfit("payback", 10)
        ),
        th.dim(),
    )));
    let list_rows = (inner.height as usize).saturating_sub(14).max(4);
    let offset = sel.saturating_sub(list_rows.saturating_sub(1));
    for (u, up) in c.upgrades.iter().enumerate().skip(offset).take(list_rows) {
        let cost = sim.upgrade_cost(u);
        let check = sim.check_buy(u);
        let pb = app.payback.iter().find(|p| p.upgrade == u);
        let locked = up.tier > sim.state.tier;
        let maxed = up.max_level.is_some_and(|m| sim.state.upgrades[u] >= m);
        let (dp, dm, pay) = match pb {
            _ if locked => ("".into(), "".into(), format!("tier {}", up.tier)),
            _ if maxed => ("".into(), "".into(), "max".into()),
            Some(p) => (
                signed(p.d_progress),
                signed(p.d_profit),
                p.payback_secs.map_or("never".into(), |s| {
                    if s > 3600.0 {
                        ">1h".into()
                    } else {
                        format!("{}:{:02}", (s / 60.0) as u64, s as u64 % 60)
                    }
                }),
            ),
            None => ("...".into(), "...".into(), "...".into()),
        };
        let text = format!(
            "{}{}{}{}{}{}{}",
            if u == sel { "> " } else { "  " },
            fit(&up.name, nw),
            rfit(&sim.state.upgrades[u].to_string(), 4),
            rfit(&num(cost), 9),
            rfit(&dp, 10),
            rfit(&dm, 10),
            rfit(&pay, 10)
        );
        let style = if u == sel {
            th.sel()
        } else if locked || maxed {
            th.dim()
        } else if check.is_ok() {
            th.bright()
        } else {
            th.base()
        };
        lines.push(Line::from(Span::styled(fit(&text, iw), style)));
    }
    lines.push(Line::from(""));

    // Selected upgrade: effects and cost curve.
    if let Some(up) = c.upgrades.get(sel) {
        for e in effect_text(app, sel) {
            lines.push(Line::from(Span::styled(
                fit(&format!("effect: {e}"), iw),
                th.base(),
            )));
        }
        let base = sim.eff(up.base_cost);
        let growth = sim.eff(up.growth);
        let disc = sim.eff(c.g.discount);
        lines.push(Line::from(Span::styled(
            format!(
                "cost(n) = {} * {:.2}^n * (1 - {disc}%)   owned {}   next 12 levels:",
                num(base),
                to_f64(growth),
                sim.state.upgrades[sel]
            ),
            th.dim(),
        )));
        let curve = economy::cost_curve(base, growth, sim.state.upgrades[sel], 12, disc);
        lines.extend(cost_chart(app, &curve, sim.state.coin, 7));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

/// Vertical bar chart of upcoming costs; bars you can afford are bright,
/// and a dashed line marks your coin.
fn cost_chart(app: &App, costs: &[Fx], coin: Fx, h: usize) -> Vec<Line<'static>> {
    let th = app.theme;
    let max = costs.iter().copied().max().unwrap_or(1).max(1);
    let coin_row = if coin > 0 && coin < max {
        Some(((coin as f64 / max as f64) * h as f64).floor() as usize)
    } else {
        None
    };
    let mut lines = vec![];
    for row in (0..h).rev() {
        let label = if row == h - 1 {
            rfit(&num(max), 7)
        } else if row == 0 {
            rfit("0", 7)
        } else {
            " ".repeat(7)
        };
        let mut spans = vec![Span::styled(format!("{label} |"), th.dim())];
        for &cst in costs {
            let v = cst as f64 / max as f64 * h as f64;
            let fill = (v - row as f64).clamp(0.0, 1.0);
            let ch = if fill <= 0.0 {
                if coin_row == Some(row) { '-' } else { ' ' }
            } else {
                SPARK[((fill * 8.0).ceil() as usize).clamp(1, 8) - 1]
            };
            let st = if cst <= coin { th.bright() } else { th.dim() };
            spans.push(Span::styled(
                format!("{ch}{ch} "),
                if fill > 0.0 { st } else { th.dim() },
            ));
        }
        if coin_row == Some(row) {
            spans.push(Span::styled(format!(" <- coin {}", num(coin)), th.dim()));
        }
        lines.push(Line::from(spans));
    }
    let labels: String = (1..=costs.len())
        .map(|k| format!("{:<3}", format!("+{k}")))
        .collect();
    lines.push(Line::from(Span::styled(
        format!("        {labels}"),
        th.dim(),
    )));
    lines
}

fn market_view(f: &mut Frame, area: Rect, app: &App) {
    let th = app.theme;
    let r = centered(area, 90, 30);
    let block = frame(app, "MARKET", "esc close");
    let inner = block.inner(r);
    f.render_widget(Clear, r);
    f.render_widget(block, r);
    let sim = &app.sim;
    let c = &*sim.chain;
    let iw = inner.width as usize;
    let mut lines: Vec<Line> = vec![];
    let Some(m) = &c.market else {
        f.render_widget(Paragraph::new("This chain has no market."), inner);
        return;
    };
    let policy = sim.eff(m.policy) as usize;
    lines.push(Line::from(vec![
        Span::styled("policy: ", th.dim()),
        Span::styled(POLICIES.get(policy).copied().unwrap_or("?"), th.emph()),
        Span::styled(
            format!(
                "   threshold {}% of base   reserve {}   max sell {:.2}/s per good",
                sim.eff(m.threshold),
                num(sim.eff(m.reserve)),
                to_f64(sim.eff(m.sell_rate))
            ),
            th.dim(),
        ),
    ]));
    lines.push(Line::from(Span::styled(
        "price = base * drift / (1 + elasticity * pressure); pressure fades by recovery/s",
        th.dim(),
    )));
    for (k, g) in m.goods.iter().enumerate() {
        let gs = &sim.state.market[k];
        let s = &sim.state.stats.goods[k];
        let base = sim.eff(g.base_price);
        let e = sim.eff(g.elasticity);
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(format!("{} ", c.res_name(g.res).to_uppercase()), th.emph()),
            Span::styled(format!("price {:.2} ", to_f64(gs.price)), th.bright()),
            Span::styled(
                format!(
                    "(base {:.2}, drift {:.2}, pressure {:.1})  stock {}",
                    to_f64(base),
                    to_f64(gs.drift),
                    to_f64(gs.pressure),
                    num(sim.state.nodes[m.node].inputs[k])
                ),
                th.dim(),
            ),
        ]));
        lines.push(Line::from(Span::styled(
            format!(
                "sold {}/min  revenue {}/min  total sold {}  total revenue {}",
                num(s.sold.per_window()),
                num(s.revenue.per_window()),
                num(gs.sold_total),
                num(gs.revenue_total)
            ),
            th.base(),
        )));
        lines.push(Line::from(vec![
            Span::styled("history ", th.dim()),
            Span::styled(
                sparkline(
                    sim.state.stats.history.prices[k].iter(),
                    iw.saturating_sub(8),
                ),
                th.bright(),
            ),
        ]));
        // Demand curve: what dumping X more units right now would do.
        let mut spans = vec![Span::styled("sell now +", th.dim())];
        for x in [0.0, 10.0, 25.0, 50.0, 100.0] {
            let p = market::price(base, gs.drift, e, gs.pressure + fx(x));
            spans.push(Span::styled(
                format!(" {x:>3.0}:{:.2}", to_f64(p)),
                th.base(),
            ));
        }
        lines.push(Line::from(spans));
        let bw = iw.saturating_sub(14);
        for x in [0.0, 25.0, 50.0, 100.0] {
            let p = market::price(base, gs.drift, e, gs.pressure + fx(x));
            let n = ((to_f64(p) / to_f64(base.max(1)) / 1.5).clamp(0.0, 1.0) * bw as f64) as usize;
            lines.push(Line::from(vec![
                Span::styled(format!("  +{x:>3.0} units "), th.dim()),
                Span::styled("█".repeat(n), th.base()),
            ]));
        }
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn menu(f: &mut Frame, area: Rect, app: &App, tab: usize, sel: usize) {
    let th = app.theme;
    let r = centered(area, 84, 24);
    let block = frame(
        app,
        "CHALLENGES & SCENARIOS",
        "tab switch  up/dn select  enter start  esc close",
    );
    let inner = block.inner(r);
    f.render_widget(Clear, r);
    f.render_widget(block, r);
    let iw = inner.width as usize;
    let mut lines = vec![Line::from(vec![
        Span::styled(" CHALLENGES ", if tab == 0 { th.sel() } else { th.dim() }),
        Span::raw("  "),
        Span::styled(" SCENARIOS ", if tab == 1 { th.sel() } else { th.dim() }),
    ])];
    lines.push(Line::from(""));
    let detail: String;
    if tab == 0 {
        for (i, id) in ChallengeId::ALL.iter().enumerate() {
            let best = app
                .best
                .get(id)
                .map_or(String::new(), |b| format!("best {b}"));
            let current = if app.run.id == *id {
                match &app.run.outcome {
                    Outcome::Running => "  <running>".to_string(),
                    Outcome::Passed { score, .. } => format!("  <passed {score}>"),
                    Outcome::Failed { .. } => "  <failed>".to_string(),
                }
            } else {
                String::new()
            };
            let text = format!(
                "{}{}{}{}",
                if i == sel { "> " } else { "  " },
                fit(id.name(), 24),
                rfit(&best, 9),
                current
            );
            lines.push(Line::from(Span::styled(
                fit(&text, iw),
                if i == sel { th.sel() } else { th.base() },
            )));
        }
        detail = ChallengeId::ALL[sel.min(ChallengeId::ALL.len() - 1)]
            .goal()
            .to_string();
    } else {
        let list = scenarios::list();
        for (i, (id, name, _)) in list.iter().enumerate() {
            let cur = if *id == app.loaded.id {
                "  <loaded>"
            } else {
                ""
            };
            let text = format!(
                "{}{}{cur}",
                if i == sel { "> " } else { "  " },
                fit(name, 24)
            );
            lines.push(Line::from(Span::styled(
                fit(&text, iw),
                if i == sel { th.sel() } else { th.base() },
            )));
        }
        detail = list.get(sel).map(|x| x.2.clone()).unwrap_or_default();
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Files in data/scenarios/*.ron override these. Loading a scenario starts the sandbox.",
            th.dim(),
        )));
    }
    lines.push(Line::from(""));
    for l in wrap(&detail, iw) {
        lines.push(Line::from(Span::styled(l, th.bright())));
    }
    if tab == 0 {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Challenges lock capability levers: win by allocating workers, switching",
            th.dim(),
        )));
        lines.push(Line::from(Span::styled(
            "nodes, sizing buffers, sell policy and buying upgrades. x = instant retry.",
            th.dim(),
        )));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn compare(f: &mut Frame, area: Rect, app: &App) {
    let th = app.theme;
    let r = centered(area, 80, 24);
    let block = frame(
        app,
        "A/B COMPARE (headless fast-forward)",
        "A/B store levers  d A=defaults  f B=file  <-/-> minutes  enter run  esc",
    );
    let inner = block.inner(r);
    f.render_widget(Clear, r);
    f.render_widget(block, r);
    let mins = COMPARE_MINUTES[app.compare.minutes];
    let mut lines = vec![
        Line::from(vec![
            Span::styled("A: ", th.emph()),
            Span::styled(app.compare.a.0.clone(), th.base()),
        ]),
        Line::from(vec![
            Span::styled("B: ", th.emph()),
            Span::styled(app.compare.b.0.clone(), th.base()),
        ]),
        Line::from(vec![
            Span::styled("run both from tick 0 for ", th.dim()),
            Span::styled(format!("{mins} simulated min"), th.bright()),
            Span::styled(
                format!(" with seed {} (upgrades not included)", app.sim.seed),
                th.dim(),
            ),
        ]),
        Line::from(""),
    ];
    let changed = |m: &std::collections::BTreeMap<String, f64>,
                   o: &std::collections::BTreeMap<String, f64>| {
        m.iter()
            .filter(|(k, v)| o.get(*k).is_some_and(|x| (x - **v).abs() > 1e-9))
            .count()
    };
    lines.push(Line::from(Span::styled(
        format!(
            "{} levers differ between A and B",
            changed(&app.compare.a.1, &app.compare.b.1)
        ),
        th.dim(),
    )));
    lines.push(Line::from(""));
    if app.compare_busy() {
        lines.push(Line::from(Span::styled("running...", th.bright())));
    }
    match &app.compare.result {
        None if !app.compare_busy() => {
            lines.push(Line::from(Span::styled("press enter to run", th.dim())));
        }
        None => {}
        Some(Err(e)) => lines.push(Line::from(Span::styled(e.clone(), th.alert()))),
        Some(Ok((a, b))) => {
            lines.push(Line::from(Span::styled(
                format!("{:<16}{:>20}{:>20}", "", "A", "B"),
                th.dim(),
            )));
            for (name, x, y, better) in chainlab_engine::engine::analysis::compare_table(a, b) {
                let (sa, sb) = match better {
                    1 => (th.base(), th.emph()),
                    -1 => (th.emph(), th.base()),
                    _ => (th.base(), th.base()),
                };
                let mark = match better {
                    1 => " <B",
                    -1 => " <A",
                    _ => "",
                };
                lines.push(Line::from(vec![
                    Span::styled(fit(&name, 16), th.dim()),
                    Span::styled(rfit(&fit(&x, 19), 20), sa),
                    Span::styled(rfit(&fit(&y, 19), 20), sb),
                    Span::styled(mark, th.bright()),
                ]));
            }
        }
    }
    f.render_widget(Paragraph::new(lines), inner);
}
