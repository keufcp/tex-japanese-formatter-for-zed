use std::fs;

use zed::serde_json::Value;
use zed::settings::LspSettings;
use zed::{Architecture, Os};
use zed_extension_api as zed;

const LANGUAGE_SERVER_ID: &str = "tex-japanese-formatter";

// The GitHub repository that hosts the LSP server releases.
const GITHUB_OWNER: &str = "keufcp";
const GITHUB_REPO: &str = "tex-japanese-formatter-for-zed";

// If set, downloads that specific tag. If None, uses the latest release.
const RELEASE_TAG: Option<&str> = None;

/// Downloaded binaries live in `<prefix><release tag>/`.
const VERSION_DIR_PREFIX: &str = "tex-japanese-formatter-lsp-";
/// Download directory used by versions up to 1.0.0, which never updated.
const LEGACY_DOWNLOAD_DIR: &str = "bin";

struct TexJapaneseFormatter {
    cached_binary_path: Option<String>,
}

impl TexJapaneseFormatter {
    fn lsp_settings(worktree: &zed::Worktree) -> zed::Result<LspSettings> {
        LspSettings::for_worktree(LANGUAGE_SERVER_ID, worktree)
    }

    fn command_from_settings(settings: zed::settings::CommandSettings) -> Option<zed::Command> {
        let path = settings.path?;
        let args = settings.arguments.unwrap_or_default();
        let env = settings
            .env
            .unwrap_or_default()
            .into_iter()
            .collect::<Vec<(String, String)>>();

        Some(zed::Command {
            command: path,
            args,
            env,
        })
    }

