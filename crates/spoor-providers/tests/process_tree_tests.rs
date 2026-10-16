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
