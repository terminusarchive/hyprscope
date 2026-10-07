use std::process::{Command, Output, Stdio};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hyprscope"))
        .args(args)
        .env_remove("XDG_RUNTIME_DIR")
        .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

#[test]
fn version_prints_one_line_and_exits_0() {
    for flag in ["--version", "-V"] {
        let out = run(&[flag]);
        assert_eq!(out.status.code(), Some(0));
        assert_eq!(String::from_utf8_lossy(&out.stdout), format!("hyprscope {}\n", env!("CARGO_PKG_VERSION")));
        assert!(out.stderr.is_empty(), "{:?}", String::from_utf8_lossy(&out.stderr));
    }
}

#[test]
fn help_prints_the_usage_line_and_exits_0() {
    for flag in ["--help", "-h"] {
        let out = run(&[flag]);
        assert_eq!(out.status.code(), Some(0));
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("Usage: hyprscope [OPTION]\n"), "{text}");
        assert!(out.stderr.is_empty(), "{:?}", String::from_utf8_lossy(&out.stderr));
    }
}

#[test]
fn an_unknown_argument_prints_two_lines_on_stderr_and_exits_2() {
    let out = run(&["--bogus"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty(), "{:?}", String::from_utf8_lossy(&out.stdout));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "hyprscope: unknown argument '--bogus'\nTry 'hyprscope --help'.\n"
    );
}

#[test]
fn a_second_argument_after_a_valid_option_is_rejected_by_name() {
    let out = run(&["--version", "extra"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("hyprscope: unknown argument 'extra'\n"));
}
