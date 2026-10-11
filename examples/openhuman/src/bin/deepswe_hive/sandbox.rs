//! Fail-closed Docker confinement for agent actions and patch inspection.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;
use std::{error, fmt};

use wait_timeout::ChildExt;

use super::task::GitLayout;
#[path = "sandbox/cleanup.rs"]
mod cleanup;
#[path = "sandbox/preflight.rs"]
mod preflight;
#[cfg(test)]
#[path = "sandbox/test_support.rs"]
mod test_support;

use cleanup::{cleanup_container, finish_with_cleanup, read_bounded, terminate};
#[cfg(test)]
pub(super) use preflight::preflight_with_limits;
#[cfg(test)]
pub(super) use test_support::shell_with_limits_at;

const DEFAULT_IMAGE: &str = "tinyhivemind-deepswe:local";
// Keep both Docker deadlines inside the 600-second model-turn deadline so the
// MCP server retains time to remove a timed-out container before it is stopped.
const COMMAND_TIMEOUT_SECONDS: &str = "540";
const ACTION_TIMEOUT: Duration = Duration::from_secs(570);
pub(super) const INSPECTOR_TIMEOUT: Duration = Duration::from_secs(600);
pub(super) const MAX_PATCH_BYTES: u64 = 32 * 1024 * 1024;
pub(super) const MAX_ACTION_OUTPUT_BYTES: u64 = 1024 * 1024;
pub(super) const MAX_ACTION_INPUT_BYTES: usize = 1024 * 1024;
const CONTAINER_INPUT: &str = "/tmp/deepswe-input";
const CONTAINER_MEMORY: &str = "1g";
const CONTAINER_CPUS: &str = "2";
const CONTAINER_PIDS: &str = "256";
const CLEAN_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

#[derive(Debug, Eq, PartialEq)]
pub(super) enum InspectorFailure {
    TimedOut { seconds: u64 },
    PatchTooLarge { max_bytes: u64 },
    ActionInputTooLarge { max_bytes: usize },
    ActionOutputTooLarge { max_bytes: u64 },
    CleanupFailed { detail: String },
    CleanupTimeout { seconds: u64 },
    PreflightTimedOut { phase: &'static str },
    PreflightOutputTooLarge { phase: &'static str, max_bytes: u64 },
}

impl fmt::Display for InspectorFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TimedOut { seconds } => {
                write!(
                    formatter,
                    "Docker inspector timed out after {seconds} seconds"
                )
            }
            Self::PatchTooLarge { max_bytes } => {
                write!(formatter, "patch exceeds {max_bytes}-byte limit")
            }
            Self::ActionInputTooLarge { max_bytes } => {
                write!(formatter, "action input exceeds {max_bytes}-byte limit")
            }
            Self::ActionOutputTooLarge { max_bytes } => {
                write!(formatter, "action output exceeds {max_bytes}-byte limit")
            }
            Self::CleanupFailed { detail } => {
                write!(formatter, "Docker container cleanup failed: {detail}")
            }
            Self::CleanupTimeout { seconds } => {
                write!(
                    formatter,
                    "Docker container cleanup timed out after {seconds} seconds"
                )
            }
            Self::PreflightTimedOut { phase } => {
                write!(formatter, "Docker {phase} preflight timed out")
            }
            Self::PreflightOutputTooLarge { phase, max_bytes } => {
                write!(
                    formatter,
                    "Docker {phase} preflight output exceeds {max_bytes}-byte limit"
                )
            }
        }
    }
}

