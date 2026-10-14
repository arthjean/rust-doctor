# Contextual scope on this repository's history

Frozen record of US-004 (`tasks/prd-react-doctor-gap-closure.md`), measured on
2026-10-14 with toolchain 1.97.1 and the release binary of the uncommitted
EP-001 worktree, under the catalog of 133 rules and the schema 21 report. It
answers one question before any hook moves off `--scope lines`: how many
findings does `--scope contextual` add per commit, the legacy findings of a
touched function an agent would then be asked to fix.

## Method

The 50 last commits of `9b8c0ab` touching `src/` (`git log -n 50 -- src/`),
from `a195d9b` back to `389c5d9`, were checked out one at a time in a local
clone. Each was scanned three times against its first parent, with
`--json --yes --advisory-db /dev/null --base <parent>`: once with
`--scope lines` to warm the build, then `--scope lines` and
`--scope contextual` timed back to back, both warm. "Extra" is the number of
diagnostic identities the contextual report holds and the lines report does
not. Every one of the 100 timed scans completed.

## Result

Contextual returned no extra finding on any of the 50 commits. Lines found 87
findings in total, on 14 of them. The warm scans took 26,990 ms under lines and
26,699 ms under contextual, a ratio of 0.99; the median per-commit ratio is
1.00 and the largest 1.25, on `4ce38dad6` (475 ms against 593 ms), within the
spread of two identical lines runs elsewhere in the table.

What this does not show: this repository holds almost no asynchronous code,
so the four item-reach rules about `async` and locks have nothing to fire on,
and the SQL, shell and TLS rules fire only on fixtures. The record bounds the cost
of the scope, not the trap the PRD fears on an async service. That needs the
same replay on a corpus repository built on tokio before the hooks change.

## Per commit

| Commit | Lines findings | Contextual findings | Extra | Lines ms | Contextual ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| `a195d9b81` | 0 | 0 | 0 | 630 | 616 |
| `bcb2b4812` | 0 | 0 | 0 | 609 | 613 |
| `563141113` | 6 | 6 | 0 | 599 | 604 |
| `0cf20091a` | 0 | 0 | 0 | 593 | 590 |
| `51440b746` | 5 | 5 | 0 | 590 | 591 |
| `abf595c67` | 33 | 33 | 0 | 596 | 585 |
| `8cce13d68` | 1 | 1 | 0 | 579 | 559 |
| `66bffe947` | 8 | 8 | 0 | 546 | 557 |
| `d6e055f22` | 0 | 0 | 0 | 537 | 520 |
| `bc8f7dfb3` | 8 | 8 | 0 | 590 | 567 |
| `98576f6c5` | 0 | 0 | 0 | 518 | 521 |
| `d2aea1a38` | 11 | 11 | 0 | 529 | 534 |
| `df31c8d46` | 0 | 0 | 0 | 529 | 530 |
| `db882afa5` | 0 | 0 | 0 | 518 | 505 |
| `bfe1bf7e0` | 0 | 0 | 0 | 505 | 496 |
| `bc11dccb0` | 0 | 0 | 0 | 516 | 496 |
| `3f405bc9a` | 1 | 1 | 0 | 512 | 491 |
| `d8cfec227` | 0 | 0 | 0 | 473 | 468 |
| `fece80e41` | 0 | 0 | 0 | 464 | 458 |
| `bfb5db2ae` | 0 | 0 | 0 | 470 | 496 |
| `4ce38dad6` | 0 | 0 | 0 | 475 | 593 |
| `5189f3ff5` | 1 | 1 | 0 | 1226 | 838 |
| `71d4a88f4` | 7 | 7 | 0 | 447 | 463 |
| `90956b1b4` | 0 | 0 | 0 | 466 | 453 |
| `c6b89c504` | 0 | 0 | 0 | 454 | 448 |
| `a53166b8b` | 0 | 0 | 0 | 497 | 511 |
| `598a3f197` | 0 | 0 | 0 | 450 | 449 |
| `494a0dbb4` | 0 | 0 | 0 | 462 | 492 |
| `99fd664e4` | 0 | 0 | 0 | 461 | 492 |
| `4df5edc0e` | 0 | 0 | 0 | 454 | 460 |
| `4d09d3eca` | 2 | 2 | 0 | 448 | 462 |
| `956848026` | 0 | 0 | 0 | 460 | 456 |
| `7fa88e456` | 0 | 0 | 0 | 476 | 465 |
| `c4382f442` | 0 | 0 | 0 | 1130 | 1106 |
| `d29e85e35` | 2 | 2 | 0 | 1128 | 1099 |
| `f6e230c37` | 0 | 0 | 0 | 704 | 733 |
| `01b48b165` | 0 | 0 | 0 | 472 | 464 |
| `c94abe518` | 0 | 0 | 0 | 452 | 441 |
| `3a0e5eec1` | 0 | 0 | 0 | 456 | 463 |
| `20500ac19` | 0 | 0 | 0 | 448 | 450 |
| `54855f9f8` | 0 | 0 | 0 | 444 | 466 |
| `7515176d5` | 0 | 0 | 0 | 450 | 472 |
| `7ebac6931` | 0 | 0 | 0 | 456 | 456 |
| `3cf59de53` | 0 | 0 | 0 | 452 | 455 |
| `296cfd45a` | 0 | 0 | 0 | 460 | 443 |
| `58f9b1b5f` | 1 | 1 | 0 | 445 | 454 |
| `1ea323516` | 0 | 0 | 0 | 452 | 451 |
| `168c62194` | 1 | 1 | 0 | 454 | 457 |
| `114658bba` | 0 | 0 | 0 | 443 | 457 |
| `389c5d921` | 0 | 0 | 0 | 465 | 453 |
