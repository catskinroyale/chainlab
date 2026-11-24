//! Main screen layout and panels.
//!
//! ```text
//! ┌ top bar: title, time, speed, seed, scenario, challenge ─────────────┐
//! │ FLOW diagram                               │ STATS                  │
//! │                                            │                        │
//! ├────────────────────────────────────────────┼────────────────────────┤
//! │ LEVERS                                     │ LOG                    │
//! └ footer: key hints / messages ──────────────────────────────────────┘
//! ```
//!
//! At 80 columns the right column narrows and sparklines shrink. At 24 rows
//! the diagram uses compact boxes.

use std::time::Duration;

use chainlab_engine::challenges::ChallengeId;
use chainlab_engine::engine::model::Status;
use chainlab_engine::engine::params::Group;
use chainlab_engine::engine::{gates, scoring};
use chainlab_engine::fixed::{fx, to_f64};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};

use super::widgets::{
    clock, fit, gauge, num, rfit, signed, sparkline, stacked, status_glyph, trend,
};
use super::{App, SPEEDS, flow, overlays};

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let th = app.theme;
    f.render_widget(Block::new().style(th.base()), area);
    if area.width < 80 || area.height < 24 {
        let msg = format!(
            "CHAINLAB needs at least 80x24 (now {}x{}). Enlarge the terminal.",
            area.width, area.height
        );
        f.render_widget(Paragraph::new(msg).style(th.bright()), area);
        return;
    }
    let bottom_h = (area.height / 3).clamp(8, 12);
    let [top, mid, bottom, foot] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(8),
        Constraint::Length(bottom_h),
        Constraint::Length(1),
    ])
    .areas(area);
    let side = if area.width >= 100 { 38 } else { 32 };
    let [flow_a, stats_a] =
        Layout::horizontal([Constraint::Min(40), Constraint::Length(side)]).areas(mid);
    let [lev_a, log_a] =
        Layout::horizontal([Constraint::Min(40), Constraint::Length(side)]).areas(bottom);

    top_bar(f, top, app);
    flow_panel(f, flow_a, app);
    stats_panel(f, stats_a, app);
    lever_panel(f, lev_a, app);
    log_panel(f, log_a, app);
    footer(f, foot, app);
    overlays::draw(f, area, app);
}

fn panel<'a>(app: &App, title: &'a str) -> Block<'a> {
    Block::bordered()
        .border_type(BorderType::Plain)
        .border_style(app.theme.dim())
        .title(Span::styled(format!(" {title} "), app.theme.bright()))
        .style(app.theme.base())
}

fn top_bar(f: &mut Frame, area: Rect, app: &App) {
    let th = app.theme;
    let speed = SPEEDS[app.speed];
    let speed_txt = if speed.is_infinite() {
        "MAX".to_string()
    } else if speed < 1.0 {
        format!("{speed}x")
    } else {
        format!("{speed:.0}x")
    };
    let mut spans = vec![
        Span::styled(" CHAINLAB ", th.sel()),
        Span::styled(format!(" t {} ", clock(app.sim.time_secs())), th.bright()),
        if app.paused {
            Span::styled(" || PAUSED ", th.alert())
        } else {
            Span::styled(format!(" > {speed_txt} "), th.base())
        },
        Span::styled(format!("| seed {} ", app.sim.seed), th.dim()),
        Span::styled(format!("| {} ", app.loaded.scenario.name), th.base()),
    ];
    if app.run.id != ChallengeId::Sandbox {
        let w = 10;
        spans.push(Span::styled(format!("| {} ", app.run.id.name()), th.emph()));
        spans.push(Span::styled(
            format!(
                "[{}] {:.0}% ",
                gauge(fx(app.run.progress), fx(1.0), w),
                app.run.progress * 100.0
            ),
            if app.run.is_running() {
                th.bright()
            } else {
                th.emph()
            },
        ));
    } else {
        spans.push(Span::styled("| sandbox ", th.dim()));
    }
    if app.sim.params.get(app.sim.chain.g.ev_enabled) != 0 {
        spans.push(Span::styled("| events ON ", th.warn()));
    }
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let tps = format!("{:.0} ticks/s ", app.tps);
    if used + tps.len() < area.width as usize {
        spans.push(Span::styled(
            " ".repeat(area.width as usize - used - tps.len()),
            th.base(),
        ));
        spans.push(Span::styled(tps, th.dim()));
    }
    f.render_widget(Paragraph::new(Line::from(spans)).style(th.base()), area);
}

