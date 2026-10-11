//! Optional real Docker confinement and patch-capture integration proof.
use super::*;

#[test]
fn live_real_docker_fixture_flow_confines_actions_and_captures_new_files() {
    if std::env::var_os("DEEPSWE_REAL_DOCKER_TEST").is_none() {
        return;
    }
    let _docker_guard = real_docker_test_guard();
    let directory = TempDir::new().expect("disposable fixture");
    copy_fixture(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/deepswe_repo"),
        directory.path(),
    );
    git(directory.path(), &["init", "-q"]);
    git(
        directory.path(),
        &["config", "user.email", "fixture@example.invalid"],
    );
    git(
        directory.path(),
        &["config", "user.name", "DeepSWE Fixture"],
    );
    git(directory.path(), &["add", "."]);
    git(directory.path(), &["commit", "-qm", "fixture"]);
    let head = git_output(directory.path(), &["rev-parse", "HEAD"]);
    let task = Task {
        instance_id: "docker-fixture".into(),
        repo_path: directory.path().to_path_buf(),
        base_commit: head.trim().into(),
        problem_statement: "Make the fixture pass".into(),
        test_command: "sh test.sh".into(),
    };
    task.validate().expect("valid checked-in fixture copy");
    let sandbox = DockerSandbox::preflight(SandboxConfig::from_env(task.repo_path.clone()))
        .expect("real Docker preflight");
    let args = action_arguments(&sandbox).expect("mount arguments");
    assert!(args.windows(2).any(|pair| pair == ["--network", "none"]));
    assert!(
        args.iter()
            .any(|arg| arg.contains("dst=/workspace/.git,readonly"))
    );
    assert!(args.windows(2).any(|pair| pair == ["--memory", "1g"]));
    assert!(args.windows(2).any(|pair| pair == ["--cpus", "2"]));
    assert!(args.windows(2).any(|pair| pair == ["--pids-limit", "256"]));
    sandbox
        .file_write("new.txt", "all new\n")
        .expect("container new file");
    let new_only_patch = sandbox
        .patch(&task.base_commit)
        .expect("all-new-file patch");
    assert!(new_only_patch.contains("diff --git a/new.txt b/new.txt"));
    assert!(!new_only_patch.contains("answer.txt"));
    sandbox
        .shell("truncate -s 1048576 huge-sparse.bin")
        .expect("large sparse file");
    let oversized = patch_with_limits(&sandbox, &task.base_commit, Duration::from_secs(10), 128)
        .expect_err("oversized patch rejected");
    assert!(oversized.to_string().contains("exceeds 128-byte limit"));
    assert_eq!(
        oversized.downcast_ref::<InspectorFailure>(),
        Some(&InspectorFailure::PatchTooLarge { max_bytes: 128 })
    );
    std::fs::remove_file(directory.path().join("huge-sparse.bin")).expect("remove sparse fixture");
    sandbox
        .file_edit("answer.txt", "wrong", "right")
        .expect("container edit");
    assert_eq!(
        sandbox.file_read("new.txt").expect("container read"),
        "all new\n"
    );
    let sentinel = directory
        .path()
        .parent()
        .expect("parent")
        .join("host-sentinel");
    std::fs::write(&sentinel, "safe\n").expect("sentinel");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&sentinel, directory.path().join("escape")).expect("escape symlink");
    assert!(sandbox.file_write("escape", "damaged\n").is_err());
    assert_eq!(
        std::fs::read_to_string(&sentinel).expect("sentinel read"),
        "safe\n"
    );
    let env = sandbox.shell("env").expect("container env");
    assert!(!env.stdout.contains("OPENROUTER"));
    assert!(!env.stdout.contains("API_KEY"));
    assert!(!env.stdout.contains("DEEPSWE_TEST_SECRET"));
    let history = sandbox
        .shell("git reset --hard HEAD; git clean -fdx; git commit --allow-empty -m pwn")
        .expect("history commands confined");
    assert_ne!(history.code, Some(0));
    assert_eq!(git_output(directory.path(), &["rev-parse", "HEAD"]), head);
    let network = sandbox
        .shell("test ! -e /sys/class/net/eth0 || ! ip route 2>/dev/null | grep -q default")
        .expect("network probe");
    assert_eq!(network.code, Some(0));
    let test = sandbox.shell(&task.test_command).expect("final test");
    assert_eq!(test.code, Some(0));
    let marker = directory
        .path()
        .parent()
        .expect("parent")
        .join("diff-driver-ran");
    std::fs::write(
        directory.path().join(".gitattributes"),
        "answer.txt diff=hostile\n",
    )
    .expect("attributes");
    git(
        directory.path(),
        &[
            "config",
            "diff.hostile.command",
            &format!("touch {}", marker.display()),
        ],
    );
    let patch = sandbox.patch(&task.base_commit).expect("inspector patch");
    assert!(patch.contains("+right"));
    assert!(patch.contains("diff --git a/new.txt b/new.txt"));
    assert!(
        !marker.exists(),
        "repository diff command must not run on the host"
    );
    assert_eq!(git_output(directory.path(), &["rev-parse", "HEAD"]), head);

    let linked_parent = TempDir::new().expect("worktree parent");
    let linked = linked_parent.path().join("linked");
    let linked_text = linked.to_string_lossy().into_owned();
    git(
        directory.path(),
        &["worktree", "add", "-qb", "linked-fixture", &linked_text],
    );
    assert!(linked.join(".git").is_file());
    let linked_head = git_output(&linked, &["rev-parse", "HEAD"]);
    let linked_task = Task {
        instance_id: "docker-worktree-fixture".into(),
        repo_path: linked.clone(),
        base_commit: linked_head.trim().into(),
        problem_statement: "Add a file without exposing worktree history".into(),
        test_command: "test -f linked-new.txt".into(),
    };
    linked_task
        .validate()
        .expect("valid linked worktree fixture");
    let linked_sandbox =
        DockerSandbox::preflight(SandboxConfig::from_env(linked_task.repo_path.clone()))
            .expect("worktree Docker preflight");
    linked_sandbox
        .file_write("linked-new.txt", "linked\n")
        .expect("worktree new file");
    assert_ne!(
        linked_sandbox
            .shell("git reset --hard HEAD")
            .expect("worktree history probe")
            .code,
        Some(0)
    );
    let linked_patch = linked_sandbox
        .patch(&linked_task.base_commit)
        .expect("worktree inspector patch");
    assert!(linked_patch.contains("diff --git a/linked-new.txt b/linked-new.txt"));
    assert_eq!(git_output(&linked, &["rev-parse", "HEAD"]), linked_head);
}
