// SPDX-License-Identifier: MIT
//! `docbrain-verify` — the standalone, dependency-free, OFFLINE verifier for
//! DocBrain `.dbev` evidence bundles. This is the auditor-facing binary: no
//! server, no API key, no config file, no network. It reads a file, runs the
//! MIT trust core's `verify_bundle`, prints the report, and exits with the
//! verdict as its process code.
//!
//! Exit codes: 0 VALID, 1 TAMPERED, 2 CANNOT_VERIFY (straight from
//! `Verdict::exit_code`), 3 a CLI-level error (file missing/unreadable) —
//! which is NOT a verdict, just a failure to run the check at all.

use clap::Parser;
use docbrain_evidence::verify_bundle;
use std::io::Write;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "docbrain-verify",
    about = "Offline verifier for DocBrain .dbev evidence bundles",
    version
)]
struct Args {
    /// Path to the `.dbev` bundle to verify.
    bundle: PathBuf,
    /// Emit the machine-readable verdict JSON instead of the human report.
    #[arg(long)]
    json: bool,
}

fn main() {
    let args = Args::parse();

    let bytes = match std::fs::read(&args.bundle) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: cannot read bundle {}: {e}", args.bundle.display());
            std::process::exit(3);
        }
    };

    let report = verify_bundle(&bytes);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report.to_json()).unwrap_or_default());
    } else {
        print!("{}", report.render_human());
    }

    // `process::exit` does not flush a block-buffered stdout; do it explicitly.
    std::io::stdout().flush().ok();
    std::process::exit(report.verdict.exit_code());
}
