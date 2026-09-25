# Clippy's correctness group on the pinned and minimum toolchains

Frozen record of US-001 (`tasks/prd-react-doctor-parity.md`), measured on
2026-09-25. It retires the barrier that kept deny-by-default lints out of the
candidate queue, and it classifies every member of `clippy::correctness` for
the pack US-003 admits.

## Flag order

The scan passes `-A clippy::all` first and then one `-W` per catalogued Clippy
rule. A scratch crate outside this repository, holding
`pub fn f() { let a = 1; if a == a {} }`, was checked with
`cargo +<toolchain> clippy --message-format=json -- <flags>`:

| Toolchain | Flags | `clippy::eq_op` | `build-finished.success` |
|---|---|---|---|
| 1.97.1 | `-A clippy::all -W clippy::eq_op` | warning | true |
| 1.97.1 | `-A clippy::all` | not reported | true |
| 1.97.1 | none | error | false |
| 1.95.0 | `-A clippy::all -W clippy::eq_op` | warning | true |
| 1.95.0 | `-A clippy::all` | not reported | true |
| 1.95.0 | none | error | false |

The trailing `-W` wins over the lint's default `deny`, and dropping it leaves
the lint allowed by `-A clippy::all`, not denied. A deny-by-default lint can
therefore be carried as a warning and switched off like any other rule. The
rejection class `deny-by-default` ("dropping its -W restores Clippy's refusal")
described the invocation before `-A clippy::all` existed, and no longer holds.

## Membership

`clippy-driver -W help` lists 67 members on 1.97.1 and 68 on 1.95.0. The
extra one on 1.95.0 is `clippy::overly_complex_bool_expr`, which 1.97 moved to
`pedantic`. It fires on 1.95.0 and is outside the 1.97.1 group, so the pack
does not carry it.

## Classification

Every member was given one minimal trigger using `std` alone, and the whole set
was checked in one run with `-A clippy::all` followed by a `-W` for each member.
A member is `std` when its trigger fires exactly once, on its own function, and
on no other member's. The run was repeated on 1.95.0 with the same result for
every `std` member, so no member is `toolchain-gated` and the pack needs no
toolchain above the declared minimum.

The 63 source triggers are kept as the positives of
`tests/fixtures/score-credibility/packs/correctness/src/lib.rs`, and the manifest
trigger as `tests/fixtures/rule-admission/lint-groups-priority/Cargo.toml`.

| Member | Class | Note |
|---|---|---|
| `clippy::absurd_extreme_comparisons` | std |  |
| `clippy::almost_swapped` | std |  |
| `clippy::approx_constant` | std |  |
| `clippy::async_yields_async` | std |  |
| `clippy::bad_bit_mask` | std |  |
| `clippy::cast_slice_different_sizes` | std |  |
| `clippy::char_indices_as_byte_indices` | std |  |
| `clippy::deprecated_semver` | std |  |
| `clippy::derive_ord_xor_partial_ord` | std |  |
| `clippy::derived_hash_with_manual_eq` | std |  |
| `clippy::eager_transmute` | std |  |
| `clippy::enum_clike_unportable_variant` | std |  |
| `clippy::eq_op` | std |  |
| `clippy::erasing_op` | std |  |
| `clippy::if_let_mutex` | std | fires only before edition 2024, whose if-let rescoping removed the deadlock; the pack crate is edition 2021 |
| `clippy::ifs_same_cond` | std |  |
| `clippy::impl_hash_borrow_with_str_and_bytes` | std |  |
| `clippy::impossible_comparisons` | std |  |
| `clippy::ineffective_bit_mask` | std |  |
| `clippy::infinite_iter` | std |  |
| `clippy::inherent_to_string_shadow_display` | std |  |
| `clippy::inline_fn_without_body` | std |  |
| `clippy::invalid_regex` | needs-crate:regex | needs a `Regex::new` call |
| `clippy::inverted_saturating_sub` | std |  |
| `clippy::invisible_characters` | std |  |
| `clippy::iter_next_loop` | std |  |
| `clippy::iter_skip_zero` | std |  |
| `clippy::iterator_step_by_zero` | std |  |
| `clippy::let_underscore_lock` | needs-crate:parking_lot | the `std` form is refused first by rustc's own deny-by-default `let_underscore_lock`; Clippy's lint only adds `parking_lot` locks |
| `clippy::lint_groups_priority` | std | fires on the `[lints]` table of `Cargo.toml`, not on source |
| `clippy::match_str_case_mismatch` | std |  |
| `clippy::mem_replace_with_uninit` | std |  |
| `clippy::min_max` | std |  |
| `clippy::mistyped_literal_suffixes` | std |  |
| `clippy::modulo_one` | std |  |
| `clippy::mut_from_ref` | std |  |
| `clippy::never_loop` | std |  |
| `clippy::non_octal_unix_permissions` | std |  |
| `clippy::nonsensical_open_options` | std |  |
| `clippy::not_unsafe_ptr_arg_deref` | std |  |
| `clippy::option_env_unwrap` | std |  |
| `clippy::out_of_bounds_indexing` | std |  |
| `clippy::panicking_overflow_checks` | std |  |
| `clippy::panicking_unwrap` | std |  |
| `clippy::possible_missing_comma` | std |  |
| `clippy::read_line_without_trim` | std |  |
| `clippy::recursive_format_impl` | std |  |
| `clippy::redundant_comparisons` | std |  |
| `clippy::reversed_empty_ranges` | std |  |
| `clippy::self_assignment` | std |  |
| `clippy::serde_api_misuse` | needs-crate:serde | needs a `serde::de::Visitor` implementation |
| `clippy::size_of_in_element_count` | std |  |
| `clippy::suspicious_splitn` | std |  |
| `clippy::transmute_null_to_fn` | std |  |
| `clippy::transmuting_null` | std |  |
| `clippy::uninit_assumed_init` | std |  |
| `clippy::uninit_vec` | std |  |
| `clippy::unit_cmp` | std |  |
| `clippy::unit_hash` | std |  |
| `clippy::unit_return_expecting_ord` | std |  |
| `clippy::unsound_collection_transmute` | std |  |
| `clippy::unused_io_amount` | std |  |
| `clippy::useless_attribute` | std |  |
| `clippy::vec_resize_to_zero` | std |  |
| `clippy::while_immutable_condition` | std |  |
| `clippy::wrong_transmute` | std |  |
| `clippy::zst_offset` | std |  |

Totals: 64 `std`, 3 `needs-crate`, 0 `untriggerable`, 0 `toolchain-gated`.
Tests never touch the network, so a `needs-crate` member cannot get a fixture
and stays in the candidate queue with its class quoted.
