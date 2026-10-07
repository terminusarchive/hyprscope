use std::process::Command;

#[test]
fn without_hyprland_variables_one_line_on_stderr_and_status_1() {
    let out = Command::new(env!("CARGO_BIN_EXE_hyprscope"))
        .env_remove("XDG_RUNTIME_DIR")
        .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty(), "{:?}", String::from_utf8_lossy(&out.stdout));
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(err.starts_with("hyprscope: "), "{err}");
    assert_eq!(err.trim_end().lines().count(), 1, "{err}");
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("hyprscope-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("hypr/sig")).unwrap();
    dir
}

fn assert_one_error_line(out: &std::process::Output, prefix: &str) {
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty(), "{:?}", String::from_utf8_lossy(&out.stdout));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.starts_with(prefix), "{err}");
    assert_eq!(err.trim_end().lines().count(), 1, "{err}");
}

#[test]
fn without_a_socket_one_cannot_connect_line_and_status_1() {
    let dir = temp_dir("noconn");
    let out = Command::new(env!("CARGO_BIN_EXE_hyprscope"))
        .env("XDG_RUNTIME_DIR", &dir)
        .env("HYPRLAND_INSTANCE_SIGNATURE", "sig")
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert_one_error_line(&out, "hyprscope: cannot connect");
}

/// Answers one request on `.socket.sock` in `dir` with the captured snapshot.
fn serve_snapshot(dir: &std::path::Path) -> std::thread::JoinHandle<()> {
    use std::io::{Read, Write};
    let listener = std::os::unix::net::UnixListener::bind(dir.join("hypr/sig/.socket.sock")).unwrap();
    std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        let mut buf = [0u8; 256];
        let _ = conn.read(&mut buf);
        conn.write_all(include_bytes!("fixtures/snapshot.txt")).unwrap();
    })
}

#[test]
fn without_an_event_socket_one_cannot_connect_line_and_status_1() {
    let dir = temp_dir("noevents");
    let server = serve_snapshot(&dir);
    let out = Command::new(env!("CARGO_BIN_EXE_hyprscope"))
        .env("XDG_RUNTIME_DIR", &dir)
        .env("HYPRLAND_INSTANCE_SIGNATURE", "sig")
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    server.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert_one_error_line(&out, "hyprscope: cannot connect");
    assert!(String::from_utf8_lossy(&out.stderr).contains(".socket2.sock"));
}

#[test]
fn without_a_terminal_one_line_on_stderr_and_status_1() {
    let dir = temp_dir("notty");
    let server = serve_snapshot(&dir);
    // The event socket is there, so start-up reaches the terminal.
    let _events = std::os::unix::net::UnixListener::bind(dir.join("hypr/sig/.socket2.sock")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hyprscope"))
        .env("XDG_RUNTIME_DIR", &dir)
        .env("HYPRLAND_INSTANCE_SIGNATURE", "sig")
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    server.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert_one_error_line(&out, "hyprscope: ");
    assert!(!String::from_utf8_lossy(&out.stderr).contains("panicked"));
}
