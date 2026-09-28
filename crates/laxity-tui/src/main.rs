//! the viewer's own binary, which is a shim over the library so that laxity-tui and laxity run the same code.

fn main() -> std::process::ExitCode {
    laxity_tui::run_cli(std::env::args().skip(1).collect())
}
