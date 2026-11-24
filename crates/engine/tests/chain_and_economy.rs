//! Tests for the full chain, workers, market, upgrades, gates, events,
//! bottleneck detection and payback analysis.

use std::collections::BTreeMap;

use chainlab_engine::config::parse_chain;
use chainlab_engine::engine::analysis;
use chainlab_engine::engine::economy::BuyBlock;
use chainlab_engine::engine::model::{EventKind, Status};
use chainlab_engine::engine::scoring::{self, Limiter};
use chainlab_engine::fixed::{ONE, fx};
use chainlab_engine::{Command, Sim, scenarios};

fn default_sim(sets: &[(&str, f64)]) -> Sim {
    let mut sim = scenarios::load("balanced").unwrap().build(Some(7)).unwrap();
    let map: BTreeMap<String, f64> = sets.iter().map(|(k, v)| (k.to_string(), *v)).collect();
    assert!(sim.apply_values(&map, false).is_empty());
    sim
}

fn id(sim: &Sim, key: &str) -> usize {
    sim.params
        .id(key)
        .unwrap_or_else(|| panic!("no param {key}"))
}

/// Source of bricks feeding a market directly.
fn market_sim(extra_market: &str, rate: f64) -> Sim {
    let text = format!(
        r#"(
        name: "M",
        resources: [(id: "bricks", name: "Bricks")],
        globals: (workers_total: 0, worker_upkeep: 0.0, start_coin: 0.0),
        nodes: [
            (id: "src", name: "Src", kind: Source(output: "bricks", rate: {rate}), out_cap: 100.0),
            (id: "market", name: "Market", kind: Market, in_cap: 1000.0),
        ],
        links: [(from: "src", to: "market", resource: "bricks", carriers: 10, capacity: 1.0, distance_secs: 0.1)],
        market: (goods: [(resource: "bricks", base_price: 10.0, elasticity: 0.02, recovery: 0.05)] {extra_market}),
    )"#
    );
    Sim::new(&parse_chain(&text).unwrap(), 1).unwrap()
}

#[test]
fn sink_consumes_complete_sets_up_to_lift() {
    let text = r#"(
        name: "S",
        resources: [(id: "a", name: "A"), (id: "b", name: "B")],
        nodes: [
            (id: "t", name: "Tower",
             kind: Sink(inputs: [("a", 2.0), ("b", 1.0)], progress: 1.0, lift_rate: 1.0),
             in_cap: 100.0),
        ],
    )"#;
    let mut sim = Sim::new(&parse_chain(text).unwrap(), 1).unwrap();
    sim.state.nodes[0].inputs = vec![20 * ONE, 5 * ONE];
    // Lift: 1 set/s = 0.1 set/tick. Planks (b) allow only 5 sets.
    sim.run(10);
    assert_eq!(sim.state.progress, ONE);
    assert_eq!(sim.state.nodes[0].status, Status::Working);
    sim.run(100);
    assert_eq!(sim.state.progress, 5 * ONE);
    assert_eq!(sim.state.nodes[0].inputs, vec![10 * ONE, 0]);
    assert_eq!(sim.state.nodes[0].status, Status::Starved);
}

#[test]
fn worker_pool_clips_requests_upstream_first() {
    let sim = default_sim(&[("global.workers", 3.0)]);
    // Order: quarry(2) gets 2, forest gets the last 1, the rest get none.
    let n = |id: &str| &sim.chain.nodes[sim.chain.node_index(id).unwrap()];
    assert_eq!(sim.workers(n("quarry")), 2);
    assert_eq!(sim.workers(n("forest")), 1);
    assert_eq!(sim.workers(n("kiln")), 0);
    assert_eq!(sim.workers_requested(), 10);
    let mut sim = sim;
    sim.run(5);
    let kiln = sim.chain.node_index("kiln").unwrap();
    assert_eq!(sim.state.nodes[kiln].status, Status::Unstaffed);
}

#[test]
fn upkeep_is_exact_over_a_minute() {
    let mut sim = default_sim(&[("global.workers", 7.0), ("global.upkeep", 1.3)]);
    sim.run(600);
    assert_eq!(sim.state.ledger.upkeep_total, fx(7.0 * 1.3));
}

#[test]
fn market_price_falls_with_sales_and_recovers() {
    let mut sim = market_sim("", 20.0);
    sim.run(300);
    let p_selling = sim.state.market[0].price;
    assert!(p_selling < fx(8.0), "price under pressure {p_selling}");
    assert!(sim.state.coin > 0);
    // Stop selling (the market's buffer would otherwise keep it selling).
    let m = id(&sim, "node.market.enabled");
    sim.command(Command::Set(m, 0));
    sim.run(3000);
    let p_rest = sim.state.market[0].price;
    assert!(p_rest > fx(9.9), "price recovered {p_rest}");
}

