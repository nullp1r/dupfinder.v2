# dupfinder

A simple CLI tool to quickly find file duplicates.

Parallel processing, custom high-performance hash function, and a three-stage process to minimize disk I/O (only reads the full file if the prefix and suffix hashes match). Results are displayed in a user-friendly format with progress bars and a summary table.

A rewrite from scratch of one of my oldest Rust projects, now with better UX and performance.

## Usage

```bash
cargo run --release -- /path/to/search
```

### Example Output

```text
files found: 31 (315 MiB)

computing 29 prefix hashes… (4 threads)
██████████████████████████████████████████████████ 100% (29 / 29)

computing 8 suffix hashes… (4 threads)
██████████████████████████████████████████████████ 100% (8 / 8)

computing 8 full hashes… (4 threads)
██████████████████████████████████████████████████ 100% (8 / 8)

b982b5aaf2aba30e · 4 files · 15.0 MiB each · 45.0 MiB duplicated
movies/vacation-2018.mp4
backup/old/vacation-2018.mp4
archive/2018/vacation-2018.mp4
and 1 more…

bc03fe831346d06d · 2 files · 50.0 MiB each · 50.0 MiB duplicated
iso/ubuntu-22.04-desktop-amd64.iso
downloads/ubuntu-22.04-desktop-amd64.iso

┌────────────┬─────────────────────────────────┬────────────────┐
│   total    │  unique and potentially unique  │   duplicates   │
├────────────┼────────────────┬────────────────┼────────────────┤
│   31 files │   25 files 81% │    2 files  6% │    4 files 13% │
│  315 MiB   │  155 MiB   49% │ 65.0 MiB   21% │ 95.0 MiB   30% │
└────────────┴────────────────┴────────────────┴────────────────┘

skipped 2 files (15.0 MiB)
computed 29 prefix hashes in 484 µs (59907 h/s · 234 MiB/s · 116 KiB)
computed 8 suffix hashes in 240 µs (33319 h/s · 130 MiB/s · 32.0 KiB)
computed 8 full hashes in 14.2 ms (563 h/s · 13.7 GiB/s · 200 MiB)
```
