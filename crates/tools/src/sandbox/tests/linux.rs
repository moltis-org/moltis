#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

#[test]
fn test_cgroup_scope_name() {
    let config = SandboxConfig::default();
    let cgroup = CgroupSandbox::new(config);
    let id = SandboxId {
        scope: SandboxScope::Session,
        key: "sess1".into(),
    };
    assert_eq!(cgroup.scope_name(&id), "moltis-sandbox-sess1");
}

#[test]
fn test_cgroup_property_args() {
    let config = SandboxConfig {
        resource_limits: ResourceLimits {
            memory_limit: Some("1G".into()),
            cpu_quota: Some(2.0),
            pids_max: Some(200),
        },
        ..Default::default()
    };
    let cgroup = CgroupSandbox::new(config);
    let args = cgroup.property_args();
    assert!(args.contains(&"MemoryMax=1G".to_string()));
    assert!(args.contains(&"CPUQuota=200%".to_string()));
    assert!(args.contains(&"TasksMax=200".to_string()));
}
