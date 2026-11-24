//! Challenge completion logic.

use chainlab_engine::Sim;
use chainlab_engine::challenges::{ChallengeId, ChallengeRun, Outcome};
use chainlab_engine::fixed::fx;

fn drive(sim: &mut Sim, run: &mut ChallengeRun, ticks: u64) {
    for _ in 0..ticks {
        if !run.is_running() {
            break;
        }
        run.before_step(sim);
        sim.step();
        run.after_step(sim);
    }
}

#[test]
fn lean_chain_passes_with_default_crew_and_scores_workers() {
    let (mut sim, mut run) = ChallengeRun::start(ChallengeId::LeanChain, 3).unwrap();
    drive(&mut sim, &mut run, 6_000);
    match &run.outcome {
        Outcome::Passed { score, detail, .. } => {
            assert_eq!(detail, "10 workers");
            assert_eq!(*score, 52);
        }
        o => panic!("expected pass, got {o:?} / {}", run.status),
    }
}

#[test]
fn find_the_bottleneck_every_fault_hurts_and_repair_passes() {
    for seed in 0..12 {
        let (mut sim, mut run) = ChallengeRun::start(ChallengeId::FindBottleneck, seed).unwrap();
        assert!(run.hide_bottleneck());
        // Faults don't show as "changed".
        assert!(sim.params.list.iter().all(|p| !p.changed()));
        drive(&mut sim, &mut run, 1_800);
        assert!(
            run.is_running(),
            "seed {seed}: fault should keep progress low: {}",
            run.status
        );
        // Repair: restore every lever to the balanced scenario's values.
        let fresh = chainlab_engine::scenarios::load("balanced")
            .unwrap()
            .build(Some(seed))
            .unwrap();
        for (i, p) in fresh.params.list.iter().enumerate() {
            if sim.params.list[i].value != p.value {
                assert!(
                    run.lever_allowed(&p.key),
                    "fault lever {} must be editable",
                    p.key
                );
                sim.command(chainlab_engine::Command::Set(i, p.value));
            }
        }
        drive(&mut sim, &mut run, 3_000);
        assert!(
            matches!(run.outcome, Outcome::Passed { .. }),
            "seed {seed}: {}",
            run.status
        );
        assert!(!run.hide_bottleneck());
    }
}

#[test]
fn challenges_lock_capability_levers() {
    let (_, run) = ChallengeRun::start(ChallengeId::SteadyState, 1).unwrap();
    assert!(run.lever_allowed("node.kiln.workers"));
    assert!(run.lever_allowed("market.policy"));
    assert!(!run.lever_allowed("node.kiln.cycle"));
    assert!(!run.lever_allowed("market.bricks.price"));
    assert!(!run.lever_allowed("link.0.carriers"));
    let (_, sandbox) = ChallengeRun::start(ChallengeId::Sandbox, 1).unwrap();
    assert!(sandbox.lever_allowed("node.kiln.cycle"));
}

#[test]
fn market_shock_cuts_prices_and_fails_if_ignored() {
    let (mut sim, mut run) = ChallengeRun::start(ChallengeId::MarketShock, 5).unwrap();
    let base = sim.chain.market.as_ref().unwrap().goods[0].base_price;
    assert_eq!(sim.params.get(base), fx(4.0));
    drive(&mut sim, &mut run, 1_801);
    assert_eq!(sim.params.get(base), fx(2.4));
    assert!(sim.state.market[0].drift < fx(0.5));
    drive(&mut sim, &mut run, 10_000);
    assert!(
        matches!(run.outcome, Outcome::Failed { .. }),
        "{}",
        run.status
    );
}

#[test]
fn cost_wall_uses_steep_growth_and_fails_on_debt() {
    let (mut sim, mut run) = ChallengeRun::start(ChallengeId::CostWall, 2).unwrap();
    let u = sim.chain.upgrade_index("hire").unwrap();
    assert_eq!(sim.upgrade_cost(u), fx(20.0));
    sim.buy(u).unwrap();
    assert_eq!(sim.upgrade_cost(u), fx(30.0));
    // Blow the budget: a huge idle workforce drives coin negative.
    let pool = sim.chain.g.workers_total;
    sim.command(chainlab_engine::Command::Set(pool, 400));
    drive(&mut sim, &mut run, 6_000);
    match &run.outcome {
        Outcome::Failed { reason, .. } => assert!(reason.contains("debt")),
        o => panic!("expected debt failure, got {o:?}"),
    }
}

#[test]
fn steady_state_reports_progress() {
    let (mut sim, mut run) = ChallengeRun::start(ChallengeId::SteadyState, 1).unwrap();
    drive(&mut sim, &mut run, 1_200);
    assert!(run.is_running());
    assert!(run.status.contains("held"), "{}", run.status);
}
