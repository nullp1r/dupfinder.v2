use std::{env, io};

use self::state::State;
use self::term::ansi;

mod state;
mod term;

fn main() -> io::Result<()> {
  let root = env::args_os().nth(1).unwrap_or_else(|| ".".into());
  let stdout = io::stdout().lock();
  ansi::enable();

  State::new(stdout, root).run()
}
