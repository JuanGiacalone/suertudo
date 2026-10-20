//! Architecture test: the layering rule is enforced, not documented.
//!
//! `domain/` and `ports/` must not depend on `iii_sdk` or `sqlx`. That
//! boundary is what makes the business rules testable without infrastructure
//! and what keeps the backend portable if the orchestrator changes
//! (`docs/architecture.md` ADR-002).
//!
//! A rule that only lives in a document is a rule that decays. This test
//! fails the build when the boundary is crossed.

// `expect` is the right tool in a test: a failing assertion should stop the
// run with a readable message. The lint exists to keep panics out of the
// production paths, not out of the tests.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};

/// Crates that are allowed to depend on the outside world.
const INFRA_CRATES: &[&str] = &["iii_sdk", "sqlx", "tokio", "uuid", "chrono", "serde"];

#[test]
fn domain_and_ports_do_not_depend_on_infrastructure() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the repository root must exist");

    let services = repo_root.join("services");
    let mut checked = 0;

    for service in fs::read_dir(&services).expect("services/ must exist") {
        let service = service.expect("readable service entry").path();
        if !service.is_dir() {
            continue;
        }

        let mut guarded: Vec<PathBuf> = Vec::new();
        let domain = service.join("src/domain");
        if domain.is_dir() {
            collect_rust_files(&domain, &mut guarded);
        }
        let ports = service.join("src/ports.rs");
        if ports.is_file() {
            guarded.push(ports);
        }

        for file in guarded {
            let source = fs::read_to_string(&file).expect("readable source file");
            for crate_name in INFRA_CRATES {
                assert!(
                    !source.contains(&format!("use {crate_name}")),
                    "{} must not depend on `{crate_name}`.\n\
                     The domain takes the outside world through the traits in \
                     `src/ports.rs`.\nSee AGENTS.md §5.1 and docs/architecture.md ADR-002.",
                    file.strip_prefix(&repo_root).unwrap_or(&file).display(),
                );
            }
            checked += 1;
        }
    }

    assert!(
        checked > 0,
        "no domain/ or ports.rs file was found: the test would pass vacuously"
    );
}

#[test]
fn every_service_has_the_standard_layout() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the repository root must exist");

    let services = repo_root.join("services");
    let mut services_checked = 0;

    for service in fs::read_dir(&services).expect("services/ must exist") {
        let service = service.expect("readable service entry").path();
        if !service.is_dir() {
            continue;
        }
        let name = service
            .file_name()
            .expect("service directory has a name")
            .to_string_lossy()
            .into_owned();

        // Every service exposes a library.
        assert!(
            service.join("src/lib.rs").is_file(),
            "service `{name}` is missing `src/lib.rs`."
        );

        // A service that is a worker also has an entry point and a surface.
        if service.join("src/main.rs").is_file() {
            assert!(
                service.join("src/handlers.rs").is_file(),
                "service `{name}` is a worker but is missing `src/handlers.rs`.\n\
                 Every worker has the same shape: see AGENTS.md §5.1."
            );
        }

        services_checked += 1;
    }

    assert!(services_checked > 0, "no service was found");
}

fn collect_rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}
