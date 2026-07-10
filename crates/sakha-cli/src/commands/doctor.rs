//! `sakha doctor`: diagnoses the local Sakha installation — config file
//! readability, provider reachability/health, tool registry integrity, and
//! (if configured) database openability.

use serde::Serialize;

use sakha_tools::ToolName;

use crate::config::{default_config_path, load_config};
use crate::output::{print_output, OutputFormat};
use crate::runtime::{build_permission_policy, build_provider, build_tool_registry};

#[derive(Debug, Serialize)]
struct DoctorCheck {
    name: String,
    ok: bool,
    detail: String,
}

#[derive(Debug, Serialize)]
struct DoctorReport {
    checks: Vec<DoctorCheck>,
    all_ok: bool,
}

pub fn execute(output: OutputFormat) -> i32 {
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("failed to start async runtime: {err}");
            return 1;
        }
    };
    rt.block_on(execute_async(output))
}

async fn execute_async(output: OutputFormat) -> i32 {
    let mut checks = Vec::new();

    let config_path = default_config_path();
    let config = match load_config(&config_path) {
        Ok(c) => {
            checks.push(DoctorCheck {
                name: "config".into(),
                ok: true,
                detail: format!("loaded from {}", config_path.display()),
            });
            c
        }
        Err(err) => {
            checks.push(DoctorCheck { name: "config".into(), ok: false, detail: err.to_string() });
            crate::config::SakhaConfig::default()
        }
    };

    let provider = build_provider(&config);
    match provider.health().await {
        Ok(health) => {
            let ok = health != sakha_provider::ProviderHealth::Unavailable;
            checks.push(DoctorCheck {
                name: "provider".into(),
                ok,
                detail: format!("selection={:?} model={} health={health:?}", config.provider.selection, config.provider.model),
            });
        }
        Err(err) => {
            checks.push(DoctorCheck { name: "provider".into(), ok: false, detail: err.to_string() });
        }
    }

    let registry = build_tool_registry();
    let has_core_tools = registry.get(&ToolName::new("file.read")).is_some() && registry.get(&ToolName::new("shell.run")).is_some();
    checks.push(DoctorCheck {
        name: "tools".into(),
        ok: has_core_tools,
        detail: format!("{} tools registered", registry.len()),
    });

    if let Some(db_path) = &config.db_path {
        match sakha_memory::Database::open(db_path) {
            Ok(_) => checks.push(DoctorCheck { name: "database".into(), ok: true, detail: db_path.clone() }),
            Err(err) => checks.push(DoctorCheck { name: "database".into(), ok: false, detail: err.to_string() }),
        }
    } else {
        checks.push(DoctorCheck {
            name: "database".into(),
            ok: true,
            detail: "no db_path configured; using in-memory stores".into(),
        });
    }

    let policy = build_permission_policy();
    checks.push(DoctorCheck {
        name: "permission_policy".into(),
        ok: true,
        detail: format!("auto_allow_below={:?}, layers={}", policy.auto_allow_below, policy.layers.len()),
    });

    let all_ok = checks.iter().all(|c| c.ok);
    let report = DoctorReport { checks, all_ok };
    print_output(&report, output);
    if all_ok {
        0
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doctor_with_mock_provider_reports_all_ok() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", dir.path());
        std::env::set_var("USERPROFILE", dir.path());
        let code = execute(OutputFormat::Json);
        assert_eq!(code, 0);
    }
}
