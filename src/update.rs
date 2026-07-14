use serde::Deserialize;
use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use futures_util::{future::BoxFuture, StreamExt};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/frankwei98/sms-relayed/releases/latest";
const OPENWRT_SERVICE: &str = "/etc/init.d/sms-relayed";
const SYSTEMD_SERVICE: &str = "sms-relayed";

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    target_commitish: String,
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

fn select_release_asset<'a>(release: &'a Release, suffix: &str) -> Result<&'a ReleaseAsset> {
    let expected_name = format!("sms-relayed-{}-{suffix}", release.tag_name);
    let mut matches = release
        .assets
        .iter()
        .filter(|asset| asset.name == expected_name);
    let asset = matches.next();
    if asset.is_none() || matches.next().is_some() {
        anyhow::bail!(
            "expected exactly one release asset named {expected_name} for release {}",
            release.tag_name
        );
    }
    Ok(asset.expect("asset presence checked above"))
}

fn release_asset_for_update<'a>(
    release: &'a Release,
    suffix: &str,
    current_commit: &str,
) -> Result<Option<&'a ReleaseAsset>> {
    if release_matches_commit(release, current_commit) {
        return Ok(None);
    }
    select_release_asset(release, suffix).map(Some)
}

fn release_matches_commit(release: &Release, current_commit: &str) -> bool {
    if is_hex_commit(&release.target_commitish) {
        return current_commit.eq_ignore_ascii_case(&release.target_commitish);
    }

    is_hex_commit(&release.tag_name)
        && current_commit
            .to_ascii_lowercase()
            .starts_with(&release.tag_name.to_ascii_lowercase())
}

fn version_output_matches_release(output: &str, release: &Release) -> bool {
    let Some(build_version) = output.trim().strip_prefix("sms-relayed ") else {
        return false;
    };
    build_version
        .rsplit_once('+')
        .map(|(version, commit)| !version.is_empty() && release_matches_commit(release, commit))
        .unwrap_or(false)
}

fn is_hex_commit(value: &str) -> bool {
    (7..=40).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn parse_openwrt_command(contents: &str) -> Option<PathBuf> {
    contents.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("procd_set_param")?.trim_start();
        let rest = rest.strip_prefix("command")?;
        if !rest.starts_with(char::is_whitespace) {
            return None;
        }
        first_shell_word(rest.trim_start()).map(PathBuf::from)
    })
}

fn parse_systemd_exec_start(value: &str) -> Option<PathBuf> {
    if let Some(rest) = value.split_once("path=").map(|(_, rest)| rest) {
        let path = rest
            .trim_start()
            .split([';', ' ', '\t', '\n'])
            .next()?
            .trim_matches(['\'', '"']);
        if path.starts_with('/') {
            return Some(PathBuf::from(path));
        }
    }

    value
        .split(|character: char| character.is_whitespace() || matches!(character, '{' | ';'))
        .map(|word| word.trim_matches(['\'', '"']))
        .find(|word| word.starts_with('/'))
        .map(PathBuf::from)
}

