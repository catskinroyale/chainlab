# CHAINLAB

A terminal **mechanics lab** for casual production-chain / supply-chain games:
*gather → process → haul → sell → spend → expand → repeat.*

It's not a game with graphics. It's a live, deterministic simulation drawn in
phosphor-green ASCII, where every number is a lever you can change while it runs.
Watch throughput, utilisation, the bottleneck and profit respond, then play the
challenges to test how well you understand the system.

```
┌Stone Quarry──┐      ╔Kiln BN═══════╗      ┌Tower Builder─┐
│[WORK]    2/2w│  ┌──>║[WORK] █░ 2/2w║ ┌───>│[WORK]    2/2w│
│out███████18.9│─o┼──>║in ██████░18.0║─┼───>│B█░░░░ P█████ │
│30.0/min      │  │   ║out█░░░░░░ 3.0║─·┐   │P 26.4 7.5/m  │
└──────────────┘  │   ╚══════════════╝ ││   └──────────────┘
┌Forest────────┐  │   ┌Sawmill───────┐ ││   ┌Market────────┐
│[BLOCKED] 2/2w│  │   │[WORK] █░ 2/2w│─┘│   │[STARVED]     │
│out██████░13.0│o─┼o─>│in █████░░11.0│─o┼──>│Bric 4.01- 0.0│
│std███████92.9│  │   │out░░░░░░░ 0.6│  └──>│Plan 2.79- 0.0│
└──────────────┘  │   └──────────────┘      └──────────────┘
```

## Running it

Requires stable Rust (built with 1.99). No network access or async runtime is
needed at run time.

```sh
cargo run --release                          # interactive, "Balanced" scenario
cargo run --release -- --scenario volatile_market
cargo run --release -- --challenge find_the_bottleneck
```

The terminal must be at least **80x24**; **100x35** or larger is best. At 80
columns the stats column narrows and the diagram switches to compact boxes.

### Native window

The same app, same screens and same keys, in its own desktop window
(eframe/egui), no terminal needed:

```sh
cargo run --release --bin chainlab-gui
cargo run --release --bin chainlab-gui -- --challenge lean_chain   # all CLI options work
```

The window renders the ratatui UI into an off-screen character grid and
paints it with a monospace font, so it looks exactly like the terminal
version. Resize the window for more room; **Cmd/Ctrl + / -** zooms. `chainlab --gui`
and `chainlab-gui --tui` switch front ends.

### Headless mode

```sh
# 5 simulated minutes, then a plain-text report
cargo run --release -- --headless --ticks 3000

# change levers first, and include the upgrade payback analysis
cargo run --release -- --headless --ticks 3000 --set node.kiln.cycle=2 --set node.tower.lift=0.4 --payback

# A/B compare two scenarios or saved lever files (same seed, fresh start)
cargo run --release -- --compare balanced expensive_logistics --ticks 6000
cargo run --release -- --compare balanced chainlab_params.ron

# run a challenge with no player input (shows what "doing nothing" scores)
cargo run --release -- --headless --challenge market_shock --ticks 6000

# list scenarios, challenges and every lever key
cargo run --release -- --list
```

