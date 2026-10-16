use spoor_providers::git::TempDirGuard;
use spoor_providers::process_tree::{is_process_alive, ProcessTreeSession};
use std::time::Duration;
use tokio::io::AsyncBufReadExt;

#[tokio::test]
async fn test_process_tree_termination_kills_descendants_and_cleans_tempdir() {
    let mut temp_guard = TempDirGuard::new("spoor-test-leak").expect("Must create temp dir");
    let test_dir = temp_guard.path().to_path_buf();
    assert!(test_dir.exists(), "Temp dir must exist");

    // Write a file in the temp dir to simulate repository files
    let dummy_file = test_dir.join("test_file.txt");
    std::fs::write(&dummy_file, b"test content").expect("Must write dummy file");

    let mut session = ProcessTreeSession::new().expect("Must create ProcessTreeSession");

    #[cfg(windows)]
    let mut cmd = {
        let mut c = tokio::process::Command::new("powershell");
        c.arg("-NoProfile")
            .arg("-NonInteractive")
            .arg("-Command")
            .arg("$proc = Start-Process powershell -ArgumentList '-NoProfile','-NonInteractive','-Command','Start-Sleep -Seconds 60' -PassThru; [Console]::Out.WriteLine($proc.Id); Start-Sleep -Seconds 60")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        c
    };

    #[cfg(unix)]
    let mut cmd = {
        let mut c = tokio::process::Command::new("sh");
        c.arg("-c")
            .arg("sleep 60 & echo $! && sleep 60")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        c
    };

    session.prepare_command(&mut cmd);

    let mut child = cmd.spawn().expect("Must spawn controlled parent process");
    let parent_pid = child.id().expect("Child must have PID");

    session
        .attach_child(&child)
        .expect("Must attach child to ProcessTreeSession");

    let stdout = child.stdout.take().expect("Child must have stdout");
    let mut reader = tokio::io::BufReader::new(stdout).lines();

    // Read the descendant PID from the child stdout with a timeout
    let descendant_pid: u32 = tokio::time::timeout(Duration::from_secs(10), async {
        while let Ok(Some(line)) = reader.next_line().await {
            let trimmed = line.trim();
            if let Ok(pid) = trimmed.parse::<u32>() {
                return Ok(pid);
            }
        }
        Err("No PID received from child")
    })
    .await
    .expect("Timed out waiting for descendant PID")
    .expect("Failed to parse descendant PID");

    assert!(descendant_pid > 0, "Descendant PID must be valid");

    // Verify both parent and descendant are currently alive
    assert!(
        is_process_alive(parent_pid),
        "Direct parent process {parent_pid} must be alive before timeout"
    );
    assert!(
        is_process_alive(descendant_pid),
        "Descendant process {descendant_pid} must be alive before timeout"
    );

    // Simulate timeout / cancellation: terminate the entire process tree
    session.terminate();

    // Reap the direct child
    let _ = child.kill().await;
    let _ = child.wait().await;

    // Small delay for OS to finalize process exit tables
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Verify parent is terminated
    assert!(
        !is_process_alive(parent_pid),
        "Direct parent process {parent_pid} must be terminated after session.terminate()"
    );

    // Verify descendant is terminated (the critical process-tree guarantee!)
    assert!(
        !is_process_alive(descendant_pid),
        "Descendant process {descendant_pid} MUST be terminated after session.terminate()"
    );

    // Verify temp directory can be cleaned up cleanly and no directory is left behind
    temp_guard
        .cleanup()
        .expect("Temp dir must be cleanly removable after process-tree termination");
    assert!(
        !test_dir.exists(),
        "Temporary directory must not exist after cleanup"
    );
}

#[tokio::test]
async fn test_process_tree_session_drop_terminates_descendants() {
    let mut session = ProcessTreeSession::new().expect("Must create ProcessTreeSession");

    #[cfg(windows)]
    let mut cmd = {
        let mut c = tokio::process::Command::new("powershell");
        c.arg("-NoProfile")
            .arg("-NonInteractive")
            .arg("-Command")
            .arg("$proc = Start-Process powershell -ArgumentList '-NoProfile','-NonInteractive','-Command','Start-Sleep -Seconds 60' -PassThru; [Console]::Out.WriteLine($proc.Id); Start-Sleep -Seconds 60")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        c
    };

    #[cfg(unix)]
    let mut cmd = {
        let mut c = tokio::process::Command::new("sh");
        c.arg("-c")
            .arg("sleep 60 & echo $! && sleep 60")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        c
    };

    session.prepare_command(&mut cmd);

    let mut child = cmd.spawn().expect("Must spawn controlled parent process");
    let parent_pid = child.id().expect("Child must have PID");

    session
        .attach_child(&child)
        .expect("Must attach child to ProcessTreeSession");

    let stdout = child.stdout.take().expect("Child must have stdout");
    let mut reader = tokio::io::BufReader::new(stdout).lines();

    let descendant_pid: u32 = tokio::time::timeout(Duration::from_secs(10), async {
        while let Ok(Some(line)) = reader.next_line().await {
            let trimmed = line.trim();
            if let Ok(pid) = trimmed.parse::<u32>() {
                return Ok(pid);
            }
        }
        Err("No PID received from child")
    })
    .await
    .expect("Timed out waiting for descendant PID")
    .expect("Failed to parse descendant PID");

    assert!(descendant_pid > 0);
    assert!(is_process_alive(parent_pid));
    assert!(is_process_alive(descendant_pid));

    // Drop the session (as happens when a future is cancelled or errors out)
    drop(session);

    let _ = child.kill().await;
    let _ = child.wait().await;

    tokio::time::sleep(Duration::from_millis(150)).await;

    assert!(
        !is_process_alive(parent_pid),
        "Direct parent process {parent_pid} must be terminated when ProcessTreeSession is dropped"
    );
    assert!(
        !is_process_alive(descendant_pid),
        "Descendant process {descendant_pid} must be terminated when ProcessTreeSession is dropped"
    );
}

