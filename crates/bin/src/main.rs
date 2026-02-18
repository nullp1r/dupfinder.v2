use std::{env, io};

use stdx::term::ansi;

use self::state::State;

mod state;

fn main() -> io::Result<()> {
  let root = env::args_os().nth(1).unwrap_or_else(|| ".".into());
  let stdout = io::stdout().lock();
  ansi::enable();

  State::new(stdout, root).run()
}
