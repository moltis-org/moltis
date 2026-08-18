use std::path::{Component, Path};

use super::{FilesError, FilesResult};

pub(crate) const MAX_LOGICAL_PATH_BYTES: usize = 4096;
pub(crate) const MAX_COMPONENT_BYTES: usize = 255;
pub(crate) const MAX_PATH_DEPTH: usize = 64;
pub(crate) const TEMP_NAME_PREFIX: &str = ".moltis-upload-";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LogicalPath {
    raw: String,
    components: Vec<String>,
}

impl LogicalPath {
    pub(crate) fn parse(raw: &str, allow_root: bool) -> FilesResult<Self> {
        if raw.is_empty() {
            return if allow_root {
                Ok(Self {
                    raw: String::new(),
                    components: Vec::new(),
                })
            } else {
                Err(FilesError::RootMutation)
            };
        }
        if raw.len() > MAX_LOGICAL_PATH_BYTES
            || raw.starts_with('/')
            || raw.contains('\\')
            || raw.chars().any(char::is_control)
        {
            return Err(FilesError::InvalidPath);
        }

        let components = raw.split('/').map(str::to_owned).collect::<Vec<String>>();
        if components.len() > MAX_PATH_DEPTH
            || components.iter().any(|component| {
                component.is_empty()
                    || component == "."
                    || component == ".."
                    || looks_like_windows_prefix(component)
                    || component.len() > MAX_COMPONENT_BYTES
                    || component.starts_with(TEMP_NAME_PREFIX)
            })
        {
            return Err(FilesError::InvalidPath);
        }

        let path = Path::new(raw);
        if path.components().any(|component| {
            matches!(
                component,
                Component::Prefix(_)
                    | Component::RootDir
                    | Component::ParentDir
                    | Component::CurDir
            )
        }) {
            return Err(FilesError::InvalidPath);
        }

        Ok(Self {
            raw: raw.to_owned(),
            components,
        })
    }

    pub(crate) fn raw(&self) -> &str {
        &self.raw
    }

    pub(crate) fn is_root(&self) -> bool {
        self.components.is_empty()
    }

    pub(crate) fn components(&self) -> &[String] {
        &self.components
    }

    pub(crate) fn parent_components(&self) -> &[String] {
        let end = self.components.len().saturating_sub(1);
        &self.components[..end]
    }

    pub(crate) fn file_name(&self) -> FilesResult<&str> {
        self.components
            .last()
            .map(String::as_str)
            .ok_or(FilesError::RootMutation)
    }
}

fn looks_like_windows_prefix(component: &str) -> bool {
    let bytes = component.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}
