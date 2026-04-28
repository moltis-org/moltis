#[path = "tests/core.rs"]
mod core;
#[path = "tests/layered.rs"]
mod layered;

use std::{path::PathBuf, sync::Mutex};

use super::{
    config_io::{
        apply_env_overrides_with, parse_config, parse_env_value, resubstitute_config,
        save_user_config_to_path, set_nested, strip_default_values,
    },
    *,
};

struct TestDirState {
    _path: Option<PathBuf>,
}

static DATA_DIR_TEST_LOCK: Mutex<TestDirState> = Mutex::new(TestDirState { _path: None });

/// Lock guarding tests that modify `CONFIG_DIR_OVERRIDE` to prevent races.
static CONFIG_DIR_TEST_LOCK: Mutex<TestDirState> = Mutex::new(TestDirState { _path: None });