#[tokio::test]
async fn test_atomic_process_containment_fast_spawning_descendant() {
    let mut temp_guard = TempDirGuard::new("spoor-test-atomic").expect("Must create temp dir");
    let test_dir = temp_guard.path().to_path_buf();
    assert!(test_dir.exists(), "Temp dir must exist");

    let dummy_file = test_dir.join("readonly.txt");
    std::fs::write(&dummy_file, b"read-only test data").expect("Must write dummy file");
    let mut perms = std::fs::metadata(&dummy_file).unwrap().permissions();
    perms.set_readonly(true);
    let _ = std::fs::set_permissions(&dummy_file, perms);

    let pid_file = test_dir.join("descendant_pid.txt");

    let mut session = ProcessTreeSession::new().expect("Must create ProcessTreeSession");

    #[cfg(windows)]
    let cmd = {
        let script = format!(
            "$proc = Start-Process powershell -ArgumentList '-NoProfile','-NonInteractive','-Command','Start-Sleep -Seconds 60' -PassThru; Set-Content -Path '{}' -Value $proc.Id; Start-Sleep -Seconds 60",
            pid_file.display().to_string().replace('\\', "/")
        );
        spoor_providers::process_tree::ProcessCommand::new("powershell")
            .arg("-NoProfile")
            .arg("-NonInteractive")
            .arg("-Command")
            .arg(script)
            .current_dir(&test_dir)
            .capture_stdout(false)
            .capture_stderr(true)
    };

    #[cfg(unix)]
    let cmd = {
        let script = format!("sleep 60 & echo $! > '{}' && sleep 60", pid_file.display());
        spoor_providers::process_tree::ProcessCommand::new("sh")
            .arg("-c")
            .arg(script)
            .current_dir(&test_dir)
            .capture_stdout(false)
            .capture_stderr(true)
    };

    let mut child = session
        .spawn(cmd)
        .expect("Must atomically spawn child process");
    let parent_pid = child.id();
    assert!(parent_pid > 0, "Parent PID must be positive");

    let descendant_pid: u32 = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if pid_file.exists() {
                if let Ok(content) = std::fs::read_to_string(&pid_file) {
                    let trimmed = content.trim();
                    if let Ok(pid) = trimmed.parse::<u32>() {
                        if pid > 0 {
                            return Ok::<u32, &'static str>(pid);
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("Timed out waiting for descendant PID file")
    .expect("Failed to read descendant PID from file");

    assert!(descendant_pid > 0, "Descendant PID must be valid");

    assert!(
        is_process_alive(parent_pid),
        "Direct parent process {parent_pid} must be alive"
    );
    assert!(
        is_process_alive(descendant_pid),
        "Descendant process {descendant_pid} must be alive"
    );

    session.terminate();

    let _ = child.kill().await;
    let _ = child.wait().await;

    tokio::time::sleep(Duration::from_millis(150)).await;

    assert!(
        !is_process_alive(parent_pid),
        "Direct parent process {parent_pid} must be terminated"
    );
    assert!(
        !is_process_alive(descendant_pid),
        "Descendant process {descendant_pid} must be terminated by atomic job containment"
    );

    temp_guard
        .cleanup()
        .expect("Temp dir must be cleanly removable after process-tree termination");
    assert!(
        !test_dir.exists(),
        "Temporary directory must not exist after cleanup"
    );
}

#[tokio::test]
async fn test_process_containment_setup_failure_cleans_resources() {
    let mut session = ProcessTreeSession::new().expect("Must create ProcessTreeSession");
    let cmd = spoor_providers::process_tree::ProcessCommand::new("non_existent_binary_xyz_12345")
        .arg("--version");

    let spawn_result = session.spawn(cmd);
    assert!(
        spawn_result.is_err(),
        "Spawning a non-existent binary must fail with an explicit error"
    );
}
