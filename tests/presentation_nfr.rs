#![allow(unsafe_code)]
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use rust_doctor::presentation::ReportPresentation;
use rust_doctor::render::{TerminalOptions, render_terminal_with_presentation};
use rust_doctor::{
    Audit, BlockingLevel, Diagnostic, DiagnosticSource, GateReport, GateStatus, InspectReport,
    ScanReport, Severity, Status, Summary, ToolchainReport,
};

const DIAGNOSTICS: usize = 10_000;
const HANDOFF_BYTES: usize = 12 * 1024;
const MEMORY_LIMIT: usize = 32 * 1024 * 1024;

struct TrackingAllocator;

static CURRENT_BYTES: AtomicUsize = AtomicUsize::new(0);
static PEAK_BYTES: AtomicUsize = AtomicUsize::new(0);

#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator;

unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            record_allocation(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            record_allocation(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        CURRENT_BYTES.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let resized = unsafe { System.realloc(pointer, layout, new_size) };
        if !resized.is_null() {
            if new_size >= layout.size() {
                record_allocation(new_size - layout.size());
            } else {
                CURRENT_BYTES.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
        }
        resized
    }
}

fn record_allocation(bytes: usize) {
    let current = CURRENT_BYTES.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK_BYTES.fetch_max(current, Ordering::Relaxed);
}

fn reset_peak() -> usize {
    let current = CURRENT_BYTES.load(Ordering::Relaxed);
    PEAK_BYTES.store(current, Ordering::Relaxed);
    current
}

fn fixture_report() -> InspectReport {
    let diagnostics: Vec<_> = (0..DIAGNOSTICS)
        .map(|index| Diagnostic {
            id: format!("diagnostic-{index}"),
            source: DiagnosticSource::Clippy,
            code: Some(format!("clippy::rule_{}", index % 100)),
            base_severity: Severity::Warning,
            severity: Severity::Warning,
            category: Some("maintainability".to_owned()),
            message: "bounded diagnostic message".to_owned(),
            help: Some("bounded diagnostic help".to_owned()),
            package: None,
            target: None,
            path: None,
            span: None,
            occurrences: 1,
        })
        .collect();
    let summary = Summary::from_diagnostics(&diagnostics);
    InspectReport {
        schema_version: 9,
        audit: Audit::build(DIAGNOSTICS, Status::Complete, &diagnostics),
        status: Status::Complete,
        complete: true,
        policy: None,
        scope: None,
        project: None,
        toolchain: ToolchainReport {
            rustc: None,
            cargo: None,
            clippy: None,
        },
        scan: ScanReport {
            command: None,
            exit_code: Some(0),
            build_finished: Some(true),
            noise_lines: Some(0),
        },
        diagnostics,
        delta: None,
        errors: Vec::new(),
        summary,
        gate: GateReport {
            blocking: BlockingLevel::Error,
            status: GateStatus::Passed,
            blocking_diagnostics: Some(0),
        },
    }
}

fn presentation_pipeline(report: &mut InspectReport) {
    report.audit = Audit::build(
        report.audit.source_files,
        report.status,
        &report.diagnostics,
    );
    let presentation = ReportPresentation::derive_terminal(report);
    render_terminal_with_presentation(
        report,
        &presentation,
        io::sink(),
        TerminalOptions {
            workspace_root: Path::new("."),
            elapsed: Duration::ZERO,
            verbose: false,
            width: 100,
            color: false,
            animate: false,
        },
    )
    .expect("static rendering should succeed");

    // Reserve the complete allowed handoff payload while the presentation is still live.
    let handoff_payload = vec![b'x'; HANDOFF_BYTES];
    black_box((&presentation, handoff_payload));
}

#[test]
fn presentation_pipeline_meets_latency_and_peak_memory_budgets() {
    let mut report = fixture_report();
    assert!(report.is_valid());

    for _ in 0..10 {
        presentation_pipeline(&mut report);
    }

    let baseline_bytes = reset_peak();
    let mut samples = Vec::with_capacity(100);
    for _ in 0..100 {
        let started = Instant::now();
        presentation_pipeline(&mut report);
        samples.push(started.elapsed());
    }
    samples.sort_unstable();

    let p95 = samples[94];
    assert!(p95 < Duration::from_millis(100), "p95 was {p95:?}");
    let peak_delta = PEAK_BYTES
        .load(Ordering::Relaxed)
        .saturating_sub(baseline_bytes);
    assert!(
        peak_delta < MEMORY_LIMIT,
        "additional peak memory was {peak_delta} bytes"
    );
}
