use std::{env, io};

use self::state::State;
use self::stdx::term;

mod state;
mod stdx;

fn main() -> io::Result<()> {
  let root = env::args_os().nth(1).unwrap_or_else(|| ".".into());
  let stdout = io::stdout();
  let w = stdout.lock();

  term::ansi::enable();

  State::new(w, root).run()
}
