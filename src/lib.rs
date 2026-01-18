use std::{fs, path::Path};

use zed::serde_json::Value;
use zed::settings::LspSettings;
use zed::{Architecture, Os};
use zed_extension_api as zed;

const LANGUAGE_SERVER_ID: &str = "tex-japanese-formatter";

// TODO: Set these to the GitHub repo that hosts the LSP server releases.
const GITHUB_OWNER: &str = "keufcp";
const GITHUB_REPO: &str = "tex-japanese-formatter-for-zed";

// If set, downloads that specific tag. If None, uses the latest release.
const RELEASE_TAG: Option<&str> = None;

struct TexJapaneseFormatter;

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

    fn ensure_repo_configured() -> zed::Result<()> {
        if GITHUB_OWNER == "CHANGE_ME_OWNER" || GITHUB_REPO == "CHANGE_ME_REPO" {
            return Err(
                "GitHub repository is not configured in src/lib.rs (GITHUB_OWNER / GITHUB_REPO)"
                    .into(),
            );
        }
        Ok(())
    }

    fn ensure_download_dir() -> zed::Result<()> {
        fs::create_dir_all("bin")
            .map_err(|err| format!("Failed to create download directory: {err}").into())
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

    fn download_server_binary(target: &str) -> zed::Result<String> {
        Self::ensure_repo_configured()?;

        let repo = format!("{GITHUB_OWNER}/{GITHUB_REPO}");
        let release = if let Some(tag) = RELEASE_TAG {
            zed::github_release_by_tag_name(&repo, tag)?
        } else {
            zed::latest_github_release(
                &repo,
                zed::GithubReleaseOptions {
                    require_assets: true,
                    pre_release: false,
                },
            )?
        };

        let asset = Self::find_asset_for_target(release, target)?;

        // The asset is expected to be a raw executable binary.
        // If you distribute archives, add extraction logic here.
        let filename = Self::binary_filename(target);
        let download_path = format!("bin/{filename}");

        if Path::new(&download_path).exists() {
            return Ok(download_path);
        }

        Self::ensure_download_dir()?;
        zed::download_file(
            &asset.download_url,
            &download_path,
            zed::DownloadedFileType::Uncompressed,
        )?;

        if !matches!(zed::current_platform().0, Os::Windows) {
            zed::make_file_executable(&download_path)?;
        }

        Ok(download_path)
    }

    fn language_server_command_inner(worktree: &zed::Worktree) -> zed::Result<zed::Command> {
        let lsp_settings = Self::lsp_settings(worktree)?;

        if let Some(binary_settings) = lsp_settings.binary {
            if let Some(command) = Self::command_from_settings(binary_settings) {
                return Ok(command);
            }
        }

        let target = Self::platform_target()?;
        let path = Self::download_server_binary(target)?;

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
        Self
    }

    fn language_server_command(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<zed::Command> {
        if language_server_id.as_ref() != LANGUAGE_SERVER_ID {
            return Err(format!("Unsupported language server: {language_server_id}").into());
        }

        Self::language_server_command_inner(worktree)
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

zed::register_extension!(TexJapaneseFormatter);
