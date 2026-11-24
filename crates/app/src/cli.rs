//! Command-line handling shared by `chainlab` (terminal) and `chainlab-gui`
//! (native window).
//!
//! ```text
//! chainlab                                   interactive UI (default scenario)
//! chainlab --scenario volatile_market        interactive, another scenario
//! chainlab --headless --ticks 6000           text summary after N ticks
//! chainlab --headless --set node.kiln.cycle=2 --payback
//! chainlab --compare balanced my_save.ron --ticks 6000
//! chainlab --challenge lean_chain            jump straight into a challenge
//! ```

use std::collections::BTreeMap;
use std::process::ExitCode;

use chainlab_engine::challenges::{ChallengeId, ChallengeRun};
use chainlab_engine::config::{ChainConfig, SavedParams, ron_options};
use chainlab_engine::engine::{analysis, report};
use chainlab_engine::fixed::to_f64;
use chainlab_engine::scenarios;

use crate::ui;

struct Args {
    gui: bool,
    headless: bool,
    ticks: u64,
    scenario: String,
    seed: Option<u64>,
    sets: Vec<(String, f64)>,
    compare: Option<(String, String)>,
    challenge: Option<ChallengeId>,
    payback: bool,
    list: bool,
    snapshot: Option<(u16, u16, String)>,
}

fn usage() -> String {
    format!(
        "CHAINLAB: production-chain mechanics lab\n\n\
         usage: chainlab [options]\n\n\
         --scenario NAME|PATH   scenario or chain file (default: balanced)\n\
         --seed N|0xHEX         random seed (default: scenario's, else 42)\n\
         --set KEY=VALUE        set a lever before starting (repeatable)\n\
         --challenge NAME       start a challenge ({})\n\
         --gui / --tui          native window / terminal UI\n\
         --headless             no UI: run --ticks and print a summary\n\
         --ticks N              ticks for headless/compare (default 3000 = 5 min)\n\
         --payback              headless: include upgrade payback analysis\n\
         --compare A B          headless A/B: scenario names or saved .ron files\n\
         --list                 list scenarios, challenges and lever keys\n\n\
         scenarios: {}",
        ChallengeId::ALL
            .iter()
            .map(|c| challenge_key(*c))
            .collect::<Vec<_>>()
            .join(", "),
        scenarios::names().join(", ")
    )
}

fn challenge_key(c: ChallengeId) -> String {
    c.name().to_ascii_lowercase().replace(' ', "_")
}

fn parse_args(gui: bool) -> Result<Args, String> {
    let mut a = Args {
        gui,
        headless: false,
        ticks: 3_000,
        scenario: "balanced".into(),
        seed: None,
        sets: vec![],
        compare: None,
        challenge: None,
        payback: false,
        list: false,
        snapshot: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut val = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--headless" => a.headless = true,
            "--gui" => a.gui = true,
            "--tui" => a.gui = false,
            "--payback" => a.payback = true,
            "--list" => a.list = true,
            "--ticks" => {
                a.ticks = val("--ticks")?
                    .parse()
                    .map_err(|e| format!("--ticks: {e}"))?
            }
            "--scenario" => a.scenario = val("--scenario")?,
            "--seed" => a.seed = Some(parse_seed(&val("--seed")?)?),
            "--set" => {
                let kv = val("--set")?;
                let (k, v) = kv.split_once('=').ok_or("--set expects key=value")?;
                let v: f64 = v.parse().map_err(|e| format!("--set {k}: {e}"))?;
                a.sets.push((k.to_string(), v));
            }
            "--compare" => {
                let x = val("--compare")?;
                let y = val("--compare")?;
                a.compare = Some((x, y));
                a.headless = true;
            }
            "--challenge" => {
                let name = val("--challenge")?
                    .to_ascii_lowercase()
                    .replace([' ', '-'], "_");
                let id = ChallengeId::ALL
                    .into_iter()
                    .find(|c| challenge_key(*c) == name)
                    .ok_or_else(|| format!("unknown challenge '{name}'"))?;
                a.challenge = Some(id);
            }
            // Hidden: render one UI frame as text after --ticks.
            "--snapshot" => {
                let dims = val("--snapshot")?;
                let (w, h) = dims.split_once('x').ok_or("--snapshot WxH")?;
                let overlay = std::env::var("CHAINLAB_OVERLAY").unwrap_or_default();
                a.snapshot = Some((
                    w.parse().map_err(|_| "bad width")?,
                    h.parse().map_err(|_| "bad height")?,
                    overlay,
                ));
            }
            "-h" | "--help" => {
                println!("{}", usage());
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument '{other}'\n\n{}", usage())),
        }
    }
    Ok(a)
}

fn parse_seed(s: &str) -> Result<u64, String> {
    let r = match s.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16),
        None => s.parse(),
    };
    r.map_err(|e| format!("--seed: {e}"))
}