fn first_shell_word(input: &str) -> Option<String> {
    let mut word = String::new();
    let mut quote = None;
    let mut escaped = false;

    for character in input.chars() {
        if escaped {
            word.push(character);
            escaped = false;
            continue;
        }
        match (quote, character) {
            (_, '\\') => escaped = true,
            (Some(active), current) if current == active => quote = None,
            (None, '\'' | '"') => quote = Some(character),
            (None, current) if current.is_whitespace() => break,
            _ => word.push(character),
        }
    }

    (!word.is_empty() && quote.is_none() && !escaped).then_some(word)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ServiceManager {
    OpenWrt,
    Systemd,
}

impl ServiceManager {
    async fn restart_with(self, executor: &dyn CommandExecutor) -> Result<()> {
        let (program, arguments): (&str, &[&str]) = match self {
            Self::OpenWrt => (OPENWRT_SERVICE, &["restart"]),
            Self::Systemd => ("systemctl", &["restart", SYSTEMD_SERVICE]),
        };
        let result = executor
            .run(program, arguments)
            .await
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::PermissionDenied {
                    anyhow::anyhow!(
                        "binary updated, but permission was denied while restarting the service; rerun with sufficient privileges (for example, sudo)"
                    )
                } else {
                    anyhow::Error::new(error)
                        .context(format!("binary updated, but failed to run {program}"))
                }
            })?;
        if !result.success {
            anyhow::bail!(
                "binary updated, but service restart failed: {program} exited with {}; rerun with sufficient privileges (for example, sudo) if authorization was denied",
                result.status
            );
        }
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
enum RestartOutcome {
    Restarted,
    NoService,
}

async fn restart_after_update(
    service_manager: Option<ServiceManager>,
    executor: &dyn CommandExecutor,
) -> Result<RestartOutcome> {
    let Some(service_manager) = service_manager else {
        return Ok(RestartOutcome::NoService);
    };
    service_manager.restart_with(executor).await?;
    Ok(RestartOutcome::Restarted)
}

#[derive(Debug)]
struct CommandResult {
    success: bool,
    status: String,
    stdout: Vec<u8>,
}

trait CommandExecutor: Send + Sync {
    fn run<'a>(
        &'a self,
        program: &'a str,
        arguments: &'a [&'a str],
    ) -> BoxFuture<'a, std::io::Result<CommandResult>>;
}

struct SystemCommandExecutor;

impl CommandExecutor for SystemCommandExecutor {
    fn run<'a>(
        &'a self,
        program: &'a str,
        arguments: &'a [&'a str],
    ) -> BoxFuture<'a, std::io::Result<CommandResult>> {
        Box::pin(async move {
            let output = tokio::process::Command::new(program)
                .args(arguments)
                .output()
                .await?;
            Ok(CommandResult {
                success: output.status.success(),
                status: output.status.to_string(),
                stdout: output.stdout,
            })
        })
    }
}

async fn detect_service() -> (Option<ServiceManager>, Option<PathBuf>) {
    detect_service_with(Path::new(OPENWRT_SERVICE), &SystemCommandExecutor).await
}

async fn detect_service_with(
    openwrt_service: &Path,
    executor: &dyn CommandExecutor,
) -> (Option<ServiceManager>, Option<PathBuf>) {
    if openwrt_service.is_file() {
        let registered_path = tokio::fs::read_to_string(openwrt_service)
            .await
            .ok()
            .and_then(|contents| parse_openwrt_command(&contents));
        return (Some(ServiceManager::OpenWrt), registered_path);
    }

    let load_state = executor
        .run(
            "systemctl",
            &["show", "--property=LoadState", "--value", SYSTEMD_SERVICE],
        )
        .await;
    let systemd_loaded = load_state.as_ref().is_ok_and(|output| {
        output.success && String::from_utf8_lossy(&output.stdout).trim().eq("loaded")
    });
    if !systemd_loaded {
        return (None, None);
    }

    let registered_path = executor
        .run(
            "systemctl",
            &["show", "--property=ExecStart", "--value", SYSTEMD_SERVICE],
        )
        .await
        .ok()
        .filter(|output| output.success)
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|value| parse_systemd_exec_start(&value));
    (Some(ServiceManager::Systemd), registered_path)
}

fn find_on_path() -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|directory| directory.join("sms-relayed"))
        .find(|candidate| candidate.is_file())
}

fn resolve_update_target(registered_path: Option<PathBuf>) -> Result<PathBuf> {
    let current_exe = env::current_exe().context("failed to resolve current executable")?;
    select_update_target([registered_path, find_on_path(), Some(current_exe)])
        .context("could not find an installed sms-relayed binary to update")
}

fn select_update_target(candidates: impl IntoIterator<Item = Option<PathBuf>>) -> Option<PathBuf> {
    candidates
        .into_iter()
        .flatten()
        .find_map(|candidate| candidate.canonicalize().ok().filter(|path| path.is_file()))
}

fn asset_suffix() -> Result<&'static str> {
    asset_suffix_for(env::consts::OS, env::consts::ARCH)
}

fn asset_suffix_for(os: &str, architecture: &str) -> Result<&'static str> {
    if os != "linux" {
        anyhow::bail!("self-update is supported only on Linux, detected {}", os);
    }
    match architecture {
        "x86_64" => Ok("linux-musl-x64"),
        "aarch64" => Ok("linux-musl-aarch64"),
        architecture => anyhow::bail!(
            "no published self-update binary is available for architecture {architecture}"
        ),
    }
}

