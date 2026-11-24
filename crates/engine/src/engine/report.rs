//! Plain-text summary of a sim, used by `--headless` and for comparisons.

use std::fmt::Write;

use crate::fixed::{fmt, to_f64};

use super::Sim;
use super::analysis::{RunSummary, compare_table};
use super::chain::{Kind, StockSpec};
use super::logistics::max_rate_per_sec;
use super::model::Status;
use super::{gates, scoring};

pub fn summary(sim: &Sim) -> String {
    let c = &*sim.chain;
    let st = &sim.state;
    let eff = sim.eff_all();
    let mut o = String::new();
    let secs = sim.time_secs();
    let _ = writeln!(
        o,
        "CHAINLAB headless | chain \"{}\" | seed {} | {} ticks ({:02}:{:04.1} sim) | tier {}",
        c.name,
        sim.seed,
        st.tick,
        (secs / 60.0) as u64,
        secs % 60.0,
        st.tier
    );
    let _ = writeln!(o);
    let _ = writeln!(
        o,
        "{:<16} {:<10} {:>5} {:>5} {:>5} {:>5} {:>5} {:>4} {:>8} {:>8} {:>10}",
        "NODE",
        "STATUS",
        "WORK",
        "STRV",
        "BLKD",
        "OFF",
        "CREW",
        "WKRS",
        "OUT/min",
        "CAP/min",
        "OUT total"
    );
    for (i, n) in c.nodes.iter().enumerate() {
        let ns = &st.nodes[i];
        let s = &st.stats.nodes[i];
        let pc = |x: Status| format!("{:.0}%", s.share(x) * 100.0);
        let tag = if sim.is_locked(i) {
            "[LOCKED]"
        } else {
            ns.status.tag()
        };
        let _ = writeln!(
            o,
            "{:<16} {:<10} {:>5} {:>5} {:>5} {:>5} {:>5} {:>4} {:>8} {:>8} {:>10}",
            truncate(&n.name, 16),
            tag,
            pc(Status::Working),
            pc(Status::Starved),
            pc(Status::Blocked),
            pc(Status::Off),
            pc(Status::Unstaffed),
            sim.workers(n),
            fmt(s.per_min(), 1),
            fmt(sim.capacity_per_min(i), 1),
            fmt(s.out_total, 1),
        );
    }

    let _ = writeln!(o);
    let _ = writeln!(o, "BUFFERS");
    for (i, n) in c.nodes.iter().enumerate() {
        let ns = &st.nodes[i];
        let in_cap = n.p.in_cap.map_or(0, |p| sim.eff(p));
        let out_cap = n.p.out_cap.map_or(0, |p| sim.eff(p));
        for (k, &r) in n.in_res.iter().enumerate() {
            let _ = writeln!(
                o,
                "  {:<16} in  {:<10} {:>8} / {:<8} {}",
                truncate(&n.name, 16),
                c.res_name(r),
                fmt(ns.inputs[k], 1),
                fmt(in_cap, 1),
                bar(ns.inputs[k], in_cap, 20)
            );
        }
        for (k, &r) in n.out_res.iter().enumerate() {
            let _ = writeln!(
                o,
                "  {:<16} out {:<10} {:>8} / {:<8} {}",
                truncate(&n.name, 16),
                c.res_name(r),
                fmt(ns.outputs[k], 1),
                fmt(out_cap, 1),
                bar(ns.outputs[k], out_cap, 20)
            );
        }
        if let Kind::Source(spec) = &n.kind {
            let (label, cap) = match spec.stock {
                StockSpec::Infinite => continue,
                StockSpec::Depleting { initial } => ("deposit", initial),
                StockSpec::Regrowing { max, .. } => ("standing", max),
            };
            let _ = writeln!(
                o,
                "  {:<16} {:<14} {:>8} / {:<8} {}",
                truncate(&n.name, 16),
                label,
                fmt(ns.stock, 1),
                fmt(cap, 1),
                bar(ns.stock, cap, 20)
            );
        }
    }

    if !c.links.is_empty() {
        let _ = writeln!(o);
        let _ = writeln!(o, "LINKS");
        for (li, l) in c.links.iter().enumerate() {
            let ls = &st.links[li];
            let carriers = sim.eff(l.p.carriers);
            let ceiling = max_rate_per_sec(carriers, sim.eff(l.p.capacity), sim.eff(l.p.distance));
            let _ = writeln!(
                o,
                "  {:<26} {:<7} carts {:>2}/{:<2} load {:>3.0}%  transit {:>5}  delivered {:>7}  max {:>5.1}/min",
                truncate(&c.link_label(li), 26),
                c.res_name(l.res),
                ls.shipments.len(),
                carriers,
                st.stats.links[li].load() * 100.0,
                fmt(ls.in_transit(), 1),
                fmt(ls.delivered_total, 1),
                to_f64(ceiling) * 60.0
            );
        }
    }

    let s = &st.stats;
    let _ = writeln!(o);
    let _ = writeln!(o, "ECONOMY");
    let _ = writeln!(
        o,
        "  coin {:.1} | income {:.1}/min | expenses {:.1}/min (upkeep {:.1}, purchases {:.1}) | net {:.1}/min",
        to_f64(st.coin),
        to_f64(s.income_per_min()),
        to_f64(s.expenses_per_min()),
        to_f64(s.upkeep_per_min()),
        to_f64(s.purchases_per_min()),
        to_f64(s.net_per_min())
    );
    let _ = writeln!(
        o,
        "  cumulative operating profit {:.1} | purchases {:.1} | workers {}/{} placed (requested {})",
        to_f64(st.ledger.operating_profit()),
        to_f64(st.ledger.purchase_total),
        sim.workers_assigned(),
        sim.worker_pool(),
        sim.workers_requested()
    );
    if let Some(m) = &c.market {
        for (k, g) in m.goods.iter().enumerate() {
            let gs = &st.market[k];
            let _ = writeln!(
                o,
                "  {:<8} price {:>6.2} (base {:>5.2}) | sold {:>7.1} | revenue {:>8.1} | {:>5.1}/min",
                c.res_name(g.res),
                to_f64(gs.price),
                to_f64(sim.eff(g.base_price)),
                to_f64(gs.sold_total),
                to_f64(gs.revenue_total),
                to_f64(s.goods[k].sold.per_window())
            );
        }
    }
    let _ = writeln!(
        o,
        "  progress {:.1} ({:.1}/min)",
        to_f64(st.progress),
        to_f64(s.progress_per_min())
    );

    let _ = writeln!(o);
    let _ = writeln!(o, "ANALYSIS");
    match scoring::bottleneck(c, eff, st) {
        Some(b) => {
            let _ = writeln!(o, "  bottleneck: {} (score {:.2})", b.reason, b.score);
        }
        None => {
            let _ = writeln!(o, "  bottleneck: none stands out");
        }
    }
    let e = scoring::efficiency(c, eff, st);
    let _ = writeln!(
        o,
        "  efficiency {}/100 (utilisation {:.0}%, workforce {:.0}%, margin {:.0}%)",
        e.score,
        e.utilisation * 100.0,
        e.workforce * 100.0,
        e.margin * 100.0
    );
    if let Some((i, frac, eta)) = gates::next(c, st) {
        let _ = writeln!(
            o,
            "  next gate: {} {:.0}% {}",
            c.gates[i].name,
            frac * 100.0,
            eta.map_or("(not progressing)".into(), |s| format!("(~{s:.0}s)"))
        );
    }
    if st.upgrades.iter().any(|&l| l > 0) {
        let owned: Vec<String> = c
            .upgrades
            .iter()
            .zip(&st.upgrades)
            .filter(|(_, l)| **l > 0)
            .map(|(u, l)| format!("{} x{l}", u.name))
            .collect();
        let _ = writeln!(o, "  upgrades: {}", owned.join(", "));
    }
    o
}

pub fn compare(a: &RunSummary, b: &RunSummary) -> String {
    let mut o = String::new();
    let _ = writeln!(o, "{:<16} {:>18} {:>18}", "", a.label, b.label);
    for (name, x, y, better) in compare_table(a, b) {
        let mark = match better {
            1 => "  <- B",
            -1 => "  <- A",
            _ => "",
        };
        let _ = writeln!(o, "{name:<16} {x:>18} {y:>18}{mark}");
    }
    o
}

fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// `[#####.....]` fill bar.
pub fn bar(v: i64, cap: i64, width: usize) -> String {
    let filled = if cap <= 0 {
        0
    } else {
        ((v.max(0) as i128 * width as i128) / cap as i128).min(width as i128) as usize
    };
    format!("[{}{}]", "#".repeat(filled), ".".repeat(width - filled))
}