impl error::Error for InspectorFailure {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SandboxConfig {
    pub(super) repo_path: PathBuf,
    pub(super) image: String,
    pub(super) docker: PathBuf,
}

impl SandboxConfig {
    pub(super) fn from_env(repo_path: PathBuf) -> Self {
        let docker = std::env::var_os("DEEPSWE_DOCKER_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| resolve_executable("docker"));
        Self {
            repo_path,
            image: std::env::var("DEEPSWE_DOCKER_IMAGE").unwrap_or_else(|_| DEFAULT_IMAGE.into()),
            docker,
        }
    }
}

fn resolve_executable(name: &str) -> PathBuf {
    let Some(path) = std::env::var_os("PATH") else {
        return PathBuf::from(name);
    };
    std::env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| PathBuf::from(name))
}

#[derive(Clone, Debug)]
pub(super) struct DockerSandbox {
    config: SandboxConfig,
    mask: Arc<tempfile::TempDir>,
    git: GitLayout,
    user: String,
}

#[derive(Debug)]
pub(super) struct ShellOutput {
    pub(super) code: Option<i32>,
    pub(super) stdout: String,
    pub(super) stderr: String,
}

impl DockerSandbox {
    pub(super) fn shell(&self, script: &str) -> anyhow::Result<ShellOutput> {
        self.action(
            ["timeout", COMMAND_TIMEOUT_SECONDS, "sh", "-lc", script],
            &[],
        )
    }

    pub(super) fn file_read(&self, path: &str) -> anyhow::Result<String> {
        validate_relative(path)?;
        let output = self.action(
            [
                "sh",
                "-c",
                "target=/workspace/$1; resolved=$(realpath -m -- \"$target\") || exit; case \"$resolved\" in /workspace/*) cat -- \"$target\" ;; *) exit 1 ;; esac",
                "deepswe-read",
                path,
            ],
            &[],
        )?;
        require_success(output, "read file").map(|output| output.stdout)
    }

    pub(super) fn file_write(&self, path: &str, content: &str) -> anyhow::Result<()> {
        validate_relative(path)?;
        let output = self.action(
            [
                "sh",
                "-c",
                "target=/workspace/$1; resolved=$(realpath -m -- \"$target\") || exit; case \"$resolved\" in /workspace/*) mkdir -p -- \"$(dirname -- \"$target\")\" && cat > \"$target\" ;; *) exit 1 ;; esac",
                "deepswe-write",
                path,
            ],
            content.as_bytes(),
        )?;
        require_success(output, "write file").map(|_| ())
    }

    pub(super) fn file_edit(&self, path: &str, old: &str, new: &str) -> anyhow::Result<()> {
        let text = self.file_read(path)?;
        if old.is_empty() || text.matches(old).count() != 1 {
            anyhow::bail!("old must match exactly once");
        }
        self.file_write(path, &text.replacen(old, new, 1))
    }

    pub(super) fn patch(&self, base_commit: &str) -> anyhow::Result<String> {
        self.patch_with_limits(base_commit, INSPECTOR_TIMEOUT, MAX_PATCH_BYTES)
    }

    fn patch_with_limits(
        &self,
        base_commit: &str,
        timeout: Duration,
        max_bytes: u64,
    ) -> anyhow::Result<String> {
        let patch_file = tempfile::NamedTempFile::new()?;
        self.patch_with_limits_at(base_commit, timeout, max_bytes, patch_file.path())
    }

    fn patch_with_limits_at(
        &self,
        base_commit: &str,
        timeout: Duration,
        max_bytes: u64,
        patch_file: &Path,
    ) -> anyhow::Result<String> {
        let script = r#"set +e
set -o pipefail
{
  git -c core.hooksPath=/dev/null -c core.fsmonitor=false -c diff.external= -c core.quotePath=true diff --binary --no-ext-diff --no-textconv "$1" -- . || exit "$?"
  git -c core.hooksPath=/dev/null -c core.fsmonitor=false ls-files --others --exclude-standard -z |
    LC_ALL=C sort -z |
    while IFS= read -r -d '' file; do
      git -c core.hooksPath=/dev/null -c core.fsmonitor=false -c diff.external= -c core.quotePath=true diff --binary --no-ext-diff --no-textconv --no-index -- /dev/null "$file" || test "$?" -eq 1
    done
} | head -c "$2" > /tmp/deepswe.patch
statuses=("${PIPESTATUS[@]}")
test "${statuses[1]}" -eq 0 || exit "${statuses[1]}"
exit "${statuses[0]}"
"#;
        let cap = max_bytes.saturating_add(1).to_string();
        let mut args = inspector_args(self)?;
        add_mount(&mut args, mount(patch_file, "/tmp/deepswe.patch", true)?)?;
        let output = run_container_output(
            &self.config,
            args,
            &["bash", "-c", script, "deepswe-patch", base_commit, &cap],
            &[],
            timeout,
            64 * 1024,
        )?;
        let patch = read_bounded(patch_file, max_bytes)?;
        if patch.len() as u64 > max_bytes {
            return Err(InspectorFailure::PatchTooLarge { max_bytes }.into());
        }
        require_success(output, "capture final patch")?;
        Ok(String::from_utf8(patch)?)
    }
    pub(super) fn mcp_args(&self) -> Vec<String> {
        vec![
            "--mcp-workspace".into(),
            "--repo".into(),
            self.config.repo_path.display().to_string(),
            "--image".into(),
            self.config.image.clone(),
            "--docker".into(),
            self.config.docker.display().to_string(),
            "--dot-git-kind".into(),
            if self.git.dot_git_is_file {
                "file".into()
            } else {
                "directory".into()
            },
            "--git-dir".into(),
            self.git.git_dir.display().to_string(),
            "--git-common-dir".into(),
            self.git.common_dir.display().to_string(),
        ]
    }

