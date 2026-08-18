# Expert review of a flagged file

The catalog has 62 rules, and a real codebase has more problems than that. Once
a file is open because rust-doctor flagged a line in it, read the rest of it
against the list below and report what you find beside the catalogued finding,
marked as coming from review rather than from the tool.

Each entry is a pattern worth raising, not a rule the tool enforces.

## Error handling

Callers that match on error variants need typed enums through `thiserror`;
callers that only propagate are better served by `anyhow` with `.context()`. One
project routinely wants both, the library half typed and the binary half not.

- `Box<dyn Error>` in a library's public API, which costs every caller the type
- `.expect("failed")` and other messages that restate the failure instead of
  naming the invariant that was supposed to hold
- `let _ = fallible_call()`, a discarded error that should at least be logged
- `.map_err(|_| MyError::Something)`, which drops the source chain that
  `#[from]` or `#[source]` would keep
- an error both logged and propagated, which prints it once per stack frame

## Security

- an `unsafe` block with no `// SAFETY:` comment stating the invariant it relies on
- `std::slice::from_raw_parts` with a length derived from untrusted input
- arithmetic on external input without `checked_*` or `saturating_*`, since
  release builds wrap silently
- literals matching `sk-`, `AKIA`, `ghp_`, `-----BEGIN`, `password=`, `token=`
- `format!("SELECT ... {}", input)`, which parameterized queries exist to prevent
- `unbounded_channel()` fed by external input, which is an out-of-memory waiting
  for load

## Async, Tokio

- `std::thread::sleep()` inside an async fn, which blocks the whole runtime
- a `tokio::sync::Mutex` guard held across an `.await` that does I/O, and a
  `std::sync::Mutex` guard worked around to survive an `.await` at all
- a future in a `tokio::select!` branch that is not cancel safe, `write_all`
  being the classic partial write lost on cancellation
- `async-trait` on a trait that no longer needs it, one heap allocation per call
  since 1.75 stabilized async fn in traits
- CPU-bound work between awaits with no `spawn_blocking` or rayon bridge, which
  starves every other task on the worker

## Performance

- `.clone()` on a large heap type in a hot path where a reference would do
- `fn f(s: String)` where `fn f(s: &str)` compiles, forcing the caller to allocate
- `.collect::<Vec<_>>()` immediately iterated, which the lazy iterator already did
- `Arc<Mutex<T>>` around a counter, where an atomic or a channel fits the access
  pattern
- an expensive computation inside the lock scope rather than before it
- deep generics on a cold path, paid for in monomorphized code size

## API design

Every `pub` item is a semver commitment, so the review question is what the type
promises rather than what it does today.

- `pub` where `pub(crate)` is what the code means
- public fields, which freeze the layout that a constructor or builder would keep free
- a public enum without `#[non_exhaustive]`, where one new variant breaks callers
- a public type missing `Debug`, `Clone` or `PartialEq`, each absence a workaround
  downstream
- boolean parameters, `fn process(validate: bool, compress: bool)`, that enums
  would make readable at the call site
- adjacent `u64` parameters, `fn ship(user_id: u64, order_id: u64)`, that
  newtypes would stop callers from swapping
- a struct carrying more than ten fields across unrelated responsibilities
- a cache or buffer that grows from external input with no bound or eviction
