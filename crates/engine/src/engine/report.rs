//! Plain-text summary of a sim, used by `--headless` and for comparisons.

use std::fmt::Write;

use crate::fixed::{fmt, to_f64};

use super::Sim;
use super::chain::{Kind, StockSpec};
use super::logistics::max_rate_per_sec;
use super::model::Status;

pub fn summary(sim: &Sim) -> String {
    let c = &*sim.chain;
    let st = &sim.state;
    let mut o = String::new();
    let secs = sim.time_secs();
    let _ = writeln!(
        o,
        "CHAINLAB headless | chain \"{}\" | seed {} | {} ticks ({:02}:{:04.1} sim)",
        c.name,
        sim.seed,
        st.tick,
        (secs / 60.0) as u64,
        secs % 60.0
    );
    let _ = writeln!(o);
    let _ = writeln!(
        o,
        "{:<16} {:<10} {:>5} {:>5} {:>5} {:>5} {:>5} {:>4} {:>9} {:>10}",
        "NODE", "STATUS", "WORK", "STRV", "BLKD", "OFF", "CREW", "WKRS", "OUT/min", "OUT total"
    );
    for (i, n) in c.nodes.iter().enumerate() {
        let ns = &st.nodes[i];
        let s = &st.stats.nodes[i];
        let pc = |x: Status| format!("{:.0}%", s.share(x) * 100.0);
        let _ = writeln!(
            o,
            "{:<16} {:<10} {:>5} {:>5} {:>5} {:>5} {:>5} {:>4} {:>9} {:>10}",
            truncate(&n.name, 16),
            ns.status.tag(),
            pc(Status::Working),
            pc(Status::Starved),
            pc(Status::Blocked),
            pc(Status::Off),
            pc(Status::Unstaffed),
            sim.workers(n),
            fmt(s.per_min(), 1),
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
            match spec.stock {
                StockSpec::Infinite => {}
                StockSpec::Depleting { initial } => {
                    let _ = writeln!(
                        o,
                        "  {:<16} deposit        {:>8} / {:<8} {}",
                        truncate(&n.name, 16),
                        fmt(ns.stock, 1),
                        fmt(initial, 1),
                        bar(ns.stock, initial, 20)
                    );
                }
                StockSpec::Regrowing { max, .. } => {
                    let _ = writeln!(
                        o,
                        "  {:<16} standing       {:>8} / {:<8} {}",
                        truncate(&n.name, 16),
                        fmt(ns.stock, 1),
                        fmt(max, 1),
                        bar(ns.stock, max, 20)
                    );
                }
            }
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
                "  {:<28} {:<8} carriers {}/{} busy  in transit {:>6}  delivered {:>8}  ceiling {:.1}/min",
                truncate(&c.link_label(li), 28),
                c.res_name(l.res),
                ls.shipments.len(),
                carriers,
                fmt(ls.in_transit(), 1),
                fmt(ls.delivered_total, 1),
                to_f64(ceiling) * 60.0
            );
        }
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
