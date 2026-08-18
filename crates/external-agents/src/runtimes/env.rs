use tokio::process::Command;

pub(crate) fn inject_managed_files_dir(command: &mut Command) {
    command.env("MOLTIS_FILES_DIR", moltis_config::managed_files_dir());
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn injects_managed_files_dir_into_local_child() {
        let mut command = Command::new("unused");
        inject_managed_files_dir(&mut command);

        let value = command
            .as_std()
            .get_envs()
            .find_map(|(key, value)| (key == "MOLTIS_FILES_DIR").then_some(value))
            .flatten()
            .unwrap();
        assert_eq!(value, moltis_config::managed_files_dir().as_os_str());
    }
}