fn flow_panel(f: &mut Frame, area: Rect, app: &App) {
    let th = app.theme;
    let mut block = panel(app, "FLOW");
    let legend = " o loaded  · empty  O waiting  ═ bottleneck ";
    if app.show_bottleneck() {
        if let Some(b) = scoring::bottleneck(&app.sim.chain, app.sim.eff_all(), &app.sim.state) {
            let w = area.width.saturating_sub(4) as usize;
            block = block.title_bottom(Span::styled(
                fit(&format!(" BOTTLENECK: {} ", b.reason), w),
                th.emph(),
            ));
        } else if app.sim.state.tick > 100 {
            block = block.title_bottom(Span::styled(
                " no single bottleneck: slack everywhere ",
                th.dim(),
            ));
        } else {
            block = block.title_bottom(Span::styled(legend, th.dim()));
        }
    } else {
        block = block.title_bottom(Span::styled(
            " bottleneck marker hidden: find it yourself ",
            th.warn(),
        ));
    }
    let inner = block.inner(area);
    f.render_widget(block, area);
    flow::draw(f.buffer_mut(), inner, &app.sim, &th, app.show_bottleneck());
}

fn stats_panel(f: &mut Frame, area: Rect, app: &App) {
    let th = app.theme;
    let block = panel(app, "STATS");
    let inner = block.inner(area);
    f.render_widget(block, area);
    let iw = inner.width as usize;
    let sim = &app.sim;
    let st = &sim.state;
    let s = &st.stats;
    let c = &*sim.chain;
    let mut lines: Vec<Line> = vec![];

    // Money
    let e = scoring::efficiency(c, sim.eff_all(), st);
    let coin_style = if st.coin < 0 { th.alert() } else { th.emph() };
    let left = format!("COIN {} {}", num(st.coin), trend(s.history.coin.iter()));
    let right = format!("EFF {}/100", e.score);
    lines.push(Line::from(vec![
        Span::styled(left.clone(), coin_style),
        Span::styled(
            rfit(&right, iw.saturating_sub(left.chars().count())),
            th.bright(),
        ),
    ]));
    lines.push(Line::from(Span::styled(
        sparkline(s.history.coin.iter(), iw),
        th.base(),
    )));
    lines.push(Line::from(Span::styled(
        fit(
            &format!(
                "util {:.0}% workforce {:.0}% margin {:.0}%",
                e.utilisation * 100.0,
                e.workforce * 100.0,
                e.margin * 100.0
            ),
            iw,
        ),
        th.dim(),
    )));
    let net = s.net_per_min();
    lines.push(Line::from(vec![
        Span::styled("in ", th.dim()),
        Span::styled(num(s.income_per_min()), th.base()),
        Span::styled(" out ", th.dim()),
        Span::styled(num(s.expenses_per_min()), th.base()),
        Span::styled(" net ", th.dim()),
        Span::styled(
            format!("{}/m", signed(net)),
            if net < 0 { th.alert() } else { th.bright() },
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("profit total ", th.dim()),
        Span::styled(num(st.ledger.operating_profit()), th.base()),
        Span::styled(format!("  tier {}/{}", st.tier, c.gates.len()), th.dim()),
    ]));

    // Progress and gates
    let ptxt = format!("PROG {} {}/m ", num(st.progress), num(s.progress_per_min()));
    let pw = iw.saturating_sub(ptxt.chars().count());
    lines.push(Line::from(vec![
        Span::styled(ptxt, th.bright()),
        Span::styled(sparkline(s.history.progress.iter(), pw), th.base()),
    ]));
    if let Some((i, frac, eta)) = gates::next(c, st) {
        let eta = eta.map_or("--".into(), |x| {
            if x > 3600.0 {
                ">1h".into()
            } else {
                format!("{}:{:02}", (x / 60.0) as u64, x as u64 % 60)
            }
        });
        let head = format!("next {} ", fit(&c.gates[i].name, 10));
        let tail = format!(" {:.0}% ~{eta}", frac * 100.0);
        let gw = iw
            .saturating_sub(head.chars().count() + tail.chars().count())
            .max(1);
        lines.push(Line::from(vec![
            Span::styled(head, th.dim()),
            Span::styled(gauge(fx(frac), fx(1.0), gw), th.base()),
            Span::styled(tail, th.dim()),
        ]));
    } else if !c.gates.is_empty() {
        lines.push(Line::from(Span::styled("all gates passed", th.emph())));
    }

    // Nodes: throughput + sparkline + utilisation
    let name_w = 8;
    let uw = if iw >= 34 { 10 } else { 8 };
    let sw = iw.saturating_sub(name_w + 1 + 5 + 1 + 1 + uw);
    lines.push(Line::from(Span::styled(
        fit(
            &format!(
                "{} out/min {} util",
                "-".repeat(name_w),
                "-".repeat(sw.saturating_sub(2))
            ),
            iw,
        ),
        th.dim(),
    )));
    for (i, n) in c.nodes.iter().enumerate() {
        let ns = &s.nodes[i];
        let name: String = n.name.chars().take(name_w).collect();
        if sim.is_locked(i) {
            lines.push(Line::from(vec![
                Span::styled(fit(&name, name_w + 1), th.dim()),
                Span::styled(format!("locked until tier {}", n.tier), th.dim()),
            ]));
            continue;
        }
        let mut spans = vec![
            Span::styled(fit(&name, name_w + 1), th.status(st.nodes[i].status)),
            Span::styled(rfit(&num(ns.per_min()), 5), th.base()),
            Span::raw(" "),
            Span::styled(sparkline(s.history.nodes[i].iter(), sw), th.dim()),
            Span::raw(" "),
        ];
        for (status, cells) in stacked(|x| ns.share(x), uw) {
            spans.push(Span::styled(
                status_glyph(status).to_string().repeat(cells),
                th.status(status),
            ));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::from(vec![
        Span::styled("█", th.base()),
        Span::styled("work ", th.dim()),
        Span::styled("▒", th.warn()),
        Span::styled("starv ", th.dim()),
        Span::styled("▓", th.alert()),
        Span::styled("block ", th.dim()),
        Span::styled("░", th.warn()),
        Span::styled("crew ", th.dim()),
        Span::styled("·", th.dim()),
        Span::styled("off", th.dim()),
    ]));

    // Market
    if let Some(m) = &c.market {
        for (k, g) in m.goods.iter().enumerate() {
            let gs = &st.market[k];
            let head = format!(
                "{} {:.2}{} {}/m ",
                fit(c.res_name(g.res), 6),
                to_f64(gs.price),
                trend(s.history.prices[k].iter()),
                num(s.goods[k].sold.per_window())
            );
            let w = iw.saturating_sub(head.chars().count());
            lines.push(Line::from(vec![
                Span::styled(head, th.base()),
                Span::styled(sparkline(s.history.prices[k].iter(), w), th.dim()),
            ]));
        }
    }
    let workers = format!(
        "workers {}/{} placed{}",
        sim.workers_assigned(),
        sim.worker_pool(),
        if sim.workers_requested() > sim.worker_pool() {
            format!(" (want {})", sim.workers_requested())
        } else {
            String::new()
        }
    );
    lines.push(Line::from(Span::styled(
        workers,
        if sim.workers_requested() > sim.worker_pool() {
            th.warn()
        } else {
            th.dim()
        },
    )));

    // Challenge status
    if app.run.id != ChallengeId::Sandbox {
        let w = iw.saturating_sub(2);
        let pct = format!(" {:.0}%", app.run.progress * 100.0);
        lines.push(Line::from(vec![
            Span::styled(
                fit(app.run.id.name(), w.saturating_sub(pct.len())),
                th.emph(),
            ),
            Span::styled(pct, th.emph()),
        ]));
        lines.push(Line::from(Span::styled(
            gauge(fx(app.run.progress), fx(1.0), iw),
            th.bright(),
        )));
        for chunk in wrap(&app.run.status, iw) {
            lines.push(Line::from(Span::styled(chunk, th.base())));
        }
    }

    // Drop the least important lines if short on height.
    let h = inner.height as usize;
    if lines.len() > h {
        lines.remove(2); // efficiency breakdown
    }
    if lines.len() > h {
        lines.remove(1); // coin sparkline
    }
    if lines.len() > h {
        let legend = lines
            .iter()
            .position(|l| l.spans.first().is_some_and(|s| s.content == "█"));
        if let Some(i) = legend {
            lines.remove(i);
        }
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn lever_panel(f: &mut Frame, area: Rect, app: &App) {
    let th = app.theme;
    let groups = app.groups();
    let gi = app.group.min(groups.len().saturating_sub(1));
    let Some(&g) = groups.get(gi) else { return };
    let title = format!(
        "LEVERS < {} > {}/{}",
        app.group_label(g),
        gi + 1,
        groups.len()
    );
    let block = panel(app, &title).title_bottom(Span::styled(
        " up/dn  <-/-> adjust (shift coarse)  tab group  r/R reset ",
        th.dim(),
    ));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let iw = inner.width as usize;
    let sim = &app.sim;
    let ids = app.group_params(g);
    let mut lines: Vec<Line> = vec![];

    // Context line for nodes and links.
    let info = match g {
        Group::Node(n) => {
            if sim.is_locked(n) {
                format!("locked until tier {}", sim.chain.nodes[n].tier)
            } else {
                let ns = &sim.state.stats.nodes[n];
                format!(
                    "{} | actual {}/min of capacity {}/min | work {:.0}%",
                    sim.state.nodes[n].status.tag(),
                    num(ns.per_min()),
                    num(sim.capacity_per_min(n)),
                    ns.share(Status::Working) * 100.0
                )
            }
        }
        Group::Link(l) => {
            let link = &sim.chain.links[l];
            let cap = chainlab_engine::engine::logistics::max_rate_per_sec(
                sim.eff(link.p.carriers),
                sim.eff(link.p.capacity),
                sim.eff(link.p.distance),
            ) * 60;
            format!(
                "{} | carts busy {:.0}% | moved {}/min of max {}/min",
                sim.chain.res_name(link.res),
                sim.state.stats.links[l].busy_share() * 100.0,
                num(sim.state.stats.links[l].delivered.per_window()),
                num(cap)
            )
        }
        Group::Global => format!(
            "workers {}/{} placed | upkeep {}/min",
            sim.workers_assigned(),
            sim.worker_pool(),
            num(sim.state.stats.upkeep_per_min())
        ),
        _ => String::new(),
    };
    if !info.is_empty() {
        lines.push(Line::from(Span::styled(fit(&info, iw), th.dim())));
    }
    let lw = iw.saturating_sub(2 + 9 + 9 + 10 + 2).max(8);
    lines.push(Line::from(Span::styled(
        fit(
            &format!(
                "  {}{}{}{}",
                fit("lever", lw),
                rfit("value", 9),
                rfit("default", 9),
                rfit("effective", 10)
            ),
            iw,
        ),
        th.dim(),
    )));
    let rows = (inner.height as usize).saturating_sub(lines.len());
    let offset = app.sel.saturating_sub(rows.saturating_sub(1));
    for (k, &id) in ids.iter().enumerate().skip(offset).take(rows) {
        let p = &sim.params.list[id];
        let allowed = app.run.lever_allowed(&p.key);
        let eff = sim.eff(id);
        let eff_txt = if eff != p.value {
            p.unit.format(eff)
        } else {
            String::new()
        };
        let flag = if !allowed {
            "L"
        } else if p.changed() {
            "*"
        } else {
            " "
        };
        let text = format!(
            "{}{}{}{}{}{}",
            if k == app.sel { "> " } else { "  " },
            fit(p.short_label(), lw),
            rfit(&p.display(), 9),
            rfit(&p.display_default(), 9),
            rfit(&eff_txt, 10),
            format_args!(" {flag}")
        );
        let style = if k == app.sel {
            th.sel()
        } else if !allowed {
            th.dim()
        } else if p.changed() {
            th.bright()
        } else {
            th.base()
        };
        lines.push(Line::from(Span::styled(fit(&text, iw), style)));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn log_panel(f: &mut Frame, area: Rect, app: &App) {
    let th = app.theme;
    let block = panel(app, "LOG");
    let inner = block.inner(area);
    f.render_widget(block, area);
    let iw = inner.width as usize;
    let h = inner.height as usize;
    let log = &app.sim.state.log;
    let mut lines: Vec<Line> = vec![];
    for e in log.iter().skip(log.len().saturating_sub(h)) {
        let t = e.tick / chainlab_engine::TICKS_PER_SEC;
        lines.push(Line::from(vec![
            Span::styled(format!("{:02}:{:02} ", t / 60, t % 60), th.dim()),
            Span::styled(fit(&e.text, iw.saturating_sub(6)), th.log(e.kind)),
        ]));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn footer(f: &mut Frame, area: Rect, app: &App) {
    let th = app.theme;
    let w = area.width as usize;
    if let Some((msg, at)) = &app.flash
        && at.elapsed() < Duration::from_secs(4)
    {
        f.render_widget(
            Paragraph::new(Span::styled(fit(&format!(" {msg}"), w), th.emph())),
            area,
        );
        return;
    }
    let keys: &[(&str, &str)] = &[
        ("spc", "pause"),
        (".", "step"),
        ("-/=", "speed"),
        ("s", "shop"),
        ("m", "market"),
        ("c", "challenges"),
        ("a", "A/B"),
        ("n", "goto BN"),
        ("x", "retry"),
        ("w/o", "save/load"),
        ("e", "events"),
        ("t", "colours"),
        ("?", "help"),
        ("q", "quit"),
    ];
    let mut spans = vec![];
    let mut used = 0;
    for (k, v) in keys {
        let len = k.len() + v.len() + 2;
        if used + len > w {
            break;
        }
        spans.push(Span::styled(format!(" {k}"), th.bright()));
        spans.push(Span::styled(format!(" {v}"), th.dim()));
        used += len;
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Greedy word wrap.
pub fn wrap(text: &str, w: usize) -> Vec<String> {
    let mut out = vec![];
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > w {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}