    fn platform_target() -> zed::Result<&'static str> {
        let (os, arch) = zed::current_platform();
        match (os, arch) {
            (Os::Windows, Architecture::X8664) => Ok("x86_64-pc-windows-msvc"),
            (Os::Mac, Architecture::X8664) => Ok("x86_64-apple-darwin"),
            (Os::Mac, Architecture::Aarch64) => Ok("aarch64-apple-darwin"),
            (Os::Linux, Architecture::X8664) => Ok("x86_64-unknown-linux-gnu"),
            (Os::Linux, Architecture::Aarch64) => Ok("aarch64-unknown-linux-gnu"),
            _ => Err(format!("Unsupported platform: {os:?} {arch:?}").into()),
        }
    }

    fn binary_filename(target: &str) -> String {
        if matches!(zed::current_platform().0, Os::Windows) {
            format!("tex-japanese-formatter-lsp-{target}.exe")
        } else {
            format!("tex-japanese-formatter-lsp-{target}")
        }
    }

    fn find_asset_for_target(
        release: zed::GithubRelease,
        target: &str,
    ) -> zed::Result<zed::GithubReleaseAsset> {
        let expected = Self::binary_filename(target);
        release
            .assets
            .into_iter()
            .find(|asset| asset.name == expected)
            .ok_or_else(|| {
                format!(
                    "No release asset found for target {target}. Expected asset name: {expected}"
                )
                .into()
            })
    }

    fn is_file(path: &str) -> bool {
        fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
    }

    fn fetch_release() -> zed::Result<zed::GithubRelease> {
        let repo = format!("{GITHUB_OWNER}/{GITHUB_REPO}");
        if let Some(tag) = RELEASE_TAG {
            zed::github_release_by_tag_name(&repo, tag)
        } else {
            zed::latest_github_release(
                &repo,
                zed::GithubReleaseOptions {
                    require_assets: true,
                    pre_release: false,
                },
            )
        }
    }

    /// Finds a previously downloaded binary, used when GitHub is unreachable.
    /// The newest version wins.
    fn installed_binary(target: &str) -> Option<String> {
        let filename = Self::binary_filename(target);
        let directories = fs::read_dir(".")
            .ok()?
            .flatten()
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| Self::is_file(&format!("{name}/{filename}")));

        newest_version_dir(directories)
            .map(|dir| format!("{dir}/{filename}"))
            .or_else(|| {
                Some(format!("{LEGACY_DOWNLOAD_DIR}/{filename}")).filter(|p| Self::is_file(p))
            })
    }

    /// Removes binaries of other versions, including the legacy `bin/` directory.
    fn remove_other_versions(current_dir: &str) {
        let Ok(entries) = fs::read_dir(".") else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            let is_old_version = name.starts_with(VERSION_DIR_PREFIX) && name != current_dir;
            if is_old_version || name == LEGACY_DOWNLOAD_DIR {
                fs::remove_dir_all(entry.path()).ok();
            }
        }
    }

    /// Downloads to `path` and makes it executable.
    fn download_executable(url: &str, path: &str) -> zed::Result<()> {
        zed::download_file(url, path, zed::DownloadedFileType::Uncompressed)?;
        if !matches!(zed::current_platform().0, Os::Windows) {
            zed::make_file_executable(path)?;
        }
        Ok(())
    }

    /// Downloads the server for `release` unless it is already present.
    /// Each release is stored in its own directory so that a new release is
    /// always picked up.
    fn install_release(
        language_server_id: &zed::LanguageServerId,
        release: zed::GithubRelease,
        target: &str,
    ) -> zed::Result<String> {
        let version_dir = version_dir_name(&release.version);
        // The asset is expected to be a raw executable binary.
        // If you distribute archives, add extraction logic here.
        let binary_path = format!("{version_dir}/{}", Self::binary_filename(target));

        if !Self::is_file(&binary_path) {
            zed::set_language_server_installation_status(
                language_server_id,
                &zed::LanguageServerInstallationStatus::Downloading,
            );
            let asset = Self::find_asset_for_target(release, target)?;
            fs::create_dir_all(&version_dir)
                .map_err(|err| format!("Failed to create download directory: {err}"))?;

            // Download to a temporary file and move it into place only when
            // it is complete, so a failure never leaves a broken binary at
            // `binary_path`.
            let temp_path = format!("{binary_path}.download");
            fs::remove_file(&temp_path).ok();
            let result =
                Self::download_executable(&asset.download_url, &temp_path).and_then(|()| {
                    fs::rename(&temp_path, &binary_path)
                        .map_err(|err| format!("Failed to move downloaded binary: {err}"))
                });
            if let Err(err) = result {
                fs::remove_file(&temp_path).ok();
                // Only succeeds if the directory is empty.
                fs::remove_dir(&version_dir).ok();
                return Err(err);
            }
        }

        Self::remove_other_versions(&version_dir);
        Ok(binary_path)
    }

    fn server_binary_path(
        &mut self,
        language_server_id: &zed::LanguageServerId,
    ) -> zed::Result<String> {
        if let Some(path) = &self.cached_binary_path
            && Self::is_file(path)
        {
            return Ok(path.clone());
        }

        let target = Self::platform_target()?;
        zed::set_language_server_installation_status(
            language_server_id,
            &zed::LanguageServerInstallationStatus::CheckingForUpdate,
        );

        let installed = Self::fetch_release()
            .and_then(|release| Self::install_release(language_server_id, release, target));

        match installed {
            Ok(path) => {
                zed::set_language_server_installation_status(
                    language_server_id,
                    &zed::LanguageServerInstallationStatus::None,
                );
                self.cached_binary_path = Some(path.clone());
                Ok(path)
            }
            // Fall back to a previously downloaded binary. The result is not
            // cached, so the next start of the language server checks for an
            // update again.
            Err(err) => match Self::installed_binary(target) {
                Some(path) => {
                    zed::set_language_server_installation_status(
                        language_server_id,
                        &zed::LanguageServerInstallationStatus::None,
                    );
                    Ok(path)
                }
                None => {
                    zed::set_language_server_installation_status(
                        language_server_id,
                        &zed::LanguageServerInstallationStatus::Failed(err.clone()),
                    );
                    Err(err)
                }
            },
        }
    }

    fn language_server_command_inner(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<zed::Command> {
        let lsp_settings = Self::lsp_settings(worktree)?;

        if let Some(binary_settings) = lsp_settings.binary {
            if let Some(command) = Self::command_from_settings(binary_settings) {
                return Ok(command);
            }
        }

        let path = self.server_binary_path(language_server_id)?;

        Ok(zed::Command {
            command: path,
            args: Vec::new(),
            env: Vec::new(),
        })
    }

    fn default_settings_value() -> Value {
        zed::serde_json::json!({
            "enabled": true,
            "formatOnSave": true,
            "targetLanguages": ["latex"]
        })
    }

    fn apply_settings_defaults(settings: Value) -> Value {
        match settings {
            Value::Object(mut map) => {
                let defaults = Self::default_settings_value();
                if let Value::Object(defaults_map) = defaults {
                    for (key, value) in defaults_map {
                        map.entry(key).or_insert(value);
                    }
                }
                Value::Object(map)
            }
            _ => Self::default_settings_value(),
        }
    }

    fn lsp_settings_value(worktree: &zed::Worktree) -> zed::Result<Option<Value>> {
        let lsp_settings = Self::lsp_settings(worktree)?;
        let settings = lsp_settings
            .settings
            .map(Self::apply_settings_defaults)
            .unwrap_or_else(Self::default_settings_value);
        Ok(Some(settings))
    }
}

