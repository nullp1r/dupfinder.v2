# dupfinder

A simple CLI tool to quickly find file duplicates.

Parallel processing, custom high-performance hash function, and a three-stage process to minimize disk I/O (only reads the full file if the prefix and suffix hashes match). Results are displayed in a user-friendly format with progress bars and a summary table.

A rewrite from scratch of one of my oldest Rust projects, now with better UX and performance.

## Usage

```bash
cargo run --release -- /path/to/search
```

### Example Output

![Example Output](assets/output.svg)
