//! CHAINLAB binary: TUI front end (milestone 4) and `--headless` mode.

use std::process::ExitCode;

use chainlab_engine::{Sim, engine::report, scenarios};

struct Args {
    headless: bool,
    ticks: u64,
    scenario: String,
    seed: u64,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        headless: false,
        ticks: 3_000,
        scenario: "default".into(),
        seed: 42,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut val = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--headless" => a.headless = true,
            "--ticks" => {
                a.ticks = val("--ticks")?
                    .parse()
                    .map_err(|e| format!("--ticks: {e}"))?
            }
            "--scenario" => a.scenario = val("--scenario")?,
            "--seed" => a.seed = parse_seed(&val("--seed")?)?,
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

fn usage() -> String {
    format!(
        "chainlab [--headless] [--ticks N] [--scenario NAME|PATH] [--seed N|0xHEX]\n\
         scenarios: {}",
        scenarios::names().join(", ")
    )
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let cfg = match scenarios::load(&args.scenario) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };
    let mut sim = match Sim::new(&cfg, args.seed) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };
    if !args.headless {
        eprintln!("The interactive UI arrives in milestone 4. Running headless instead.\n");
    }
    sim.run(args.ticks);
    print!("{}", report::summary(&sim));
    ExitCode::SUCCESS
}
