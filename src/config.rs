//! The configuration summary: what Hyprland reports about errors, options, binds and
//! animations, reduced to the figures the Config panel shows.

use crate::hypr::Hypr;
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Separator Hyprland puts between the replies of a `[[BATCH]]` request.
const SEPARATOR: &str = "\n\n\n";

/// The figures of the Config panel. Each is `None` when its reply is missing or does
/// not parse.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    /// The configuration errors, in the order Hyprland reports them.
    pub errors: Option<Vec<String>>,
    /// Options set by the configuration, and options in all.
    pub options: Option<(usize, usize)>,
    /// Binds, and the submaps they belong to (binds outside a submap are one submap).
    pub binds: Option<(usize, usize)>,
    /// Animations, how many the configuration overrides, and bezier curves.
    pub animations: Option<(usize, usize, usize)>,
}

fn json<'a, T: Deserialize<'a>>(reply: &'a str) -> Option<T> {
    serde_json::from_str(reply.trim()).ok()
}

/// `j/configerrors`: a list of strings, where one empty string means no errors.
fn errors(reply: &str) -> Option<Vec<String>> {
    let list: Vec<String> = json(reply)?;
    Some(list.into_iter().filter(|e| !e.trim().is_empty()).collect())
}

#[derive(Deserialize)]
struct Bind {
    submap: String,
}

/// `j/binds`: the number of binds and of distinct submaps.
fn binds(reply: &str) -> Option<(usize, usize)> {
    let list: Vec<Bind> = json(reply)?;
    let submaps: HashSet<&str> = list.iter().map(|b| b.submap.as_str()).collect();
    Some((list.len(), submaps.len()))
}

#[derive(Deserialize)]
struct Animation {
    overridden: bool,
}

/// `j/animations`: a list of animations followed by a list of curves.
fn animations(reply: &str) -> Option<(usize, usize, usize)> {
    let (animations, curves): (Vec<Animation>, Vec<serde::de::IgnoredAny>) = json(reply)?;
    Some((animations.len(), animations.iter().filter(|a| a.overridden).count(), curves.len()))
}

#[derive(Deserialize)]
struct Option_ {
    #[serde(default)]
    set: bool,
}

/// The replies of a batch of `j/getoption`, one per option. A reply that is not an
/// object, such as `invalid type (internal error)`, is an option that exists and is
/// not set. `total` is how many replies to expect; any other count is no answer.
fn options(reply: &str, total: usize) -> Option<(usize, usize)> {
    let parts: Vec<&str> = reply.split(SEPARATOR).collect();
    if total == 0 || parts.len() != total {
        return None;
    }
    let set = parts.iter().filter(|p| json::<Option_>(p).is_some_and(|o| o.set)).count();
    Some((set, total))
}

/// Asks Hyprland for the summary. It keeps the option names, which do not change while
/// Hyprland runs and cost the most to fetch.
pub struct Config {
    names: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct Described {
    name: String,
}

impl Config {
    /// Fetches the option names. If that fails, `refresh` tries again.
    pub fn new(hypr: &Hypr) -> Self {
        Self { names: option_names(hypr) }
    }