#[test]
fn threshold_policy_holds_stock_when_price_is_low() {
    // Sell above 95% of base only: pressure quickly pushes price below that.
    let mut sim = market_sim(", policy: 1, threshold_pct: 95", 20.0);
    sim.run(400);
    let m = sim.chain.market.as_ref().unwrap().node;
    assert!(sim.state.nodes[m].inputs[0] > 10 * ONE, "stock held back");
    assert!(sim.state.stats.nodes[m].share(Status::Blocked) > 0.3);
    // Sell-all policy sells through instead.
    let mut all = market_sim("", 20.0);
    all.run(400);
    assert!(all.state.market[0].sold_total > sim.state.market[0].sold_total);
}

#[test]
fn reserve_policy_keeps_units() {
    let mut sim = market_sim(", policy: 2, reserve: 15.0", 5.0);
    sim.run(600);
    let src = id(&sim, "node.src.enabled");
    sim.command(Command::Set(src, 0));
    sim.run(3000);
    let m = sim.chain.market.as_ref().unwrap().node;
    assert_eq!(sim.state.nodes[m].inputs[0], 15 * ONE);
}

#[test]
fn purchase_applies_effect_and_escalates_cost() {
    let mut sim = default_sim(&[]);
    sim.state.coin = fx(1000.0);
    let u = sim.chain.upgrade_index("kiln_speed").unwrap();
    let cycle = id(&sim, "node.kiln.cycle");
    assert_eq!(sim.eff(cycle), 40);
    let c0 = sim.upgrade_cost(u);
    assert_eq!(c0, fx(40.0));
    sim.command(Command::Buy(u));
    assert_eq!(sim.state.upgrades[u], 1);
    assert_eq!(sim.state.coin, fx(960.0));
    assert_eq!(sim.eff(cycle), 36, "x0.9 cycle");
    assert_eq!(sim.params.get(cycle), 40, "lever value untouched");
    assert_eq!(sim.upgrade_cost(u), fx(47.2));
    // Growth is a live lever.
    let g = id(&sim, "cost.kiln_speed.growth");
    sim.command(Command::Set(g, 2_000));
    assert_eq!(sim.upgrade_cost(u), fx(80.0));
    // Discount lever.
    let d = id(&sim, "cost.discount");
    sim.command(Command::Set(d, 50));
    assert_eq!(sim.upgrade_cost(u), fx(40.0));
    assert_eq!(sim.state.ledger.purchase_total, fx(40.0));
    assert!(
        sim.state
            .log
            .iter()
            .any(|e| e.text.contains("BUY Kiln: hotter fire"))
    );
}

#[test]
fn purchase_refused_when_poor_or_locked() {
    let mut sim = default_sim(&[]);
    sim.state.coin = fx(10.0);
    let u = sim.chain.upgrade_index("kiln_build").unwrap();
    assert!(matches!(sim.buy(u), Err(BuyBlock::TooExpensive(_))));
    sim.state.coin = fx(10_000.0);
    let lift = sim.chain.upgrade_index("lift").unwrap();
    assert_eq!(sim.buy(lift), Err(BuyBlock::Locked(1)));
}

#[test]
fn building_units_scale_output_and_crew() {
    let mut sim = default_sim(&[]);
    let kiln = sim.chain.node_index("kiln").unwrap();
    let before = sim.capacity_per_min(kiln);
    sim.state.coin = fx(1000.0);
    sim.command(Command::Buy(sim.chain.upgrade_index("kiln_build").unwrap()));
    // Two kilns but still only 2 workers: crew factor halves, so no gain...
    assert_eq!(sim.capacity_per_min(kiln), before);
    // ...until the second kiln is staffed.
    let w = id(&sim, "node.kiln.workers");
    sim.command(Command::Set(w, 4));
    let g = id(&sim, "global.workers");
    sim.command(Command::Set(g, 20));
    assert_eq!(sim.capacity_per_min(kiln), before * 2);
}

#[test]
fn gates_unlock_tiers_and_nodes() {
    let mut sim = default_sim(&[]);
    let deep = sim.chain.node_index("deep_quarry").unwrap();
    assert!(sim.is_locked(deep));
    sim.run(20);
    assert_eq!(sim.state.nodes[deep].status, Status::Off);
    sim.state.progress = fx(30.0);
    sim.step();
    assert_eq!(sim.state.tier, 1);
    sim.state.ledger.income_total = fx(10_000.0);
    sim.step();
    assert_eq!(sim.state.tier, 2);
    assert!(!sim.is_locked(deep));
    let w = id(&sim, "node.deep_quarry.workers");
    let g = id(&sim, "global.workers");
    sim.command(Command::Set(g, 20));
    sim.command(Command::Set(w, 2));
    sim.run(5);
    assert_eq!(sim.state.nodes[deep].status, Status::Working);
    assert!(
        sim.state
            .log
            .iter()
            .any(|e| e.text.contains("UNLOCKED tier 2"))
    );
}