impl zed::Extension for TexJapaneseFormatter {
    fn new() -> Self {
        Self {
            cached_binary_path: None,
        }
    }

    fn language_server_command(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<zed::Command> {
        if language_server_id.as_ref() != LANGUAGE_SERVER_ID {
            return Err(format!("Unsupported language server: {language_server_id}").into());
        }

        self.language_server_command_inner(language_server_id, worktree)
    }

    fn language_server_initialization_options(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<Option<Value>> {
        if language_server_id.as_ref() != LANGUAGE_SERVER_ID {
            return Ok(None);
        }

        Self::lsp_settings_value(worktree)
    }

    fn language_server_workspace_configuration(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<Option<Value>> {
        if language_server_id.as_ref() != LANGUAGE_SERVER_ID {
            return Ok(None);
        }

        Self::lsp_settings_value(worktree)
    }
}

/// Directory name for a release tag. Characters that are not safe in a
/// single path component (e.g. `/` in `lsp/v1.0.0`) are replaced, so the
/// directory never nests.
fn version_dir_name(tag: &str) -> String {
    let safe: String = tag
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+') {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{VERSION_DIR_PREFIX}{safe}")
}

/// A pre-release identifier. Numeric identifiers sort before alphanumeric
/// ones, as in Semantic Versioning.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Identifier {
    Numeric(u64),
    Alphanumeric(String),
}

/// A pre-release sorts before the release with the same numbers.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Stage {
    PreRelease(Vec<Identifier>),
    Release,
}

/// Sort key for a release tag such as `v1.10.0` or `v2.0.0-rc.10`,
/// following the precedence rules of Semantic Versioning. Build metadata
/// (`+...`) is ignored and trailing zeros are dropped (`v1.2` == `v1.2.0`).
fn version_key(tag: &str) -> (Vec<u64>, Stage) {
    let tag = tag.strip_prefix('v').unwrap_or(tag);
    let tag = tag.split('+').next().unwrap_or(tag);
    let (core, stage) = match tag.split_once('-') {
        Some((core, pre)) => {
            let identifiers = pre
                .split('.')
                .map(|part| match part.parse() {
                    Ok(number) => Identifier::Numeric(number),
                    Err(_) => Identifier::Alphanumeric(part.to_string()),
                })
                .collect();
            (core, Stage::PreRelease(identifiers))
        }
        None => (tag, Stage::Release),
    };

    let mut numbers: Vec<u64> = core
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect();
    while numbers.last() == Some(&0) {
        numbers.pop();
    }
    (numbers, stage)
}

/// Picks the directory of the newest version among `names`, ignoring names
/// that are not version directories.
fn newest_version_dir(names: impl Iterator<Item = String>) -> Option<String> {
    names
        .filter_map(|name| {
            let key = version_key(name.strip_prefix(VERSION_DIR_PREFIX)?);
            Some((key, name))
        })
        .max()
        .map(|(_, name)| name)
}

zed::register_extension!(TexJapaneseFormatter);

#[cfg(test)]
mod tests {
    use super::*;