type Operand = (String, ChainConfig, BTreeMap<String, f64>, u64);

/// Resolve a compare operand: a saved params file, else a scenario name.
fn operand(spec: &str, seed: Option<u64>) -> Result<Operand, String> {
    if let Ok(text) = std::fs::read_to_string(spec)
        && let Ok(saved) = ron_options().from_str::<SavedParams>(&text)
    {
        let loaded = scenarios::load(&saved.scenario)?;
        return Ok((
            spec.to_string(),
            loaded.chain,
            saved.values,
            seed.unwrap_or(saved.seed),
        ));
    }
    let loaded = scenarios::load(spec)?;
    let sim = loaded.build(seed)?;
    Ok((
        spec.to_string(),
        loaded.chain,
        sim.params.to_human_map(),
        sim.seed,
    ))
}

/// Entry point for both binaries. `gui` picks the default front end.
pub fn main(gui: bool) -> ExitCode {
    match real_main(gui) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}

fn real_main(gui: bool) -> Result<(), String> {
    let args = parse_args(gui)?;

    if args.list {
        println!("SCENARIOS");
        for (id, name, desc) in scenarios::list() {
            println!("  {id:<20} {name}: {desc}");
        }
        println!("\nCHALLENGES");
        for c in ChallengeId::ALL {
            println!("  {:<20} {}", challenge_key(c), c.goal());
        }
        let sim = scenarios::load(&args.scenario)?.build(args.seed)?;
        println!("\nLEVERS ({})", args.scenario);
        for p in &sim.params.list {
            println!("  {:<34} {:>12}  {}", p.key, p.display(), p.label);
        }
        return Ok(());
    }

    if let Some((x, y)) = &args.compare {
        let (la, chain, va, seed) = operand(x, args.seed)?;
        let (lb, _, vb, _) = operand(y, args.seed)?;
        let a = analysis::run_headless(&chain, &va, seed, args.ticks, &la)?;
        let b = analysis::run_headless(&chain, &vb, seed, args.ticks, &lb)?;
        println!(
            "A/B compare: {} ticks ({:.1} simulated min), seed {seed}\n",
            args.ticks,
            args.ticks as f64 / 600.0
        );
        print!("{}", report::compare(&a, &b));
        return Ok(());
    }

    let loaded = scenarios::load(&args.scenario)?;
    let mut sim = loaded.build(args.seed)?;
    let sets: BTreeMap<String, f64> = args.sets.iter().cloned().collect();
    let unknown = sim.apply_values(&sets, false);
    if !unknown.is_empty() {
        return Err(format!(
            "unknown parameters: {} (see --list)",
            unknown.join(", ")
        ));
    }

    if let Some((w, h, overlay)) = &args.snapshot {
        if args.challenge.is_none() {
            sim.run(args.ticks);
        } else {
            sim.state.tick = args.ticks; // replayed inside the challenge
        }
        print!(
            "{}",
            ui::snapshot(loaded, sim, *w, *h, overlay, args.challenge)
        );
        return Ok(());
    }

    if !args.headless {
        if args.gui {
            return crate::gui::run(loaded, sim, args.challenge);
        }
        return ui::run(loaded, sim, args.challenge).map_err(|e| e.to_string());
    }

    if let Some(id) = args.challenge {
        let (mut sim, mut run) = ChallengeRun::start(id, args.seed.unwrap_or(42))?;
        for _ in 0..args.ticks {
            if !run.is_running() {
                break;
            }
            run.before_step(&mut sim);
            sim.step();
            run.after_step(&sim);
        }
        print!("{}", report::summary(&sim));
        println!("\nCHALLENGE {}: {}", id.name(), run.status);
        return Ok(());
    }

    sim.run(args.ticks);
    print!("{}", report::summary(&sim));
    if args.payback {
        println!("\nUPGRADE PAYBACK (fork + 2 simulated minutes each)");
        println!(
            "  {:<28} {:>4} {:>9} {:>9} {:>10} {:>9}",
            "upgrade", "lvl", "cost", "+prog/m", "+profit/m", "payback"
        );
        let mut list = analysis::payback(&sim, analysis::HORIZON);
        list.sort_by(|a, b| {
            a.payback_secs
                .unwrap_or(f64::INFINITY)
                .total_cmp(&b.payback_secs.unwrap_or(f64::INFINITY))
        });
        for p in list {
            let u = &sim.chain.upgrades[p.upgrade];
            println!(
                "  {:<28} {:>4} {:>9.1} {:>9.2} {:>10.2} {:>9}",
                u.name,
                sim.state.upgrades[p.upgrade],
                to_f64(p.cost),
                to_f64(p.d_progress),
                to_f64(p.d_profit),
                p.payback_secs
                    .map_or("never".into(), |s| format!("{s:.0}s"))
            );
        }
    }
    Ok(())
}