    fn action<const N: usize>(&self, args: [&str; N], input: &[u8]) -> anyhow::Result<ShellOutput> {
        self.action_with_limits(args, input, ACTION_TIMEOUT, MAX_ACTION_OUTPUT_BYTES)
    }

    fn action_with_limits<const N: usize>(
        &self,
        args: [&str; N],
        input: &[u8],
        timeout: Duration,
        max_bytes: u64,
    ) -> anyhow::Result<ShellOutput> {
        let output = tempfile::NamedTempFile::new()?;
        self.action_with_limits_at(args, input, timeout, max_bytes, output.path())
    }

    fn action_with_limits_at<const N: usize>(
        &self,
        args: [&str; N],
        input: &[u8],
        timeout: Duration,
        max_bytes: u64,
        output: &Path,
    ) -> anyhow::Result<ShellOutput> {
        if input.len() > MAX_ACTION_INPUT_BYTES {
            return Err(InspectorFailure::ActionInputTooLarge {
                max_bytes: MAX_ACTION_INPUT_BYTES,
            }
            .into());
        }
        let mut container_args = action_args(self, false)?;
        add_mount(
            &mut container_args,
            mount(output, "/tmp/deepswe-action-output", true)?,
        )?;
        let cap = max_bytes.saturating_add(1).to_string();
        let wrapper = r#"set +e
cap=$1
shift
"$@" 2>&1 | head -c "$cap" > /tmp/deepswe-action-output
statuses=("${PIPESTATUS[@]}")
test "${statuses[1]}" -eq 0 || exit "${statuses[1]}"
exit "${statuses[0]}"
"#;
        let mut command_args = vec!["bash", "-c", wrapper, "deepswe-action", &cap];
        command_args.extend(args);
        let status = run_container(&self.config, container_args, &command_args, input, timeout)?;
        let bytes = read_bounded(output, max_bytes)?;
        if bytes.len() as u64 > max_bytes {
            return Err(InspectorFailure::ActionOutputTooLarge { max_bytes }.into());
        }
        Ok(ShellOutput {
            code: status.code,
            stdout: String::from_utf8(bytes)?,
            stderr: status.stderr,
        })
    }
}

fn prepare_mask(mask: &tempfile::TempDir, file: bool) -> anyhow::Result<()> {
    let path = mask.path().join("git-mask");
    if file {
        std::fs::write(path, [])?;
    } else {
        std::fs::create_dir(path)?;
    }
    Ok(())
}

fn mask_source(sandbox: &DockerSandbox) -> PathBuf {
    sandbox.mask.path().join("git-mask")
}

fn action_args(sandbox: &DockerSandbox, create: bool) -> anyhow::Result<Vec<String>> {
    Ok(container_args(
        &sandbox.config,
        &sandbox.user,
        false,
        [
            mount(&sandbox.config.repo_path, "/workspace", true)?,
            mount(&mask_source(sandbox), "/workspace/.git", false)?,
        ],
        create,
    ))
}

fn inspector_args(sandbox: &DockerSandbox) -> anyhow::Result<Vec<String>> {
    let mut mounts = vec![mount(&sandbox.config.repo_path, "/workspace", false)?];
    mounts.push(mount(
        &sandbox.config.repo_path.join(".git"),
        "/workspace/.git",
        false,
    )?);
    let standard_git_dir = sandbox.config.repo_path.join(".git").canonicalize().ok();
    if standard_git_dir.as_ref() != Some(&sandbox.git.git_dir) {
        mounts.push(mount(
            &sandbox.git.git_dir,
            &sandbox.git.git_dir.display().to_string(),
            false,
        )?);
    }
    if sandbox.git.common_dir != sandbox.git.git_dir
        && standard_git_dir.as_ref() != Some(&sandbox.git.common_dir)
    {
        mounts.push(mount(
            &sandbox.git.common_dir,
            &sandbox.git.common_dir.display().to_string(),
            false,
        )?);
    }
    Ok(container_args(
        &sandbox.config,
        &sandbox.user,
        false,
        mounts,
        false,
    ))
}

fn container_args(
    config: &SandboxConfig,
    user: &str,
    remove: bool,
    mounts: impl IntoIterator<Item = String>,
    create: bool,
) -> Vec<String> {
    let mut args = vec![if create { "create" } else { "run" }.into()];
    if remove {
        args.push("--rm".into());
    }
    args.extend([
        "--network".into(),
        "none".into(),
        "--memory".into(),
        CONTAINER_MEMORY.into(),
        "--cpus".into(),
        CONTAINER_CPUS.into(),
        "--pids-limit".into(),
        CONTAINER_PIDS.into(),
        "--cap-drop".into(),
        "ALL".into(),
        "--security-opt".into(),
        "no-new-privileges".into(),
        "--user".into(),
        user.into(),
    ]);
    for value in mounts {
        args.extend(["--mount".into(), value]);
    }
    args.extend([
        "--workdir".into(),
        "/workspace".into(),
        config.image.clone(),
        "env".into(),
        "-i".into(),
        "HOME=/tmp".into(),
        format!("PATH={CLEAN_PATH}"),
    ]);
    args
}

pub(super) fn mount(source: &Path, destination: &str, writable: bool) -> anyhow::Result<String> {
    let source = source.display().to_string();
    if source.contains(',') || destination.contains(',') {
        anyhow::bail!("Docker bind mount paths must not contain commas")
    }
    let readonly = if writable { "" } else { ",readonly" };
    Ok(format!(
        "type=bind,src={source},dst={destination}{readonly}"
    ))
}

fn run_container_output(
    config: &SandboxConfig,
    container_args: Vec<String>,
    args: &[&str],
    input: &[u8],
    timeout: Duration,
    max_bytes: u64,
) -> anyhow::Result<ShellOutput> {
    let status = run_container(config, container_args, args, input, timeout)?;
    if status.stdout.len() as u64 > max_bytes || status.stderr.len() as u64 > max_bytes {
        return Err(InspectorFailure::ActionOutputTooLarge { max_bytes }.into());
    }
    Ok(ShellOutput {
        code: status.code,
        stdout: String::from_utf8(status.stdout)?,
        stderr: status.stderr,
    })
}

struct ContainerStatus {
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: String,
}

fn run_container(
    config: &SandboxConfig,
    mut container_args: Vec<String>,
    args: &[&str],
    input: &[u8],
    timeout: Duration,
) -> anyhow::Result<ContainerStatus> {
    let control = tempfile::tempdir()?;
    let cidfile = control.path().join("container.cid");
    let container_name = format!(
        "deepswe-{}",
        control
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow::anyhow!("temporary control directory has no UTF-8 name"))?
    );
    if container_args.first().is_some_and(|arg| arg == "run") {
        container_args.splice(
            1..1,
            [
                "--cidfile".into(),
                cidfile.display().to_string(),
                "--name".into(),
                container_name.clone(),
            ],
        );
    }
    if input.len() > MAX_ACTION_INPUT_BYTES {
        return Err(InspectorFailure::ActionInputTooLarge {
            max_bytes: MAX_ACTION_INPUT_BYTES,
        }
        .into());
    }
    let mut staged_input = tempfile::NamedTempFile::new()?;
    staged_input.write_all(input)?;
    staged_input.as_file_mut().sync_all()?;
    add_mount(
        &mut container_args,
        mount(staged_input.path(), CONTAINER_INPUT, false)?,
    )?;
    let stdout = tempfile::NamedTempFile::new()?;
    let stderr = tempfile::NamedTempFile::new()?;
    let mut child = command(config)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout.reopen()?))
        .stderr(Stdio::from(stderr.reopen()?))
        .args(container_args)
        .args([
            "sh",
            "-c",
            "exec \"$@\" < /tmp/deepswe-input",
            "deepswe-input",
        ])
        .args(args)
        .spawn()?;
    let result = match child.wait_timeout(timeout) {
        Ok(Some(status)) => Ok(status),
        Ok(None) => {
            terminate(&mut child);
            Err(InspectorFailure::TimedOut {
                seconds: timeout.as_secs(),
            }
            .into())
        }
        Err(error) => {
            terminate(&mut child);
            Err(error.into())
        }
    };
    let cleanup = cleanup_container(config, &cidfile, &container_name);
    let status = finish_with_cleanup(result, cleanup)?;
    let stdout_bytes = read_bounded(stdout.path(), MAX_ACTION_OUTPUT_BYTES)?;
    let stderr_bytes = read_bounded(stderr.path(), MAX_ACTION_OUTPUT_BYTES)?;
    Ok(ContainerStatus {
        code: status.code(),
        stdout: stdout_bytes,
        stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
    })
}