    fn newest(names: &[&str]) -> Option<String> {
        newest_version_dir(names.iter().map(|name| name.to_string()))
    }

    #[test]
    fn compares_versions_numerically() {
        let dirs = [
            "tex-japanese-formatter-lsp-v1.9.0",
            "tex-japanese-formatter-lsp-v1.10.0",
            "tex-japanese-formatter-lsp-v1.2.0",
        ];
        assert_eq!(
            newest(&dirs).as_deref(),
            Some("tex-japanese-formatter-lsp-v1.10.0")
        );
    }

    #[test]
    fn release_wins_over_pre_release() {
        let dirs = [
            "tex-japanese-formatter-lsp-v2.0.0-rc.1",
            "tex-japanese-formatter-lsp-v2.0.0",
            "tex-japanese-formatter-lsp-v1.99.0",
        ];
        assert_eq!(
            newest(&dirs).as_deref(),
            Some("tex-japanese-formatter-lsp-v2.0.0")
        );
    }

    #[test]
    fn ignores_other_directories() {
        assert_eq!(newest(&["bin", "other"]), None);
        assert_eq!(
            newest(&["bin", "tex-japanese-formatter-lsp-v1.0.0"]).as_deref(),
            Some("tex-japanese-formatter-lsp-v1.0.0")
        );
    }

    #[test]
    fn version_key_handles_missing_prefix_and_metadata() {
        assert_eq!(version_key("1.2.3"), (vec![1, 2, 3], Stage::Release));
        assert_eq!(version_key("v1.2.3+build.5"), version_key("v1.2.3"));
        // A `-` inside build metadata does not make it a pre-release.
        assert_eq!(version_key("v1.2.3+build-5"), version_key("v1.2.3"));
        assert_eq!(
            version_key("v1.2.3-beta+exp.sha-1"),
            (
                vec![1, 2, 3],
                Stage::PreRelease(vec![Identifier::Alphanumeric("beta".into())])
            )
        );
    }

    #[test]
    fn trailing_zeros_are_ignored() {
        assert_eq!(version_key("v1.2"), version_key("v1.2.0"));
        assert_eq!(version_key("v1"), version_key("v1.0.0"));
        assert!(version_key("v1.2.1") > version_key("v1.2"));
    }

    #[test]
    fn pre_release_identifiers_follow_semver() {
        // The precedence example from the Semantic Versioning specification.
        let ordered = [
            "v1.0.0-alpha",
            "v1.0.0-alpha.1",
            "v1.0.0-alpha.beta",
            "v1.0.0-beta",
            "v1.0.0-beta.2",
            "v1.0.0-beta.11",
            "v1.0.0-rc.1",
            "v1.0.0",
        ];
        for pair in ordered.windows(2) {
            assert!(
                version_key(pair[0]) < version_key(pair[1]),
                "{} < {}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn newest_pre_release_is_picked() {
        let dirs = [
            "tex-japanese-formatter-lsp-v2.0.0-rc.9",
            "tex-japanese-formatter-lsp-v2.0.0-rc.10",
        ];
        assert_eq!(
            newest(&dirs).as_deref(),
            Some("tex-japanese-formatter-lsp-v2.0.0-rc.10")
        );
    }

    #[test]
    fn version_dir_name_never_nests() {
        assert_eq!(
            version_dir_name("v1.0.0"),
            "tex-japanese-formatter-lsp-v1.0.0"
        );
        assert_eq!(
            version_dir_name("lsp/v1.1.0"),
            "tex-japanese-formatter-lsp-lsp_v1.1.0"
        );
        assert_eq!(
            version_dir_name("a\\b:c*d"),
            "tex-japanese-formatter-lsp-a_b_c_d"
        );
        assert_eq!(
            version_dir_name("v2.0.0-rc.1+build"),
            "tex-japanese-formatter-lsp-v2.0.0-rc.1+build"
        );
    }
}