    /// Two requests: errors, binds and animations in one batch, every option's state in
    /// another. A figure whose reply is missing or malformed is `None`.
    pub fn refresh(&mut self, hypr: &Hypr) -> Summary {
        let mut summary = Summary::default();
        if let Ok(reply) = hypr.request("[[BATCH]]j/configerrors;j/binds;j/animations") {
            let parts: Vec<&str> = reply.split(SEPARATOR).collect();
            if let [errs, binds_reply, anims] = parts[..] {
                summary.errors = errors(errs);
                summary.binds = binds(binds_reply);
                summary.animations = animations(anims);
            }
        }
        if self.names.is_none() {
            self.names = option_names(hypr);
        }
        if let Some(names) = &self.names {
            let command = format!("[[BATCH]]{}", names.iter().map(|n| format!("j/getoption {n}")).collect::<Vec<_>>().join(";"));
            if let Ok(reply) = hypr.request(&command) {
                summary.options = options(&reply, names.len());
                if summary.options.is_none() && reply.split(SEPARATOR).count() != names.len() {
                    // The options changed (a plugin loaded): fetch the names again next time.
                    self.names = None;
                }
            }
        }
        summary
    }
}

/// The names of all options, from `j/descriptions`. Only the names are used: its values
/// are not reliable. `None` when the reply is missing, malformed or lists nothing.
fn option_names(hypr: &Hypr) -> Option<Vec<String>> {
    let list: Vec<Described> = json(&hypr.request("j/descriptions").ok()?)?;
    (!list.is_empty()).then(|| list.into_iter().map(|d| d.name).collect())
}

/// The configuration file Hyprland was started with, from the bytes of its
/// `/proc/<pid>/cmdline` (arguments separated by NUL): the path after `--config` or
/// `-c`, or in `--config=<path>`; else `hyprland.lua` or `hyprland.conf` in `config_dir`,
/// the provider's own first. A leading `home` is written `~`. `None` when the command
/// line is empty or the file cannot be told.
pub fn start_file(cmdline: &[u8], provider: &str, home: Option<&Path>, config_dir: Option<&Path>) -> Option<String> {
    let text = std::str::from_utf8(cmdline).ok()?;
    let mut args = text.split('\0').filter(|a| !a.is_empty()).peekable();
    args.peek()?;
    let mut given = None;
    while let Some(arg) = args.next() {
        if let Some(path) = arg.strip_prefix("--config=") {
            given = Some(path);
        } else if arg == "--config" || arg == "-c" {
            given = args.next();
        }
    }
    let path = match given {
        Some(path) => PathBuf::from(path),
        None => {
            let dir = config_dir?;
            let (first, second) = if provider == "lua" { ("hyprland.lua", "hyprland.conf") } else { ("hyprland.conf", "hyprland.lua") };
            [first, second].into_iter().map(|name| dir.join(name)).find(|p| p.is_file())?
        }
    };
    let shown = match home.and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) => Path::new("~").join(rest),
        None => path,
    };
    Some(shown.to_string_lossy().into_owned())
}