#[test]
fn drought_and_breakdown_modify_effective_params() {
    let mut sim = default_sim(&[]);
    let water = sim.chain.g.water;
    sim.command(Command::Trigger {
        kind: EventKind::Drought,
        severity: 60,
        pick: 0.0,
    });
    assert_eq!(sim.eff(water), fx(0.4));
    sim.run(700); // drought lasts at most 60 s
    assert_eq!(sim.eff(water), fx(1.0));

    sim.command(Command::Trigger {
        kind: EventKind::Breakdown,
        severity: 50,
        pick: 0.0,
    });
    let broken = sim.state.events[0].target.unwrap();
    assert!(sim.is_broken(broken));
    sim.step();
    assert_eq!(sim.state.nodes[broken].status, Status::Off);
}

#[test]
fn price_crash_hits_drift_then_recovers() {
    let mut sim = default_sim(&[]);
    sim.command(Command::Trigger {
        kind: EventKind::PriceCrash,
        severity: 100,
        pick: 0.0,
    });
    assert_eq!(sim.state.market[0].drift, fx(0.3));
    sim.run(1200);
    assert!(sim.state.market[0].drift > fx(0.85));
}

#[test]
fn bottleneck_follows_the_levers() {
    let name = |sim: &Sim| {
        let b = scoring::bottleneck(&sim.chain, sim.eff_all(), &sim.state).expect("a bottleneck");
        match b.at {
            Limiter::Node(n) => sim.chain.nodes[n].id.clone(),
            Limiter::Link(l) => format!("link{l}"),
        }
    };
    let mut s = default_sim(&[]);
    s.run(3000);
    assert_eq!(name(&s), "kiln");

    let mut s = default_sim(&[("node.kiln.cycle", 2.0)]);
    s.run(3000);
    assert_eq!(name(&s), "tower");

    let mut s = default_sim(&[("node.kiln.cycle", 1.5), ("node.tower.lift", 0.4)]);
    s.run(3000);
    assert_eq!(name(&s), "quarry");

    let mut s = default_sim(&[("link.0.carriers", 1.0), ("link.0.distance", 10.0)]);
    s.run(3000);
    assert_eq!(name(&s), "link0");

    let mut s = default_sim(&[("node.kiln.workers", 0.0)]);
    s.run(3000);
    assert_eq!(name(&s), "kiln");
}

#[test]
fn payback_ranks_bottleneck_upgrades_above_others() {
    let mut sim = default_sim(&[]);
    sim.run(1800);
    let pb = analysis::payback(&sim, analysis::HORIZON);
    let get = |id: &str| {
        let u = sim.chain.upgrade_index(id).unwrap();
        pb.iter().find(|p| p.upgrade == u).unwrap().clone()
    };
    let kiln = get("kiln_speed");
    let quarry = get("quarry_rate");
    assert!(kiln.d_progress > 0, "faster kiln helps progress: {kiln:?}");
    assert!(
        quarry.d_progress <= kiln.d_progress / 4,
        "quarry is not the bottleneck: {quarry:?}"
    );
    // The forked analysis must not touch the real sim.
    assert_eq!(sim.state.tick, 1800);
}

#[test]
fn all_scenarios_load_and_run() {
    for (id, name, _) in scenarios::list() {
        let mut sim = scenarios::load(&id).unwrap().build(None).unwrap();
        sim.run(600);
        assert!(sim.state.tick == 600, "{name}");
    }
    let sf = scenarios::load("starved_forest")
        .unwrap()
        .build(None)
        .unwrap();
    let w = id(&sf, "global.water");
    assert_eq!(
        sf.params.list[w].default,
        fx(0.35),
        "override becomes default"
    );
    assert!(!sf.params.list[w].changed());
}

#[test]
fn full_chain_determinism_with_events_and_purchases() {
    let script = |sim: &mut Sim| {
        let ev = sim.chain.g.ev_enabled;
        sim.command(Command::Set(ev, 1));
        let freq = sim.chain.g.ev_frequency;
        sim.command(Command::Set(freq, fx(3.0)));
        for t in 0..6_000u64 {
            if t % 900 == 450 {
                sim.command(Command::Buy((t / 900) as usize % sim.chain.upgrades.len()));
            }
            sim.step();
        }
    };
    let mut a = default_sim(&[]);
    let mut b = default_sim(&[]);
    script(&mut a);
    script(&mut b);
    assert_eq!(a.state, b.state);
    assert!(
        a.state
            .log
            .iter()
            .any(|e| e.kind == chainlab_engine::engine::model::LogKind::Event)
    );
}
