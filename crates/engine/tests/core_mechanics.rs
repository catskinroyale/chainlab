//! Headless tests of the core mechanics: recipes, buffers, logistics,
//! sources and determinism.

use chainlab_engine::config::parse_chain;
use chainlab_engine::engine::model::Status;
use chainlab_engine::fixed::{ONE, fx};
use chainlab_engine::{Command, Sim};

/// Quarry -> Kiln chain with tunable bits spliced in.
fn chain(source_extra: &str, quarry_cap: f64, kiln: &str, link: &str) -> Sim {
    let text = format!(
        r#"(
        name: "Test",
        resources: [(id: "stone", name: "Stone"), (id: "bricks", name: "Bricks")],
        nodes: [
            (id: "quarry", name: "Quarry",
             kind: Source(output: "stone", rate: 10.0 {source_extra}),
             out_cap: {quarry_cap}, workers: 1, optimal_workers: 1),
            (id: "kiln", name: "Kiln",
             kind: Processor(inputs: [("stone", 2.0)], outputs: [("bricks", 1.0)], cycle_secs: 1.0),
             {kiln}),
        ],
        links: [(from: "quarry", to: "kiln", resource: "stone", {link})],
    )"#
    );
    let cfg = parse_chain(&text).unwrap();
    Sim::new(&cfg, 1).unwrap()
}

fn id(sim: &Sim, key: &str) -> usize {
    sim.params
        .id(key)
        .unwrap_or_else(|| panic!("no param {key}"))
}

/// A kiln with its upstream switched off, so we control its input exactly.
fn isolated_kiln(kiln: &str) -> Sim {
    let mut sim = chain(
        "",
        10.0,
        kiln,
        "carriers: 1, capacity: 1.0, distance_secs: 1.0",
    );
    let q = id(&sim, "node.quarry.enabled");
    sim.command(Command::Set(q, 0));
    sim
}

#[test]
fn recipe_converts_inputs_to_outputs_exactly() {
    let mut sim = isolated_kiln("in_cap: 100.0, out_cap: 100.0, workers: 1, optimal_workers: 1");
    sim.state.nodes[1].inputs[0] = 20 * ONE;
    // 10 cycles of 10 ticks each are possible from 20 stone.
    sim.run(200);
    let k = &sim.state.nodes[1];
    assert_eq!(k.inputs[0], 0);
    assert_eq!(k.outputs[0], 10 * ONE);
    assert_eq!(k.status, Status::Starved);
    assert_eq!(sim.state.stats.nodes[1].out_total, 10 * ONE);
}

#[test]
fn efficiency_scales_output() {
    let mut sim = isolated_kiln(
        "in_cap: 100.0, out_cap: 100.0, workers: 1, optimal_workers: 1, efficiency: 50",
    );
    sim.state.nodes[1].inputs[0] = 20 * ONE;
    sim.run(200);
    assert_eq!(sim.state.nodes[1].outputs[0], 5 * ONE);
}

#[test]
fn cycle_time_and_crew_set_speed() {
    // Half crew => half speed: 1 cycle takes 20 ticks instead of 10.
    let mut sim = isolated_kiln("in_cap: 100.0, out_cap: 100.0, workers: 1, optimal_workers: 2");
    sim.state.nodes[1].inputs[0] = 100 * ONE;
    sim.run(19);
    assert_eq!(sim.state.nodes[1].outputs[0], 0);
    sim.run(1);
    assert_eq!(sim.state.nodes[1].outputs[0], ONE);
}

#[test]
fn full_output_buffer_blocks_and_keeps_inputs() {
    let mut sim = isolated_kiln("in_cap: 100.0, out_cap: 3.0, workers: 1, optimal_workers: 1");
    sim.state.nodes[1].inputs[0] = 20 * ONE;
    sim.run(100);
    let k = &sim.state.nodes[1];
    assert_eq!(k.outputs[0], 3 * ONE);
    assert_eq!(k.status, Status::Blocked);
    // Only 3 batches were started, so 14 stone remain.
    assert_eq!(k.inputs[0], 14 * ONE);
}

#[test]
fn empty_input_starves() {
    let mut sim = isolated_kiln("in_cap: 10.0, out_cap: 10.0, workers: 1, optimal_workers: 1");
    sim.run(5);
    assert_eq!(sim.state.nodes[1].status, Status::Starved);
}

#[test]
fn backpressure_propagates_upstream() {
    // Kiln off: its input fills, then the link stops, then the quarry blocks.
    let mut sim = chain(
        "",
        5.0,
        "in_cap: 4.0, out_cap: 10.0, workers: 1, optimal_workers: 1",
        "carriers: 2, capacity: 2.0, distance_secs: 0.5",
    );
    let k = id(&sim, "node.kiln.enabled");
    sim.command(Command::Set(k, 0));
    sim.run(200);
    assert_eq!(sim.state.nodes[1].status, Status::Off);
    assert_eq!(sim.state.nodes[1].inputs[0], 4 * ONE);
    assert_eq!(sim.state.nodes[0].status, Status::Blocked);
    assert_eq!(sim.state.nodes[0].outputs[0], 5 * ONE);
}

