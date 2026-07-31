# Cargo/Clippy protocol captures

These corpora were captured on Linux x86_64 with Cargo/rustc 1.97.1 and Clippy
0.1.97 by running this command from each fixture workspace:

```text
cargo clippy --workspace --all-targets --no-deps --message-format=json
```

Each `<fixture>.jsonl` file is the complete stdout stream. The matching
`<fixture>.exit-code` records the process status, while the final
`build-finished` object records Cargo's structured completion signal. The
fixture workspace's absolute canonical path was replaced with `<workspace>`;
no other field was rewritten.

`synthetic-noise.jsonl` appends one non-JSON tool-output line to a valid Cargo
message. The parser contract counts that line as tolerated noise, not as a
diagnostic or malformed Cargo JSON.