| option | meaning |
|---|---|
| `--scenario NAME\|PATH` | built-in scenario, or a scenario/chain `.ron` file |
| `--seed N\|0xHEX` | random seed (default: the scenario's, else 42) |
| `--set KEY=VALUE` | set a lever in human units, repeatable (`--list` shows keys) |
| `--challenge NAME` | `steady_state`, `find_the_bottleneck`, `lean_chain`, `market_shock`, `cost_wall` |
| `--headless --ticks N` | run N ticks (10 per simulated second) and print the report |
| `--payback` | add the forked-sim payback table to the headless report |
| `--compare A B` | headless A/B comparison |

## Key bindings

| key | action |
|---|---|
| `space` | pause / resume |
| `.` / `>` | step one tick / one second (pauses) |
| `-` `=` or `1`..`6` | speed 0.5x, 1x, 2x, 5x, 20x, max |
| `↑` `↓` | select lever |
| `←` `→` (`shift` = coarse), `[` `]`, `{` `}` | adjust lever |
| `tab` / `shift-tab` | next / previous lever group (each node, each haul link, Market, Costs, Global, Events) |
| `r` / `R` | reset selected lever / reset all levers |
| `n` | jump to the bottleneck's lever group |
| `s` (or `b`) | **shop**: upgrades, cost curves, payback; `enter` buys |
| `m` | market detail: prices, history, demand curve |
| `c` | challenges and scenarios menu |
| `a` | A/B compare (`A`/`B` store current levers, `d` A = defaults, `f` B = save file, `←→` minutes, `enter` run) |
| `x` | retry the challenge, or restart the sandbox keeping levers |
| `w` / `o` | save / load levers to `chainlab_params.ron` |
| `k` | new seed (reseeds the random streams from now) |
| `e` | random events on/off |
| `t` | toggle warning colours (amber/red) |
| `?` | help: one page per mechanic |
| `q`, `ctrl-c` | quit |

In the lever panel `*` marks a lever changed from its default and `L` marks
one locked by the current challenge. The *effective* column shows the value
the sim actually uses after upgrades, events, tier locks and the worker pool.

## Screen

- **Top bar**: simulated time, speed or PAUSED, seed, scenario, challenge progress, measured ticks/s.
- **Flow diagram**: nodes in columns by depth. Box border style shows status.
  The bottleneck gets a `╔═╗` double border with `BN` in the title.
  Inside each box: status tag (`[WORK]` `[STARVED]` `[BLOCKED]` `[OFF]` `[NOCREW]`
  `[LOCKED]` `[BROKEN]` `[HOLDING]`), crew `placed/optimal w`, cycle
  progress, and buffer tanks `████░░`. On links: `o` = loaded cart, `·` = empty
  cart returning, `O` = cart waiting to unload, `>` = destination.
  The panel footer explains the bottleneck with evidence.
- **Stats**: coin with sparkline, efficiency score and its parts, income /
  expenses / net per minute, cumulative profit, progress, next gate with ETA,
  per-node throughput sparkline and stacked utilisation bar
  (`█` work `▒` starved `▓` blocked `░` no crew `·` off), market prices.
- **Levers**, **Log** (purchases, events, unlocks, lever changes), **footer** (key hints and messages).

## The default chain

```
Branch A:  Stone Quarry --haul--> Kiln ---haul--+--> Tower Builder  (Progress)
Branch B:  Forest ------haul--> Sawmill --haul--+
Surplus:   Kiln, Sawmill --haul--> Market  (Coin)
Tier 2:    Deep Quarry --haul--> Kiln
```

The Tower consumes **sets** (2 Bricks + 1 Plank) to make Progress, limited by
its lift. Links leaving the same node are served in config order, so the Tower
gets first pick and only surplus reaches the Market. That's why *Progress vs
Profit* is a real trade-off: a faster Kiln adds progress but can *lower*
profit, because more planks go into the Tower instead of being sold.

## The mechanics, one page

Each mechanic is a separate engine module that starts with a plain-language
doc comment (`crates/engine/src/engine/*.rs`). The in-app help (`?`) has the
same pages.

| # | mechanic | how it works | levers that show it |
|---|---|---|---|
| 1 | **Raw sources** (`source.rs`) | yield/tick = rate × buildings × crew × efficiency × (1 ± variance). Full output buffer → BLOCKED. Depleting deposits run dry. The forest regrows **logistically**, `r·water·S·(1−S/K)`: fastest at half stock, so over-harvesting is punished twice. Max sustainable harvest is `r·K/4`. | Forest *harvest rate* > 1.25/s; Global *water*, *regrowth*; scenario *Starved Forest* |
| 2 | **Processors** (`processor.rs`) | Start a cycle only if inputs hold a full batch **and** outputs have room. Short = STARVED, no room = BLOCKED (inputs kept). Progress per tick = crew factor; leftover progress carries over. | Kiln *cycle time*, *efficiency*, *buildings* |
| 3 | **Logistics** (`logistics.rs`) | Carts loop: load → travel → unload → return. Link max = carts × load / (2 × distance). The dispatcher ships only what fits at the destination, so a full receiver backs up into the sender. | any Haul group: *carriers*, *load/carrier*, *distance*; scenario *Expensive Logistics* |
| 4 | **Storage and caps** | Every node has input/output buffers. Buffers absorb bursts and breakdowns but can't raise long-run throughput. | Kiln *output cap* to 2; turn events on and compare buffer sizes |
| 5 | **Workers** (`workers.rs`) | Shared pool handed out upstream-first (over-assign and downstream nodes get NOCREW). The whole pool pays upkeep. Above the optimal crew, extra worker *k* adds `d^k`. | Global *worker pool*, *over-crew returns*, *upkeep*; node *workers*; challenge *Lean Chain* |
| 6 | **Market** (`market.rs`) | `price = base × drift / (1 + elasticity × pressure)`. Sales add pressure, which fades by *recovery*/s. Drift is a mean-reverting random walk. Policies: sell all, above threshold (HOLDING otherwise), keep reserve. | Market group; `m` view; scenario *Volatile Market*; challenge *Market Shock* |
| 7 | **Escalating cost** (`economy.rs`) | `cost(n) = base × growth^n × (1 − discount)`. The shop charts the next 12 levels against your coin. | Costs group: per-upgrade *growth*, *base cost*, *bulk discount*; challenge *Cost Wall* |
| 8 | **Upgrades** (`economy.rs`, `mod.rs`) | Each level applies `Add`/`Mul` effects to named levers through the *effective* layer, so the lever's own value and default stay visible. Purchases are logged. | shop (`s`) |
| 9 | **Gates** (`gates.rs`) | Milestones in order raise the tier. Tier 1 (30 progress) unlocks lift and stall upgrades. Tier 2 (600 profit) unlocks the Deep Quarry node (assign it workers!) and glazing. | stats panel "next" line with ETA |
| 10 | **Time and events** (`events.rs`) | 10 ticks per simulated second, fixed order. Events: drought, price crash, breakdown, windfall, at *frequency*/min and *severity*%. | `e`; Events group |

### Statistics

- **Throughput**: output over the last 60 simulated seconds, per minute.
- **Utilisation**: share of those 600 ticks in each status.
- **Bottleneck** (`scoring.rs`): each stage's *load* = actual ÷ theoretical
  capacity. For links, load = goods kept on the road ÷ (carts × load / 2), so
  half-empty carts count half and carts waiting at a full receiver don't count.
  Start at the Tower, find its scarcest input (fewest complete sets in buffer +
  in transit), and walk upstream along it. The highest-load stage wins, ties go
  to the stage nearest the Tower, and nothing is reported below 85%.
- **Payback** (`analysis.rs`): fork the sim, grant the upgrade for free, run both
  futures for 2 simulated minutes with the *same random stream*, and compare.
  `payback = cost ÷ extra operating profit`. An upgrade on a non-bottleneck
  shows roughly zero gain.
- **Efficiency score**: `100 × (0.4·utilisation + 0.3·workforce + 0.3·margin)`.
  Utilisation is the mean Working share of production nodes. Workforce is the
  share of the pool on nodes that are actually working. Margin is
  (income − upkeep) ÷ income.

## Challenges

There is no win state. Challenges are optional goals that score your tuning
from 0 to 100, and `x` retries instantly. They lock the "free capability"
levers (rates, cycle times, prices, carts), so you win the way a player
would: by assigning workers, switching nodes, sizing buffers, choosing a sell
policy and buying upgrades.

| challenge | goal |
|---|---|
| Steady State | operating profit ≥ 35/min with no production node starved/blocked > 10%, held for 2 min (score: speed) |
| Find the Bottleneck | one lever is secretly mis-tuned (seed-chosen, no "changed" marker) and the bottleneck marker is hidden. Get progress back to ≥ 7/min for 45 s, counted from 3:00 (score: speed). Node and link levers are unlocked |
| Lean Chain | progress ≥ 7/min for 60 s using the fewest workers (score: workers) |
| Market Shock | at 3:00 prices crash and base prices drop 40% for good. Get operating profit back to 80% of pre-crash within 5 min (score: speed) |
| Cost Wall | upgrade growth x1.5: own 15 upgrade levels within 15 min without more than 10 s in debt (score: speed) |
| Sandbox | no goals, nothing locked |

## Scenarios, saving and A/B

Built-in scenarios: **Balanced**, **Starved Forest**, **Expensive Logistics**,
**Volatile Market** (`data/scenarios/*.ron`, embedded in the binary; files on
disk take precedence). A scenario names a chain and a set of lever overrides.
The overrides become the levers' defaults, so `*` and reset are relative to the
scenario.

```ron
(
    name: "Starved Forest",
    description: "Low water and slow regrowth: harvest too hard and the forest collapses.",
    chain: "default",
    overrides: { "global.water": 0.35, "global.regrowth": 0.03, "node.forest.rate": 0.9 },
)
```

`w` saves every lever to `chainlab_params.ron` and `o` loads it. The A/B view
(`a`) fast-forwards two lever sets from tick 0 with the same seed on a
background thread and shows key stats side by side, with the better value
marked. Upgrades aren't carried into A/B runs; compare lever settings.

## Adding a node or recipe (no recompiling)

Chains are RON files: resources, nodes, links, market goods, upgrades, gates
and events. Values use human units (units, seconds, percent, coin).
`data/examples/pottery.ron` is a complete small chain:

```sh
cargo run --release -- --scenario data/examples/pottery.ron
```

To add, say, a **Glassworks** to the default chain, edit `data/chain_default.ron`.
The file on disk overrides the embedded copy when you run from the repo root.

1. Declare the new resources:
   ```ron
   (id: "sand", name: "Sand"), (id: "glass", name: "Glass"),
   ```
2. Add a source and a processor. `tier: 1` keeps them locked until the first gate:
   ```ron
   (id: "dunes", name: "Sand Dunes", kind: Source(output: "sand", rate: 1.0),
    out_cap: 20.0, workers: 0, optimal_workers: 2, tier: 1),
   (id: "glassworks", name: "Glassworks",
    kind: Processor(inputs: [("sand", 2.0)], outputs: [("glass", 1.0)], cycle_secs: 6.0),
    in_cap: 20.0, out_cap: 20.0, workers: 0, optimal_workers: 2, tier: 1),
   ```
3. Link them, and sell the output (or add it to the Tower's `Sink` inputs):
   ```ron
   (from: "dunes", to: "glassworks", resource: "sand", carriers: 2, capacity: 4.0, distance_secs: 4.0),
   (from: "glassworks", to: "market", resource: "glass", carriers: 1, capacity: 4.0, distance_secs: 5.0),
   ```
   and in `market.goods`: `(resource: "glass", base_price: 8.0, elasticity: 0.03, recovery: 0.05)`.
4. Optionally add an upgrade. Effects name any lever key (`--list` prints them all):
   ```ron
   (id: "glass_speed", name: "Glassworks: hotter furnace", base_cost: 80.0, growth: 1.2, tier: 1,
    effects: [Mul("node.glassworks.cycle", 0.9)]),
   ```

New links get the next index (`link.7.*`, ...). Node kinds are `Source` (with
`stock: Infinite | Depleting(amount) | Regrowing(standing, max, water_need)`),
`Processor` (recipe), `Sink` (sets → Progress, with `lift_rate`) and `Market`
(at most one). Gate conditions are `Progress(x)`, `Coin(x)`, `Profit(x)` and
`Produced("node_id", x)`. The diagram lays out any acyclic chain automatically.

## Architecture

```
crates/engine   chainlab_engine: pure library, depends only on serde + ron
  config/         RON schema (chains, scenarios, saved levers)
  engine/         Sim, tick loop, one module per mechanic, stats, scoring, analysis, report
  scenarios/      built-in + on-disk scenario loading
  challenges/     challenge definitions, lever locks, scoring
  fixed.rs rng.rs fixed-point maths, deterministic RNG
crates/app      chainlab + chainlab-gui binaries (thin layers)
  cli.rs          shared command line
  ui/             ratatui UI: theme, widgets, flow diagram, panels, overlays, worker
  gui/            native window: draws the ui/ screens into an egui window
data/           default chain, scenarios, example chain
```

The UI only reads `Sim` state and sends `Command`s (`Set`, `Adjust`, `Reset`,
`Buy`, `Reseed`, `Trigger`...). Payback analysis and A/B runs clone the sim
onto a worker thread.

### Numbers and determinism

- All simulation **state** is `i64` fixed point (1 unit = 1000), so arithmetic
  is exact and platform-independent.
- Non-linear formulas (cost growth, demand curve, regrowth, diminishing
  returns) are evaluated in `f64` as pure functions of the integer state and
  rounded straight back. Nothing fractional is carried between ticks, so there
  is no drift. Upkeep keeps its sub-milli remainder in an integer accumulator.
- Iteration order is fixed (indexed `Vec`s in topological order, no hash-map
  iteration).
- Randomness is a built-in xoshiro256** generator with one stream per
  subsystem (yields, market, events). Each stream draws a fixed number of
  values per tick, so toggling one thing doesn't shift another's sequence.
- Lever changes and purchases are applied between ticks, so
  `(chain, seed, [(tick, command)])` reproduces a run exactly. A test checks
  this, and the simulation speed setting can't affect results.

## Tests

```sh
cargo test                  # engine + UI tests (headless)
cargo clippy --all-targets -- -D warnings
```

Coverage includes recipe conversion, efficiency and crew speed, buffer blocking
and starving, backpressure, logistics latency and cart ceilings, depletion and
regrowth, worker-pool clipping, exact upkeep, the escalating-cost formula,
market demand, recovery and sell policies, purchases and effects, gates,
events, bottleneck detection as levers move, payback ranking, determinism with
events and purchases, every challenge's pass and fail logic, scenario loading,
save/load round-trips, and rendering every screen at 80x24, 100x35 and 160x50.