#[test]
fn unstaffed_and_off_states() {
    let mut sim = isolated_kiln("in_cap: 10.0, out_cap: 10.0, workers: 1, optimal_workers: 1");
    let w = id(&sim, "node.kiln.workers");
    sim.command(Command::Set(w, 0));
    sim.run(1);
    assert_eq!(sim.state.nodes[1].status, Status::Unstaffed);
    assert_eq!(sim.state.nodes[0].status, Status::Off);
}

#[test]
fn logistics_latency_and_carrier_ceiling() {
    // Quarry makes 1 unit/tick. One carrier, 1 unit per load, 5 ticks each way:
    // a round trip is 10 ticks, so 1 unit per 10 ticks arrives, first at tick 5.
    let mut sim = chain(
        "",
        100.0,
        "in_cap: 1000.0, out_cap: 10.0, workers: 1, optimal_workers: 1, enabled: false",
        "carriers: 1, capacity: 1.0, distance_secs: 0.5",
    );
    sim.run(5); // ticks 0..=4
    assert_eq!(sim.state.nodes[1].inputs[0], 0);
    assert_eq!(sim.state.links[0].in_transit(), ONE);
    sim.run(1); // tick 5: arrival
    assert_eq!(sim.state.nodes[1].inputs[0], ONE);
    sim.run(94); // through tick 99
    assert_eq!(sim.state.links[0].delivered_total, 10 * ONE);
}

#[test]
fn more_carriers_more_throughput() {
    let make = |carriers: u32| {
        let mut sim = chain(
            "",
            100.0,
            "in_cap: 1000.0, out_cap: 10.0, workers: 1, optimal_workers: 1, enabled: false",
            &format!("carriers: {carriers}, capacity: 1.0, distance_secs: 0.5"),
        );
        sim.run(500);
        sim.state.links[0].delivered_total
    };
    let one = make(1);
    let four = make(4);
    assert!(four >= one * 39 / 10, "1 carrier {one}, 4 carriers {four}");
}

#[test]
fn depleting_source_runs_dry() {
    let mut sim = chain(
        ", stock: Depleting(amount: 5.0)",
        100.0,
        "in_cap: 10.0, out_cap: 10.0, workers: 1, optimal_workers: 1, enabled: false",
        "carriers: 0, capacity: 1.0, distance_secs: 1.0",
    );
    sim.run(4);
    assert_eq!(sim.state.nodes[0].outputs[0], 4 * ONE);
    sim.run(10);
    assert_eq!(sim.state.nodes[0].outputs[0], 5 * ONE);
    assert_eq!(sim.state.nodes[0].stock, 0);
    assert_eq!(sim.state.nodes[0].status, Status::Starved);
}

#[test]
fn regrowing_source_recovers_and_needs_water() {
    let mut sim = chain(
        ", stock: Regrowing(standing: 10.0, max: 100.0, water_need: 1.0)",
        100.0,
        "in_cap: 10.0, out_cap: 10.0, workers: 1, optimal_workers: 1, enabled: false",
        "carriers: 0, capacity: 1.0, distance_secs: 1.0",
    );
    let q = id(&sim, "node.quarry.enabled");
    sim.command(Command::Set(q, 0)); // no harvesting, pure regrowth
    let mut dry = sim.clone();
    let water = id(&dry, "global.water");
    dry.command(Command::Set(water, 0));

    sim.run(600);
    dry.run(600);
    assert!(
        sim.state.nodes[0].stock > fx(10.0) * 3,
        "wet {}",
        sim.state.nodes[0].stock
    );
    assert_eq!(dry.state.nodes[0].stock, fx(10.0), "no water, no growth");
    assert!(sim.state.nodes[0].stock <= fx(100.0));
}

#[test]
fn determinism_same_seed_params_and_commands() {
    let cfg = chainlab_engine::scenarios::load("default").unwrap();
    let script = |sim: &mut Sim| {
        let rate = sim.params.id("node.quarry.rate").unwrap();
        let carriers = sim.params.id("link.0.carriers").unwrap();
        for t in 0..5_000u64 {
            match t {
                700 => sim.command(Command::Adjust {
                    id: rate,
                    steps: 3,
                    coarse: false,
                }),
                1_900 => sim.command(Command::Set(carriers, 5)),
                3_000 => sim.command(Command::Reseed(99)),
                _ => {}
            }
            sim.step();
        }
    };
    let mut a = Sim::new(&cfg, 1234).unwrap();
    let mut b = Sim::new(&cfg, 1234).unwrap();
    script(&mut a);
    script(&mut b);
    assert_eq!(a.state, b.state);
    assert_eq!(a.params, b.params);

    let mut c = Sim::new(&cfg, 4321).unwrap();
    script(&mut c);
    assert_ne!(a.state, c.state, "a different seed should change yields");
}

#[test]
fn bad_config_is_rejected() {
    let text = r#"(name: "Bad", resources: [], nodes: [
        (id: "q", name: "Q", kind: Source(output: "nothing", rate: 1.0))])"#;
    let cfg = parse_chain(text).unwrap();
    assert!(Sim::new(&cfg, 0).is_err());
}