async fn fetch_latest_release(client: &reqwest::Client) -> Result<Release> {
    client
        .get(LATEST_RELEASE_URL)
        .send()
        .await
        .context("failed to query the latest GitHub release")?
        .error_for_status()
        .context("GitHub returned an error for the latest release")?
        .json()
        .await
        .context("failed to parse the latest GitHub release")
}

async fn download_and_replace(
    client: &reqwest::Client,
    release: &Release,
    asset: &ReleaseAsset,
    target: &Path,
) -> Result<()> {
    let directory = target
        .parent()
        .context("update target does not have a parent directory")?;
    let temporary = directory.join(format!(".sms-relayed.update-{}", Uuid::new_v4()));
    let result = download_validate_and_replace(client, release, asset, target, &temporary).await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temporary).await;
    }
    result
}

async fn download_validate_and_replace(
    client: &reqwest::Client,
    release: &Release,
    asset: &ReleaseAsset,
    target: &Path,
    temporary: &Path,
) -> Result<()> {
    let response = client
        .get(&asset.browser_download_url)
        .send()
        .await
        .context("failed to download the release binary")?
        .error_for_status()
        .context("GitHub returned an error while downloading the release binary")?;
    let mut file = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(temporary)
        .await
        .map_err(|error| update_io_error(error, target))?;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        file.write_all(&chunk.context("release download was interrupted")?)
            .await
            .context("failed to write the downloaded release binary")?;
    }
    file.flush()
        .await
        .context("failed to flush the downloaded release binary")?;
    file.sync_all()
        .await
        .context("failed to sync the downloaded release binary")?;
    drop(file);

    fs::set_permissions(temporary, fs::Permissions::from_mode(0o755))
        .map_err(|error| update_io_error(error, target))?;
    let output = tokio::process::Command::new(temporary)
        .arg("--version")
        .output()
        .await
        .context("downloaded release binary could not be executed for validation")?;
    let version = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() || !version_output_matches_release(&version, release) {
        anyhow::bail!(
            "downloaded release binary failed version validation for release {}",
            release.tag_name
        );
    }

    tokio::fs::rename(temporary, target)
        .await
        .map_err(|error| update_io_error(error, target))?;
    Ok(())
}

fn update_io_error(error: std::io::Error, target: &Path) -> anyhow::Error {
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        anyhow::anyhow!(
            "permission denied while updating {}; rerun the command with sufficient privileges (for example, sudo)",
            target.display()
        )
    } else {
        anyhow::Error::new(error).context(format!("failed to update {}", target.display()))
    }
}