fn add_mount(args: &mut Vec<String>, value: String) -> anyhow::Result<()> {
    let position = args
        .iter()
        .position(|arg| arg == "--workdir")
        .ok_or_else(|| anyhow::anyhow!("Docker arguments have no workdir boundary"))?;
    args.splice(position..position, ["--mount".into(), value]);
    Ok(())
}

fn require_success(output: ShellOutput, action: &str) -> anyhow::Result<ShellOutput> {
    if output.code != Some(0) {
        let detail = if output.stderr.trim().is_empty() {
            output.stdout.trim()
        } else {
            output.stderr.trim()
        };
        anyhow::bail!("{action} failed: {detail}");
    }
    Ok(output)
}

fn validate_relative(value: &str) -> anyhow::Result<()> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        anyhow::bail!("path must be a simple relative path inside /workspace");
    }
    Ok(())
}

fn command(config: &SandboxConfig) -> Command {
    let mut command = Command::new(&config.docker);
    command
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

#[cfg(test)]
pub(super) fn action_arguments(sandbox: &DockerSandbox) -> anyhow::Result<Vec<String>> {
    action_args(sandbox, false)
}

#[cfg(test)]
pub(super) fn patch_with_limits(
    sandbox: &DockerSandbox,
    base_commit: &str,
    timeout: Duration,
    max_bytes: u64,
) -> anyhow::Result<String> {
    sandbox.patch_with_limits(base_commit, timeout, max_bytes)
}

#[cfg(test)]
pub(super) fn patch_with_limits_at(
    sandbox: &DockerSandbox,
    base_commit: &str,
    timeout: Duration,
    max_bytes: u64,
    patch_file: &Path,
) -> anyhow::Result<String> {
    sandbox.patch_with_limits_at(base_commit, timeout, max_bytes, patch_file)
}

#[cfg(test)]
pub(super) fn shell_with_limits(
    sandbox: &DockerSandbox,
    script: &str,
    timeout: Duration,
    max_bytes: u64,
) -> anyhow::Result<ShellOutput> {
    sandbox.action_with_limits(
        ["timeout", COMMAND_TIMEOUT_SECONDS, "sh", "-lc", script],
        &[],
        timeout,
        max_bytes,
    )
}
