//! `chainlab-gui`: CHAINLAB in a native window (use `--tui` for the terminal).
fn main() -> std::process::ExitCode {
    chainlab::cli::main(true)
}