pub async fn run() -> Result<()> {
    let suffix = asset_suffix()?;
    let (service_manager, registered_path) = detect_service().await;
    let target = resolve_update_target(registered_path)?;
    let client = reqwest::Client::builder()
        .user_agent(format!("sms-relayed/{}", env!("SMS_RELAYED_BUILD_VERSION")))
        .build()
        .context("failed to create the update HTTP client")?;
    let release = fetch_latest_release(&client).await?;

    println!("current version: {}", env!("SMS_RELAYED_BUILD_VERSION"));
    println!("latest release: {}", release.tag_name);
    println!("update target: {}", target.display());
    let asset = release_asset_for_update(&release, suffix, env!("SMS_RELAYED_BUILD_COMMIT"))?;
    let Some(asset) = asset else {
        println!("sms-relayed is already up to date");
        return Ok(());
    };

    println!("downloading asset: {}", asset.browser_download_url);
    download_and_replace(&client, &release, asset, &target).await?;
    println!("updated binary: {}", target.display());

    match restart_after_update(service_manager, &SystemCommandExecutor).await? {
        RestartOutcome::Restarted => println!("restarted sms-relayed service"),
        RestartOutcome::NoService => {
            eprintln!("warning: no OpenWrt or systemd sms-relayed service was found; binary updated without restarting a service");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::path::Path;
    use std::sync::Mutex;
    use std::{fs, io, os::unix::fs::symlink};

    use axum::body::{Body, Bytes};
    use axum::http::Response;
    use axum::{routing::get, Router};
    use futures_util::stream;

    use super::{
        asset_suffix_for, detect_service_with, download_and_replace, parse_openwrt_command,
        parse_systemd_exec_start, release_asset_for_update, release_matches_commit,
        restart_after_update, select_release_asset, select_update_target, update_io_error,
        version_output_matches_release, CommandExecutor, CommandResult, Release, ReleaseAsset,
        RestartOutcome, ServiceManager,
    };

    #[derive(Default)]
    struct FakeCommandExecutor {
        responses: Mutex<VecDeque<io::Result<CommandResult>>>,
        calls: Mutex<Vec<(String, Vec<String>)>>,
    }

    impl FakeCommandExecutor {
        fn with_responses(responses: Vec<CommandResult>) -> Self {
            Self {
                responses: Mutex::new(responses.into_iter().map(Ok).collect()),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl CommandExecutor for FakeCommandExecutor {
        fn run<'a>(
            &'a self,
            program: &'a str,
            arguments: &'a [&'a str],
        ) -> futures_util::future::BoxFuture<'a, io::Result<CommandResult>> {
            self.calls.lock().unwrap().push((
                program.to_string(),
                arguments.iter().map(|value| (*value).to_string()).collect(),
            ));
            let response = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("fake command response should be configured");
            Box::pin(async move { response })
        }
    }

    fn command_result(success: bool, stdout: &str) -> CommandResult {
        CommandResult {
            success,
            status: if success {
                "exit status: 0".to_string()
            } else {
                "exit status: 1".to_string()
            },
            stdout: stdout.as_bytes().to_vec(),
        }
    }

    struct TestDirectory(std::path::PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("sms-relayed-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn release_with_assets(names: &[&str]) -> Release {
        Release {
            tag_name: "0937382".to_string(),
            target_commitish: "09373820cd4ab63023359acf300d708d47c9f509".to_string(),
            assets: names
                .iter()
                .map(|name| ReleaseAsset {
                    name: (*name).to_string(),
                    browser_download_url: format!("https://example.test/{name}"),
                })
                .collect(),
        }
    }

    #[test]
    fn release_requires_one_exact_architecture_asset() {
        let release = release_with_assets(&[
            "sms-relayed-0937382-linux-musl-aarch64",
            "sms-relayed-0937382-linux-musl-x64",
        ]);

        let selected = select_release_asset(&release, "linux-musl-x64")
            .expect("one exact asset should be selected");

        assert_eq!(
            selected.browser_download_url,
            "https://example.test/sms-relayed-0937382-linux-musl-x64"
        );
    }

    #[test]
    fn release_rejects_duplicate_architecture_assets() {
        let release = release_with_assets(&[
            "sms-relayed-0937382-linux-musl-x64",
            "sms-relayed-0937382-linux-musl-x64",
        ]);

        let error = select_release_asset(&release, "linux-musl-x64")
            .expect_err("duplicate exact assets should fail");

        assert!(error.to_string().contains("expected exactly one"));

        let missing = release_with_assets(&[]);
        assert!(select_release_asset(&missing, "linux-musl-x64").is_err());
    }

    #[test]
    fn current_release_does_not_require_an_asset() {
        let release = release_with_assets(&[]);

        let asset = release_asset_for_update(
            &release,
            "linux-musl-x64",
            "09373820cd4ab63023359acf300d708d47c9f509",
        )
        .expect("the current release should be a successful no-op");

        assert!(asset.is_none());
    }

    #[test]
    fn unsupported_platforms_and_architectures_are_rejected() {
        assert_eq!(
            asset_suffix_for("linux", "x86_64").unwrap(),
            "linux-musl-x64"
        );
        assert_eq!(
            asset_suffix_for("linux", "aarch64").unwrap(),
            "linux-musl-aarch64"
        );
        assert!(asset_suffix_for("macos", "aarch64").is_err());
        assert!(asset_suffix_for("linux", "arm").is_err());
    }

    #[test]
    fn service_definitions_reveal_the_registered_binary() {
        let openwrt = r#"
start_service() {
  procd_set_param command "/opt/sms relayed/sms-relayed" run --config /etc/sms-relayed/config.toml
}
"#;
        let systemd = "{ path=/usr/local/bin/sms-relayed ; argv[]=/usr/local/bin/sms-relayed run ; ignore_errors=no ; }";

        assert_eq!(
            parse_openwrt_command(openwrt).as_deref(),
            Some(Path::new("/opt/sms relayed/sms-relayed"))
        );
        assert_eq!(
            parse_systemd_exec_start(systemd).as_deref(),
            Some(Path::new("/usr/local/bin/sms-relayed"))
        );
        assert_eq!(parse_systemd_exec_start("not configured"), None);
    }

    #[test]
    fn current_release_is_detected_from_commit_metadata() {
        let release = release_with_assets(&[]);

        assert!(release_matches_commit(
            &release,
            "09373820cd4ab63023359acf300d708d47c9f509"
        ));
        assert!(!release_matches_commit(
            &release,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ));

        let release_with_branch_target = Release {
            target_commitish: "main".to_string(),
            ..release
        };
        assert!(release_matches_commit(
            &release_with_branch_target,
            "09373820cd4ab63023359acf300d708d47c9f509"
        ));
    }

    #[test]
    fn downloaded_binary_must_report_the_release_commit() {
        let release = release_with_assets(&[]);

        assert!(version_output_matches_release(
            "sms-relayed 1.0.7+09373820cd4ab63023359acf300d708d47c9f509\n",
            &release
        ));
        assert!(!version_output_matches_release(
            "sms-relayed 1.0.7+aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
            &release
        ));
        assert!(!version_output_matches_release(
            "not-sms-relayed 1.0.7+09373820cd4ab63023359acf300d708d47c9f509\n",
            &release
        ));
        assert!(!version_output_matches_release(
            "sms-relayed +09373820cd4ab63023359acf300d708d47c9f509\n",
            &release
        ));
    }

    #[test]
    fn service_binary_wins_and_symlinks_resolve_to_the_real_file() {
        let directory = TestDirectory::new();
        let service_binary = directory.path().join("service-real");
        let service_link = directory.path().join("service-link");
        let path_binary = directory.path().join("path-binary");
        let current_binary = directory.path().join("current-binary");
        for file in [&service_binary, &path_binary, &current_binary] {
            fs::write(file, b"fixture").unwrap();
        }
        symlink(&service_binary, &service_link).unwrap();

        let selected = select_update_target([
            Some(service_link),
            Some(path_binary.clone()),
            Some(current_binary.clone()),
        ])
        .expect("a service binary should be selected");

        assert_eq!(selected, service_binary.canonicalize().unwrap());
        assert_eq!(
            select_update_target([
                None,
                Some(path_binary.clone()),
                Some(current_binary.clone()),
            ])
            .unwrap(),
            path_binary.canonicalize().unwrap()
        );
        assert_eq!(
            select_update_target([None, None, Some(current_binary.clone())]).unwrap(),
            current_binary.canonicalize().unwrap()
        );
    }

    #[test]
    fn permission_errors_recommend_elevated_privileges() {
        let error = update_io_error(
            io::Error::from(io::ErrorKind::PermissionDenied),
            Path::new("/usr/bin/sms-relayed"),
        );

        assert!(error.to_string().contains("sudo"));
        assert!(error.to_string().contains("/usr/bin/sms-relayed"));
    }

    #[tokio::test]
    async fn openwrt_service_takes_priority_without_calling_systemd() {
        let directory = TestDirectory::new();
        let service = directory.path().join("sms-relayed-init");
        fs::write(
            &service,
            "procd_set_param command /opt/sms-relayed run --config /etc/sms-relayed/config.toml\n",
        )
        .unwrap();
        let executor = FakeCommandExecutor::default();

        let detected = detect_service_with(&service, &executor).await;

        assert_eq!(detected.0, Some(ServiceManager::OpenWrt));
        assert_eq!(detected.1.as_deref(), Some(Path::new("/opt/sms-relayed")));
        assert!(executor.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn systemd_service_is_used_when_openwrt_is_absent() {
        let executor = FakeCommandExecutor::with_responses(vec![
            command_result(true, "loaded\n"),
            command_result(
                true,
                "{ path=/usr/bin/sms-relayed ; argv[]=/usr/bin/sms-relayed run ; }\n",
            ),
        ]);

        let detected = detect_service_with(Path::new("/definitely/missing"), &executor).await;

        assert_eq!(detected.0, Some(ServiceManager::Systemd));
        assert_eq!(
            detected.1.as_deref(),
            Some(Path::new("/usr/bin/sms-relayed"))
        );
        assert_eq!(executor.calls.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn restart_failure_is_an_error_but_no_service_is_successful() {
        let failing = FakeCommandExecutor::with_responses(vec![command_result(false, "")]);
        let error = restart_after_update(Some(ServiceManager::Systemd), &failing)
            .await
            .expect_err("a failed service restart should fail the command");
        assert!(error.to_string().contains("binary updated"));
        assert!(error.to_string().contains("sudo"));

        let no_commands = FakeCommandExecutor::default();
        let outcome = restart_after_update(None, &no_commands)
            .await
            .expect("missing services should be a successful outcome");
        assert_eq!(outcome, RestartOutcome::NoService);
        assert!(no_commands.calls.lock().unwrap().is_empty());
    }

    async fn serve_binary(contents: &'static str) -> String {
        let app = Router::new().route("/binary", get(move || async move { contents }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{address}/binary")
    }

    async fn serve_interrupted_binary() -> String {
        let app = Router::new().route(
            "/binary",
            get(|| async {
                let chunks = vec![
                    Ok::<Bytes, io::Error>(Bytes::from_static(b"partial binary")),
                    Err(io::Error::new(
                        io::ErrorKind::ConnectionReset,
                        "fixture interrupted download",
                    )),
                ];
                Response::new(Body::from_stream(stream::iter(chunks)))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{address}/binary")
    }

    fn update_fixture(asset_url: String) -> (Release, ReleaseAsset) {
        (
            release_with_assets(&[]),
            ReleaseAsset {
                name: "sms-relayed-0937382-linux-musl-x64".to_string(),
                browser_download_url: asset_url,
            },
        )
    }

    #[tokio::test]
    async fn valid_download_atomically_replaces_the_installed_binary() {
        const BINARY: &str = "#!/bin/sh\nprintf '%s\\n' 'sms-relayed 1.0.7+09373820cd4ab63023359acf300d708d47c9f509'\n";
        let asset_url = serve_binary(BINARY).await;
        let (release, asset) = update_fixture(asset_url);
        let directory = TestDirectory::new();
        let target = directory.path().join("sms-relayed");
        fs::write(&target, b"old binary").unwrap();

        download_and_replace(&reqwest::Client::new(), &release, &asset, &target)
            .await
            .expect("a valid release binary should replace the target");

        assert_eq!(fs::read_to_string(&target).unwrap(), BINARY);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn invalid_download_preserves_the_installed_binary_and_cleans_up() {
        const WRONG_BINARY: &str = "#!/bin/sh\nprintf '%s\\n' 'sms-relayed 1.0.7+aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'\n";
        let asset_url = serve_binary(WRONG_BINARY).await;
        let (release, asset) = update_fixture(asset_url);
        let directory = TestDirectory::new();
        let target = directory.path().join("sms-relayed");
        fs::write(&target, b"old binary").unwrap();

        let error = download_and_replace(&reqwest::Client::new(), &release, &asset, &target)
            .await
            .expect_err("a mismatched release binary should be rejected");

        assert!(error.to_string().contains("version validation"));
        assert_eq!(fs::read(&target).unwrap(), b"old binary");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn interrupted_download_preserves_the_installed_binary_and_cleans_up() {
        let asset_url = serve_interrupted_binary().await;
        let (release, asset) = update_fixture(asset_url);
        let directory = TestDirectory::new();
        let target = directory.path().join("sms-relayed");
        fs::write(&target, b"old binary").unwrap();

        download_and_replace(&reqwest::Client::new(), &release, &asset, &target)
            .await
            .expect_err("an interrupted release download should fail");

        assert_eq!(fs::read(&target).unwrap(), b"old binary");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
