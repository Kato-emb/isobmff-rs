//! Tasks of this repository that are not part of building it, run as `cargo xtask <task>`
//!
//! `cargo xtask fixtures` regenerates the files under
//! `crates/isobmff/tests/fixtures/` with FFmpeg, then the files that are one
//! of them with a field of a box changed, and
//! `cargo xtask fixtures --help` lists what each holds and the command that
//! made it. The crate depends on no crate of this workspace, so a fixture
//! cannot come from this repository's own writer.

mod fixtures;

use std::env;
use std::process::ExitCode;

const USAGE: &str = "\
Usage: cargo xtask <task>

Tasks:
  fixtures   regenerate crates/isobmff/tests/fixtures/ (--help lists them)
";

fn main() -> ExitCode {
    let arguments: Vec<String> = env::args().skip(1).collect();
    match arguments
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["fixtures"] => fixtures::generate(),
        ["fixtures", "--help" | "-h"] => {
            print!("{}", fixtures::help());
            ExitCode::SUCCESS
        }
        ["--help" | "-h"] => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => {
            eprint!("{USAGE}");
            ExitCode::FAILURE
        }
    }
}
