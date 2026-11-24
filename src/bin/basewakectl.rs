//! Console front end: `basewakectl on|off|toggle|status|scan|install|uninstall`.

use clap::Parser;
use basewake::cli::{self, Cli};

fn main() {
    let cli = Cli::parse();
    let _log = basewake::engine::log::init(true);
    let Some(cmd) = cli.command else {
        eprintln!("usage: basewakectl <on|off|toggle|status|scan|install|uninstall>");
        std::process::exit(2);
    };
    let (code, text) = cli::run(cmd, !cli.local);
    print!("{text}");
    std::process::exit(code);
}