/// `start_file` for the running process, with the home and configuration directories
/// from the environment: `$XDG_CONFIG_HOME/hypr`, else `~/.config/hypr`.
pub fn running_start_file(pid: u32, provider: &str) -> Option<String> {
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let config_dir = match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        Some(xdg) => Some(PathBuf::from(xdg).join("hypr")),
        None => home.as_ref().map(|h| h.join(".config/hypr")),
    };
    start_file(&cmdline, provider, home.as_deref(), config_dir.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_string_in_the_list_means_no_errors() {
        assert_eq!(errors(r#"[""]"#), Some(vec![]));
        assert_eq!(errors("[]\n"), Some(vec![]));
    }

    #[test]
    fn errors_keep_their_order() {
        let reply = r#"["hyprland.lua:212: unknown dispatcher", "hyprland.lua:9: bad value"]"#;
        assert_eq!(
            errors(reply),
            Some(vec!["hyprland.lua:212: unknown dispatcher".into(), "hyprland.lua:9: bad value".into()])
        );
    }

    #[test]
    fn only_options_with_set_true_count_as_set() {
        let reply = [
            r#"{"option":"general:border_size","int":2,"set":true}"#,
            r#"{"option":"general:gaps_in","int":5,"set":false}"#,
            r#"{"option":"decoration:rounding","int":0}"#,
            r#"{"option":"misc:x","str":"a","set":true}"#,
            "\n{\"option\":\"misc:y\",\"bool\":true,\"set\":true}\n",
        ]
        .join(SEPARATOR);
        assert_eq!(options(&reply, 5), Some((3, 5)));
    }

    #[test]
    fn a_reply_that_is_not_an_object_exists_and_is_not_set() {
        let reply = [
            r#"{"option":"a","int":1,"set":true}"#,
            "invalid type (internal error)",
            "",
            "[1,2]",
        ]
        .join(SEPARATOR);
        assert_eq!(options(&reply, 4), Some((1, 4)));
    }

    #[test]
    fn options_with_the_wrong_number_of_replies_are_no_answer() {
        let reply = [r#"{"set":true}"#, r#"{"set":true}"#].join(SEPARATOR);
        assert_eq!(options(&reply, 3), None);
        assert_eq!(options(&reply, 1), None);
        assert_eq!(options("", 0), None);
        assert_eq!(options("", 2), None);
    }

    #[test]
    fn binds_are_counted_with_their_distinct_submaps() {
        let reply = r#"[{"submap":"","key":"a"},{"submap":"","key":"b"},{"submap":"resize","key":"c"}]"#;
        assert_eq!(binds(reply), Some((3, 2)));
        assert_eq!(binds(r#"[{"submap":""}]"#), Some((1, 1)));
        assert_eq!(binds("[]"), Some((0, 0)));
    }

    #[test]
    fn animations_overridden_and_curves_come_from_the_two_lists() {
        let reply = r#"[[{"name":"a","overridden":true},{"name":"b","overridden":false},{"name":"c","overridden":true}],
                        [{"name":"easeOut"},{"name":"linear"}]]"#;
        assert_eq!(animations(reply), Some((3, 2, 2)));
        assert_eq!(animations("[[],[]]"), Some((0, 0, 0)));
    }

    #[test]
    fn a_malformed_reply_gives_none_for_its_own_figure() {
        assert_eq!(errors(r#"{"a":1}"#), None);
        assert_eq!(binds(r#"[{"key":"a"}]"#), None);
        assert_eq!(binds("not json"), None);
        assert_eq!(animations(r#"[[{"overridden":true}]]"#), None);
        assert_eq!(animations("[1,2]"), None);
    }

    #[test]
    fn empty_and_truncated_replies_give_none_and_do_not_panic() {
        let binds_reply = r#"[{"submap":"","key":"a"},{"submap":"x"}]"#;
        let anim_reply = r#"[[{"overridden":true}],[{"name":"c"}]]"#;
        for cut in 0..binds_reply.len() {
            assert_eq!(binds(&binds_reply[..cut]), None, "binds cut at {cut}");
        }
        for cut in 0..anim_reply.len() {
            assert_eq!(animations(&anim_reply[..cut]), None, "animations cut at {cut}");
        }
        assert_eq!(errors(""), None);
        assert_eq!(errors(r#"["a"#), None);
    }

    use crate::hypr::Hypr;
    use crate::testdir::TempDir;
    use std::io::{Read, Write};
    use std::os::unix::net::UnixListener;
    use std::sync::{Arc, Mutex};

    /// Answers `connections` requests on a socket in a fresh temp dir. `answer` gets the
    /// command and returns the reply, or `None` to drop the connection without one.
    /// The commands received are kept in the returned list, in order.
    fn fake(
        name: &str,
        connections: usize,
        answer: impl Fn(&str) -> Option<String> + Send + 'static,
    ) -> (TempDir, Hypr, Arc<Mutex<Vec<String>>>) {
        let dir = TempDir::new(name);
        let path = dir.join(".socket.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        std::thread::spawn(move || {
            for _ in 0..connections {
                let (mut conn, _) = listener.accept().unwrap();
                let mut buf = [0u8; 65536];
                let n = conn.read(&mut buf).unwrap();
                let command = String::from_utf8_lossy(&buf[..n]).into_owned();
                if let Some(reply) = answer(&command) {
                    conn.write_all(reply.as_bytes()).unwrap();
                }
                log.lock().unwrap().push(command);
            }
        });
        (dir, Hypr::at(path), seen)
    }

    const DESCRIPTIONS: &str = r#"[{"name":"general:border_size","description":"x","default":1,"current":1},
        {"name":"general:gaps_in","description":"y"},{"name":"group:groupbar:font_weight_active"}]"#;

    fn replies(parts: &[&str]) -> String {
        parts.join(SEPARATOR)
    }

    /// Answers like Hyprland for one configuration: no errors, 2 binds in 2 submaps, one
    /// overridden animation and one curve, and options of which the first is set.
    fn hyprland(command: &str) -> Option<String> {
        Some(match command {
            "j/descriptions" => DESCRIPTIONS.into(),
            "[[BATCH]]j/configerrors;j/binds;j/animations" => replies(&[
                r#"[""]"#,
                r#"[{"submap":""},{"submap":"resize"}]"#,
                r#"[[{"overridden":true},{"overridden":false}],[{"name":"c"}]]"#,
            ]),
            c if c.starts_with("[[BATCH]]j/getoption ") => replies(&[
                r#"{"option":"general:border_size","int":2,"set":true}"#,
                r#"{"option":"general:gaps_in","int":5,"set":false}"#,
                "invalid type (internal error)",
            ]),
            other => panic!("unexpected command {other}"),
        })
    }

    #[test]
    fn new_asks_for_the_option_names_once_and_refresh_sends_the_two_batches() {
        let (_dir, hypr, seen) = fake("cfg", 5, hyprland);
        let mut config = Config::new(&hypr);
        let first = config.refresh(&hypr);
        let second = config.refresh(&hypr);
        assert_eq!(
            first,
            Summary {
                errors: Some(vec![]),
                options: Some((1, 3)),
                binds: Some((2, 2)),
                animations: Some((2, 1, 1)),
            }
        );
        assert_eq!(second, first);
        let seen = seen.lock().unwrap();
        let batch = "[[BATCH]]j/getoption general:border_size;j/getoption general:gaps_in;j/getoption group:groupbar:font_weight_active";
        assert_eq!(
            *seen,
            [
                "j/descriptions",
                "[[BATCH]]j/configerrors;j/binds;j/animations",
                batch,
                "[[BATCH]]j/configerrors;j/binds;j/animations",
                batch,
            ]
        );
    }

    #[test]
    fn a_dropped_second_batch_leaves_only_the_options_missing() {
        let (_dir, hypr, _) = fake("drop", 3, |c| if c.starts_with("[[BATCH]]j/getoption") { None } else { hyprland(c) });
        let summary = Config::new(&hypr).refresh(&hypr);
        assert_eq!(summary.options, None);
        assert_eq!(summary.errors, Some(vec![]));
        assert_eq!(summary.binds, Some((2, 2)));
        assert_eq!(summary.animations, Some((2, 1, 1)));
    }

    #[test]
    fn a_dropped_first_batch_leaves_only_the_other_three_missing() {
        let (_dir, hypr, _) = fake("drop1", 3, |c| if c.starts_with("[[BATCH]]j/configerrors") { None } else { hyprland(c) });
        let summary = Config::new(&hypr).refresh(&hypr);
        assert_eq!(summary, Summary { options: Some((1, 3)), ..Summary::default() });
    }

    #[test]
    fn a_missing_socket_gives_no_figures() {
        let hypr = Hypr::at(std::env::temp_dir().join(format!("hyprscope-cfgnone-{}/.socket.sock", std::process::id())));
        let mut config = Config::new(&hypr);
        assert_eq!(config.refresh(&hypr), Summary::default());
    }

    #[test]
    fn the_option_names_are_asked_for_again_when_the_first_try_failed() {
        let calls = Arc::new(Mutex::new(0));
        let n = Arc::clone(&calls);
        let (_dir, hypr, seen) = fake("retry", 4, move |c| {
            if c == "j/descriptions" {
                let mut n = n.lock().unwrap();
                *n += 1;
                return if *n == 1 { None } else { hyprland(c) };
            }
            hyprland(c)
        });
        let mut config = Config::new(&hypr);
        let summary = config.refresh(&hypr);
        assert_eq!(summary.options, Some((1, 3)));
        assert_eq!(seen.lock().unwrap().iter().filter(|c| *c == "j/descriptions").count(), 2);
    }

    #[test]
    fn option_names_are_fetched_again_after_the_option_count_changes() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let grown = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&grown);
        let (_dir, hypr, seen) = fake("grow", 8, move |c| {
            if !flag.load(Ordering::SeqCst) {
                return hyprland(c);
            }
            match c {
                "j/descriptions" => Some(r#"[{"name":"a"},{"name":"b"},{"name":"c"},{"name":"d"}]"#.into()),
                c if c.starts_with("[[BATCH]]j/getoption ") => Some(replies(&[r#"{"set":true}"#; 4])),
                _ => hyprland(c),
            }
        });
        let mut config = Config::new(&hypr);
        assert_eq!(config.refresh(&hypr).options, Some((1, 3)));
        grown.store(true, Ordering::SeqCst);
        let stale = config.refresh(&hypr);
        assert_eq!(stale, Summary { options: None, ..config_figures() });
        assert_eq!(config.refresh(&hypr).options, Some((4, 4)));
        let batch3 = "[[BATCH]]j/getoption general:border_size;j/getoption general:gaps_in;j/getoption group:groupbar:font_weight_active";
        let batch4 = "[[BATCH]]j/getoption a;j/getoption b;j/getoption c;j/getoption d";
        let other = "[[BATCH]]j/configerrors;j/binds;j/animations";
        assert_eq!(*seen.lock().unwrap(), ["j/descriptions", other, batch3, other, batch3, other, "j/descriptions", batch4]);
    }

    fn config_figures() -> Summary {
        Summary { errors: Some(vec![]), options: None, binds: Some((2, 2)), animations: Some((2, 1, 1)) }
    }

    // --- start_file ---

    use std::path::Path;

    fn args(list: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for a in list {
            out.extend_from_slice(a.as_bytes());
            out.push(0);
        }
        out
    }

    const HOME: &str = "/home/user";

    fn file(list: &[&str]) -> Option<String> {
        start_file(&args(list), "lua", Some(Path::new(HOME)), None)
    }

    #[test]
    fn the_config_argument_in_each_of_its_three_forms_gives_the_path() {
        assert_eq!(file(&["Hyprland", "--config", "/etc/h.conf"]), Some("/etc/h.conf".into()));
        assert_eq!(file(&["Hyprland", "-c", "/etc/h.conf"]), Some("/etc/h.conf".into()));
        assert_eq!(file(&["Hyprland", "--config=/etc/h.conf"]), Some("/etc/h.conf".into()));
        assert_eq!(file(&["Hyprland", "--verbose", "-c", "/etc/h.conf", "--other"]), Some("/etc/h.conf".into()));
    }

    #[test]
    fn a_path_under_the_home_directory_starts_with_a_tilde() {
        assert_eq!(
            file(&["Hyprland", "--config", "/home/user/.config/hypr/hyprland-sessions.lua"]),
            Some("~/.config/hypr/hyprland-sessions.lua".into())
        );
        // Only a whole leading directory counts.
        assert_eq!(file(&["Hyprland", "-c", "/home/user2/h.conf"]), Some("/home/user2/h.conf".into()));
        assert_eq!(start_file(&args(&["H", "-c", "/home/user/h"]), "lua", None, None), Some("/home/user/h".into()));
    }

    #[test]
    fn without_the_argument_the_file_in_the_config_directory_is_given() {
        let dir = TempDir::new("start-file");
        let home = dir.join("home");
        let cfg = home.join(".config/hypr");
        std::fs::create_dir_all(&cfg).unwrap();
        let find = |provider: &str| start_file(&args(&["Hyprland"]), provider, Some(&home), Some(&cfg));
        assert_eq!(find("lua"), None, "neither exists");
        std::fs::write(cfg.join("hyprland.conf"), "").unwrap();
        assert_eq!(find("lua"), Some("~/.config/hypr/hyprland.conf".into()));
        std::fs::write(cfg.join("hyprland.lua"), "").unwrap();
        assert_eq!(find("lua"), Some("~/.config/hypr/hyprland.lua".into()));
        assert_eq!(find("hyprlang"), Some("~/.config/hypr/hyprland.conf".into()));
        std::fs::remove_file(cfg.join("hyprland.conf")).unwrap();
        assert_eq!(find("hyprlang"), Some("~/.config/hypr/hyprland.lua".into()), "the only one that exists");
        // An argument wins over the directory.
        assert_eq!(start_file(&args(&["Hyprland", "-c", "/x.conf"]), "lua", Some(&home), Some(&cfg)), Some("/x.conf".into()));
        assert_eq!(start_file(&args(&["Hyprland"]), "lua", Some(&home), None), None);
    }

    #[test]
    fn an_empty_or_unreadable_command_line_gives_nothing() {
        let dir = TempDir::new("start-file-empty");
        std::fs::write(dir.join("hyprland.lua"), "").unwrap();
        assert_eq!(start_file(b"", "lua", None, Some(&dir)), None);
        assert_eq!(start_file(&args(&["Hyprland", "-c"]), "lua", None, None), None, "an argument with no path");
        assert_eq!(start_file(&[0xff, 0xfe, 0], "lua", None, None), None);
    }
}
