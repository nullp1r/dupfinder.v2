use std::{env, io};

use stdx::term;

use self::state::State;

mod state;

fn main() -> io::Result<()> {
  let root = env::args_os().nth(1).unwrap_or_else(|| ".".into());
  let stdout = io::stdout();
  let w = stdout.lock();

  term::ansi::enable();

  State::new(w, root).run()
}
