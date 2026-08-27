use std::fmt;

const DEFAULT_BASE_IMAGE: &str = "ubuntu:25.10";
const MAX_BASE_IMAGE_LEN: usize = 255;
const MAX_IMAGE_NAME_LEN: usize = 128;
const MAX_PACKAGE_COUNT: usize = 100;
const MAX_PACKAGE_NAME_LEN: usize = 128;

pub(crate) const CHECK_PACKAGES_SCRIPT: &str = r#"for pkg do
    status="$(dpkg-query -W -f='${Status}' -- "$pkg" 2>/dev/null)"
    if [ "$status" = 'install ok installed' ] || command -v "$pkg" >/dev/null 2>&1; then
        printf 'FOUND:%s\n' "$pkg"
    fi
done"#;

#[derive(Debug, serde::Deserialize)]
struct ImageRequest {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    base: Option<String>,
    #[serde(default)]
    packages: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PackageName(String);

impl PackageName {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PackageName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl TryFrom<String> for PackageName {
    type Error = ImageInputError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let value = value.trim();
        if value.is_empty() {
            return Err(ImageInputError::PackageNameEmpty);
        }
        if value.len() > MAX_PACKAGE_NAME_LEN {
            return Err(ImageInputError::PackageNameTooLong);
        }

        let mut bytes = value.bytes();
        let valid = bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
            && bytes.all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'+' | b':')
            });
        if !valid {
            return Err(ImageInputError::PackageNameInvalid);
        }

        Ok(Self(value.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BaseImageReference(String);

impl BaseImageReference {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BaseImageReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl TryFrom<String> for BaseImageReference {
    type Error = ImageInputError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let value = value.trim();
        let value = if value.is_empty() {
            DEFAULT_BASE_IMAGE
        } else {
            value
        };
        if value.len() > MAX_BASE_IMAGE_LEN {
            return Err(ImageInputError::BaseImageTooLong);
        }

        let mut bytes = value.bytes();
        let valid = bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
            && bytes.all(|byte| {
                byte.is_ascii_alphanumeric()
                    || matches!(byte, b'-' | b'.' | b':' | b'/' | b'_' | b'@')
            });
        if !valid {
            return Err(ImageInputError::BaseImageInvalid);
        }

        Ok(Self(value.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImageName(String);

impl ImageName {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ImageName {
    type Error = ImageInputError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let value = value.trim();
        if value.is_empty()
            || value.len() > MAX_IMAGE_NAME_LEN
            || !value.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            })
        {
            return Err(ImageInputError::ImageNameInvalid);
        }

        Ok(Self(value.to_string()))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedImageRequest {
    name: Option<ImageName>,
    base: BaseImageReference,
    packages: Vec<PackageName>,
}

impl ValidatedImageRequest {
    pub(crate) fn name(&self) -> Option<&ImageName> {
        self.name.as_ref()
    }

    pub(crate) fn base(&self) -> &BaseImageReference {
        &self.base
    }

    pub(crate) fn packages(&self) -> &[PackageName] {
        &self.packages
    }
}

pub(crate) fn package_check_args(request: &ValidatedImageRequest) -> Vec<&str> {
    let mut args = vec![
        "run",
        "--rm",
        "--entrypoint",
        "sh",
        request.base().as_str(),
        "-c",
        CHECK_PACKAGES_SCRIPT,
        "moltis-package-check",
    ];
    args.extend(request.packages().iter().map(PackageName::as_str));
    args
}

impl TryFrom<serde_json::Value> for ValidatedImageRequest {
    type Error = ImageInputError;

    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        let request: ImageRequest = serde_json::from_value(value)
            .map_err(|error| ImageInputError::RequestInvalid(error.to_string()))?;
        Self::try_from(request)
    }
}

impl TryFrom<ImageRequest> for ValidatedImageRequest {
    type Error = ImageInputError;

    fn try_from(request: ImageRequest) -> Result<Self, Self::Error> {
        let package_values = request.packages.unwrap_or_default();
        if package_values.len() > MAX_PACKAGE_COUNT {
            return Err(ImageInputError::TooManyPackages);
        }
        let package_values: Vec<String> = package_values
            .into_iter()
            .map(|package| package.trim().to_string())
            .filter(|package| !package.is_empty())
            .collect();

        let packages = package_values
            .into_iter()
            .map(PackageName::try_from)
            .collect::<Result<_, _>>()?;
        let base = BaseImageReference::try_from(request.base.unwrap_or_default())?;

        let name = request
            .name
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
            .map(ImageName::try_from)
            .transpose()?;

        Ok(Self {
            name,
            base,
            packages,
        })
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ImageInputError {
    #[error("request body is invalid: {0}")]
    RequestInvalid(String),
    #[error("package name must not be empty")]
    PackageNameEmpty,
    #[error("package name exceeds 128 ASCII characters")]
    PackageNameTooLong,
    #[error(
        "package name must start with an ASCII letter or digit and contain only ASCII letters, digits, dash, dot, plus, or colon"
    )]
    PackageNameInvalid,
    #[error("base image reference exceeds 255 ASCII characters")]
    BaseImageTooLong,
    #[error(
        "base image reference must start with an ASCII letter or digit and contain only ASCII letters, digits, dash, dot, colon, slash, underscore, or @"
    )]
    BaseImageInvalid,
    #[error("packages list exceeds the maximum of 100 entries")]
    TooManyPackages,
    #[error("image name must contain only ASCII letters, digits, dash, or underscore")]
    ImageNameInvalid,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(value: impl Into<String>) -> Result<PackageName, ImageInputError> {
        PackageName::try_from(value.into())
    }

    fn base_image(value: impl Into<String>) -> Result<BaseImageReference, ImageInputError> {
        BaseImageReference::try_from(value.into())
    }

    #[test]
    fn accepts_valid_package_names() {
        for value in [
            "curl",
            "libssl3",
            "libc6:amd64",
            "g++",
            "python3.13-minimal",
        ] {
            assert!(package(value).is_ok(), "expected {value:?} to be valid");
        }
    }

    #[test]
    fn enforces_package_name_length_boundary() {
        assert_eq!(package("  "), Err(ImageInputError::PackageNameEmpty));
        assert!(package("a".repeat(MAX_PACKAGE_NAME_LEN)).is_ok());
        assert_eq!(
            package("a".repeat(MAX_PACKAGE_NAME_LEN + 1)),
            Err(ImageInputError::PackageNameTooLong)
        );
    }

    #[test]
    fn rejects_package_source_and_option_injection() {
        for value in [
            "curl'",
            "curl\"",
            "curl;id",
            "curl\nwhoami",
            "curl package",
            "-oProxyCommand=id",
            "cürl",
        ] {
            assert_eq!(
                package(value),
                Err(ImageInputError::PackageNameInvalid),
                "expected {value:?} to be rejected"
            );
        }
    }

    #[test]
    fn accepts_valid_base_image_references() {
        for value in [
            "ubuntu:25.10",
            "docker.io/library/ubuntu:latest",
            "localhost:5000/moltis_image:v1",
            "ubuntu@sha256:abcdef0123456789",
        ] {
            assert!(base_image(value).is_ok(), "expected {value:?} to be valid");
        }
    }

    #[test]
    fn enforces_base_image_length_boundary() {
        assert!(base_image("a".repeat(MAX_BASE_IMAGE_LEN)).is_ok());
        assert_eq!(
            base_image("a".repeat(MAX_BASE_IMAGE_LEN + 1)),
            Err(ImageInputError::BaseImageTooLong)
        );
    }

    #[test]
    fn rejects_dockerfile_and_container_option_injection() {
        for value in [
            "ubuntu'",
            "ubuntu;RUN id",
            "ubuntu\nRUN id",
            "ubuntu AS attacker",
            "ubuntu\\latest",
            "-v/tmp:/host",
            "übuntu:latest",
        ] {
            assert_eq!(
                base_image(value),
                Err(ImageInputError::BaseImageInvalid),
                "expected {value:?} to be rejected"
            );
        }
    }

    #[test]
    fn defaults_and_trims_request_values() {
        let request = serde_json::json!({
            "base": "   ",
            "packages": ["  curl  ", "", "   "]
        });
        let result = ValidatedImageRequest::try_from(request);
        let Ok(request) = result else {
            panic!("expected request to be valid: {result:?}");
        };

        assert_eq!(request.base().as_str(), DEFAULT_BASE_IMAGE);
        assert_eq!(request.packages(), &[PackageName("curl".to_string())]);

        let missing_values = ValidatedImageRequest::try_from(serde_json::json!({}));
        let Ok(missing_values) = missing_values else {
            panic!("expected missing values to use defaults: {missing_values:?}");
        };
        assert_eq!(missing_values.base().as_str(), DEFAULT_BASE_IMAGE);
        assert!(missing_values.packages().is_empty());
    }

    #[test]
    fn accepts_package_count_boundary_and_rejects_excess() {
        let at_limit = serde_json::json!({ "packages": vec!["curl"; MAX_PACKAGE_COUNT] });
        assert!(ValidatedImageRequest::try_from(at_limit).is_ok());

        let over_limit = serde_json::json!({ "packages": vec!["curl"; MAX_PACKAGE_COUNT + 1] });
        assert_eq!(
            ValidatedImageRequest::try_from(over_limit),
            Err(ImageInputError::TooManyPackages)
        );

        let empty_over_limit = serde_json::json!({
            "packages": vec![" "; MAX_PACKAGE_COUNT + 1]
        });
        assert_eq!(
            ValidatedImageRequest::try_from(empty_over_limit),
            Err(ImageInputError::TooManyPackages)
        );
    }

    #[test]
    fn rejects_malformed_package_array() {
        let malformed = serde_json::json!({ "packages": ["curl", 42] });
        assert!(matches!(
            ValidatedImageRequest::try_from(malformed),
            Err(ImageInputError::RequestInvalid(_))
        ));
    }

    #[test]
    fn builds_fixed_package_check_arguments_with_shell_sentinel() {
        let result = ValidatedImageRequest::try_from(serde_json::json!({
            "base": "ubuntu:25.10",
            "packages": ["curl", "git"]
        }));
        let Ok(request) = result else {
            panic!("expected request to be valid: {result:?}");
        };
        let args = package_check_args(&request);

        assert_eq!(args, [
            "run",
            "--rm",
            "--entrypoint",
            "sh",
            "ubuntu:25.10",
            "-c",
            CHECK_PACKAGES_SCRIPT,
            "moltis-package-check",
            "curl",
            "git"
        ]);
        assert!(CHECK_PACKAGES_SCRIPT.contains("-f='${Status}' -- \"$pkg\""));
        assert!(CHECK_PACKAGES_SCRIPT.contains("install ok installed"));
        assert!(!CHECK_PACKAGES_SCRIPT.contains("curl"));
    }

    #[test]
    fn image_name_validation_is_ascii_only() {
        assert_eq!(
            ImageName::try_from("developer_1-test".to_string()),
            Ok(ImageName("developer_1-test".to_string()))
        );
        assert_eq!(
            ImageName::try_from("déveloper".to_string()),
            Err(ImageInputError::ImageNameInvalid)
        );
        assert_eq!(
            ImageName::try_from("a".repeat(MAX_IMAGE_NAME_LEN + 1)),
            Err(ImageInputError::ImageNameInvalid)
        );
    }

    #[test]
    fn converts_valid_request_to_typed_values() {
        let input = serde_json::json!({
            "name": "  developer  ",
            "base": "  ghcr.io/moltis-org/base:v1  ",
            "packages": [" curl ", "git"]
        });
        let result = ValidatedImageRequest::try_from(input);
        let Ok(request) = result else {
            panic!("expected request to be valid: {result:?}");
        };

        assert_eq!(request.name(), Some(&ImageName("developer".to_string())));
        assert_eq!(request.base().as_str(), "ghcr.io/moltis-org/base:v1");
        assert_eq!(request.packages(), &[
            PackageName("curl".to_string()),
            PackageName("git".to_string())
        ]);
    }
}
