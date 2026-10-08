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

/// The lists of the Config screen: each part is `None` when its reply is missing or
/// does not parse.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Details {
    /// Every option, in the order Hyprland lists them.
    pub options: Option<Vec<OptionInfo>>,
    pub binds: Option<Vec<BindInfo>>,
    pub animations: Option<Animations>,
    pub devices: Option<Devices>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OptionInfo {
    pub name: String,
    pub description: String,
    pub default: String,
    pub value: Option<String>,
    pub set: bool,
}

impl OptionInfo {
    /// Whether the value prints the same as the default: the dimming rule. Both
    /// are in one text form (see `text`), so `2.0` equals `2` and `false` equals
    /// `false`. An option with no value is not at its default.
    pub fn is_default(&self) -> bool {
        self.value.as_deref() == Some(self.default.as_str())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BindInfo {
    pub submap: String,
    pub modifiers: Vec<&'static str>,
    pub key: String,
    pub flags: Vec<&'static str>,
    pub description: Option<String>,
    pub dispatcher: Option<String>,
    pub arg: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnimationInfo {
    pub name: String,
    pub enabled: bool,
    pub speed: f64,
    pub curve: String,
    pub style: String,
    pub overridden: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Curve {
    pub name: String,
    pub points: [(f64, f64); 2],
}

#[derive(Debug, Clone, PartialEq)]
pub struct Animations {
    pub list: Vec<AnimationInfo>,
    pub curves: Vec<Curve>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Keyboard {
    pub name: String,
    pub layout: String,
    pub variant: String,
    pub keymap: String,
    pub main: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mouse {
    pub name: String,
    pub speed: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Devices {
    pub keyboards: Vec<Keyboard>,
    pub mice: Vec<Mouse>,
    pub touch: Vec<String>,
    pub tablets: Vec<String>,
    pub switches: Vec<String>,
}

/// One text form for a value in `descriptions` or in a `getoption` reply, so that a
/// value and its default compare equal when they are the same: a number prints without
/// a needless fraction (`2.0` is `2`), a boolean as `true` or `false`, a string without
/// quotes (Hyprland's `[[EMPTY]]` is the empty string), a list such as a `vec2` as its
/// elements separated by a space, and anything else as compact JSON.
fn text(value: &serde_json::Value) -> String {
    use serde_json::Value;
    match value {
        Value::String(s) if s == "[[EMPTY]]" => String::new(),
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => match (n.as_i64(), n.as_u64(), n.as_f64()) {
            (Some(i), _, _) => i.to_string(),
            (_, Some(u), _) => u.to_string(),
            (_, _, Some(f)) => f.to_string(),
            _ => n.to_string(),
        },
        Value::Array(list) => list.iter().map(text).collect::<Vec<_>>().join(" "),
        Value::Object(_) => value.to_string(),
    }
}

#[derive(Deserialize)]
struct Description {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    default: serde_json::Value,
}

/// The value field of a `getoption` reply, whatever its kind (`int`, `float`, `str`,
/// `vec2`, `gradient`, `css`, `bool`, `custom`, or one not seen yet), and whether the
/// option is set. A reply that is not an object, or has no value field, has no value.
fn reported(reply: &str) -> (Option<serde_json::Value>, bool) {
    let Some(serde_json::Value::Object(fields)) = json(reply) else { return (None, false) };
    let set = fields.get("set").and_then(|s| s.as_bool()).unwrap_or(false);
    let value = fields.into_iter().find(|(k, _)| k != "option" && k != "set").map(|(_, v)| v);
    (value, set)
}

/// The text of an option's value. Hyprland describes some colours (`misc:background_color`,
/// `group:groupbar:text_color`) by a default of 8 hex digits, such as `ff111111`, but
/// reports their value as an integer; such an integer, when it fits 32 bits, is printed
/// as 8 hex digits too, so that the two compare. Any other value is as `text` gives it.
fn value_text(value: &serde_json::Value, default: &serde_json::Value) -> String {
    use serde_json::Value;
    if let (Value::Number(n), Value::String(d)) = (value, default)
        && d.len() == 8
        && d.bytes().all(|b| b.is_ascii_hexdigit())
        && let Some(colour) = n.as_u64().and_then(|u| u32::try_from(u).ok())
    {
        return format!("{colour:08x}");
    }
    text(value)
}

/// The options from the reply of `j/descriptions` and the replies of the batch of
/// `j/getoption` for them, in order. The `current` of `descriptions` is wrong on some
/// machines and is never read. `None` when `descriptions` does not parse or lists
/// nothing, or the batch has not one reply for each option.
fn option_list(descriptions: &str, getoptions: &str) -> Option<Vec<OptionInfo>> {
    let described: Vec<Description> = json(descriptions)?;
    let replies: Vec<&str> = getoptions.split(SEPARATOR).collect();
    if described.is_empty() || replies.len() != described.len() {
        return None;
    }
    Some(
        described
            .into_iter()
            .zip(replies)
            .map(|(d, reply)| {
                let (value, set) = reported(reply);
                let value = value.map(|v| value_text(&v, &d.default));
                OptionInfo { name: d.name, description: d.description, default: text(&d.default), value, set }
            })
            .collect(),
    )
}

/// The names of the modifiers in a bind's mask, SUPER, CTRL, ALT and SHIFT first.
fn modifiers(mask: u32) -> Vec<&'static str> {
    const NAMES: [(u32, &str); 8] =
        [(64, "SUPER"), (4, "CTRL"), (8, "ALT"), (1, "SHIFT"), (2, "CAPS"), (16, "MOD2"), (32, "MOD3"), (128, "MOD5")];
    NAMES.iter().filter(|(bit, _)| mask & bit != 0).map(|(_, name)| *name).collect()
}

#[derive(Deserialize)]
struct RawBind {
    submap: String,
    #[serde(default)]
    key: String,
    #[serde(default)]
    keycode: u32,
    #[serde(default)]
    modmask: u32,
    #[serde(default)]
    has_description: bool,
    #[serde(default)]
    description: String,
    dispatcher: String,
    #[serde(default)]
    arg: serde_json::Value,
    #[serde(default)]
    locked: bool,
    #[serde(default)]
    release: bool,
    #[serde(default)]
    repeat: bool,
    #[serde(default, rename = "longPress")]
    long_press: bool,
    #[serde(default)]
    mouse: bool,
    #[serde(default)]
    non_consuming: bool,
    #[serde(default)]
    auto_consuming: bool,
    #[serde(default)]
    catch_all: bool,
}

/// The dispatcher Hyprland reports for a bind made with a Lua function: its
/// `HyprCtl.cpp` prints the keybind's `handler`, and `LuaBindingsToplevel.cpp` sets the
/// handler of a function bind to this, with the function's registry number as `arg`
/// (Hyprland v0.56.2).
const LUA_DISPATCHER: &str = "__lua";

/// `j/binds`, in Hyprland's order. A bind on a key code is keyed `code:NN`.
fn bind_list(reply: &str) -> Option<Vec<BindInfo>> {
    let raw: Vec<RawBind> = json(reply)?;
    Some(
        raw.into_iter()
            .map(|b| {
                let flags = [
                    (b.locked, "locked"),
                    (b.release, "release"),
                    (b.repeat, "repeat"),
                    (b.long_press, "long press"),
                    (b.mouse, "mouse"),
                    (b.non_consuming, "non-consuming"),
                    (b.auto_consuming, "auto-consuming"),
                    (b.catch_all, "catch-all"),
                ];
                BindInfo {
                    submap: b.submap,
                    modifiers: modifiers(b.modmask),
                    key: if b.keycode != 0 { format!("code:{}", b.keycode) } else { b.key },
                    flags: flags.iter().filter(|(on, _)| *on).map(|(_, name)| *name).collect(),
                    description: (b.has_description && !b.description.is_empty()).then_some(b.description),
                    dispatcher: (b.dispatcher != LUA_DISPATCHER).then_some(b.dispatcher),
                    arg: text(&b.arg),
                }
            })
            .collect(),
    )
}

#[derive(Deserialize)]
struct RawAnimation {
    name: String,
    overridden: bool,
    bezier: String,
    enabled: bool,
    speed: f64,
    style: String,
}

#[derive(Deserialize)]
struct RawCurve {
    name: String,
    #[serde(rename = "X0")]
    x0: f64,
    #[serde(rename = "Y0")]
    y0: f64,
    #[serde(rename = "X1")]
    x1: f64,
    #[serde(rename = "Y1")]
    y1: f64,
}

/// `j/animations`: the animations followed by the curves. A curve an animation names
/// (`default`, a spring such as `spring:easy`, or none at all) need not be in the list.
fn animation_lists(reply: &str) -> Option<Animations> {
    let (list, curves): (Vec<RawAnimation>, Vec<RawCurve>) = json(reply)?;
    Some(Animations {
        list: list
            .into_iter()
            .map(|a| AnimationInfo {
                name: a.name,
                enabled: a.enabled,
                speed: a.speed,
                curve: a.bezier,
                style: a.style,
                overridden: a.overridden,
            })
            .collect(),
        curves: curves.into_iter().map(|c| Curve { name: c.name, points: [(c.x0, c.y0), (c.x1, c.y1)] }).collect(),
    })
}

#[derive(Deserialize)]
struct Named {
    name: String,
}

#[derive(Deserialize)]
struct RawKeyboard {
    name: String,
    layout: String,
    variant: String,
    active_keymap: String,
    main: bool,
}

#[derive(Deserialize)]
struct RawMouse {
    name: String,
    #[serde(rename = "defaultSpeed")]
    speed: f64,
}

#[derive(Deserialize)]
struct RawDevices {
    keyboards: Vec<RawKeyboard>,
    mice: Vec<RawMouse>,
    touch: Vec<Named>,
    tablets: Vec<Named>,
    switches: Vec<Named>,
}

/// `j/devices`: a keyboard keeps its layout, variant, keymap and whether it is main, a
/// mouse its speed, and the other kinds only their names.
fn device_lists(reply: &str) -> Option<Devices> {
    let raw: RawDevices = json(reply)?;
    let names = |list: Vec<Named>| list.into_iter().map(|d| d.name).collect();
    Some(Devices {
        keyboards: raw
            .keyboards
            .into_iter()
            .map(|k| Keyboard { name: k.name, layout: k.layout, variant: k.variant, keymap: k.active_keymap, main: k.main })
            .collect(),
        mice: raw.mice.into_iter().map(|m| Mouse { name: m.name, speed: m.speed }).collect(),
        touch: names(raw.touch),
        tablets: names(raw.tablets),
        switches: names(raw.switches),
    })
}

impl Config {
    /// The lists of the Config screen, in three requests: `j/descriptions`, the batch of
    /// `j/getoption` for the options it names, and one batch of binds, animations and
    /// devices. Only for the time the screen shows. A part whose reply is missing or
    /// malformed is `None`; the others are not affected.
        pub fn details(&self, hypr: &Hypr) -> Details {
        let mut details = Details::default();
        if let Ok(descriptions) = hypr.request("j/descriptions")
            && let Some(names) = json::<Vec<Named>>(&descriptions)
            && !names.is_empty()
        {
            let command = format!("[[BATCH]]{}", names.iter().map(|n| format!("j/getoption {}", n.name)).collect::<Vec<_>>().join(";"));
            if let Ok(reply) = hypr.request(&command) {
                details.options = option_list(&descriptions, &reply);
            }
        }
        if let Ok(reply) = hypr.request("[[BATCH]]j/binds;j/animations;j/devices") {
            let parts: Vec<&str> = reply.split(SEPARATOR).collect();
            if let [binds_reply, animations_reply, devices_reply] = parts[..] {
                details.binds = bind_list(binds_reply);
                details.animations = animation_lists(animations_reply);
                details.devices = device_lists(devices_reply);
            }
        }
        details
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

/// The captured replies, read as `details` reads them.
#[cfg(test)]
pub fn fixture_details() -> Details {
    let binds_reply = include_str!("../tests/fixtures/config/binds.json");
    Details {
        options: option_list(
            include_str!("../tests/fixtures/config/descriptions.json"),
            include_str!("../tests/fixtures/config/getoptions.txt"),
        ),
        binds: bind_list(binds_reply),
        animations: animation_lists(include_str!("../tests/fixtures/config/animations.json")),
        devices: device_lists(include_str!("../tests/fixtures/config/devices.json")),
    }
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

    // --- details ---

    const FX_DESCRIPTIONS: &str = include_str!("../tests/fixtures/config/descriptions.json");
    const FX_GETOPTIONS: &str = include_str!("../tests/fixtures/config/getoptions.txt");
    const FX_BINDS: &str = include_str!("../tests/fixtures/config/binds.json");
    const FX_ANIMATIONS: &str = include_str!("../tests/fixtures/config/animations.json");
    const FX_DEVICES: &str = include_str!("../tests/fixtures/config/devices.json");

    fn fx_options() -> Vec<OptionInfo> {
        option_list(FX_DESCRIPTIONS, FX_GETOPTIONS).unwrap()
    }

    fn fx_option(name: &str) -> OptionInfo {
        fx_options().into_iter().find(|o| o.name == name).unwrap()
    }

    #[test]
    fn an_option_has_its_name_description_default_value_and_whether_it_is_set() {
        let options = fx_options();
        assert_eq!(options.len(), 15);
        assert_eq!(options[0].name, "general:border_size");
        let border = fx_option("general:border_size");
        assert_eq!(border.description, "size of the border around windows");
        assert_eq!(border.default, "1");
        assert_eq!(border.value.as_deref(), Some("2"));
        assert!(border.set);
        assert!(!border.is_default());
        let gaps = fx_option("general:gaps_workspaces");
        assert_eq!((gaps.value.as_deref(), gaps.set), (Some("0"), false));
    }

    #[test]
    fn the_value_is_the_getoption_reply_and_never_the_current_of_descriptions() {
        // The fixture says current 1 for border_size, and getoption says 2.
        let descriptions = r#"[{"name":"a:b","description":"d","default":1,"current":1}]"#;
        let options = option_list(descriptions, r#"{"option":"a:b","int":2,"set":true}"#).unwrap();
        assert_eq!(options[0].value.as_deref(), Some("2"));
        let options = option_list(descriptions, "invalid type (internal error)").unwrap();
        assert_eq!(options[0].value, None);
    }

    #[test]
    fn every_kind_of_value_in_the_captured_replies_becomes_text() {
        let text = |name: &str| fx_option(name).value;
        assert_eq!(text("general:border_size").as_deref(), Some("2"));
        assert_eq!(text("general:gaps_in").as_deref(), Some("5 5 5 5"));
        assert_eq!(text("general:col.active_border").as_deref(), Some("ffffb596 0deg"));
        assert_eq!(text("general:layout").as_deref(), Some("dwindle"));
        assert_eq!(text("general:resize_on_border").as_deref(), Some("false"));
        assert_eq!(text("decoration:rounding_power").as_deref(), Some("2"));
        assert_eq!(text("decoration:blur:vibrancy").as_deref(), Some("0.1696"));
        assert_eq!(text("decoration:shadow:offset").as_deref(), Some("0 0"));
        assert_eq!(text("input:kb_model").as_deref(), Some(""));
        assert_eq!(text("group:groupbar:text_color_inactive").as_deref(), Some("4294041560"));
    }

    #[test]
    fn a_colour_reported_as_an_integer_against_a_hex_default_reads_as_hex() {
        // Hyprland reports these as `int` but describes their default as 8-digit hex text.
        let colour = fx_option("group:groupbar:text_color");
        assert_eq!((colour.value.as_deref(), colour.set, colour.is_default()), (Some("ff442a1f"), true, false));
        for name in ["misc:background_color", "misc:col.splash"] {
            let option = fx_option(name);
            assert!(option.is_default() && !option.set, "{name}: {option:?}");
        }
        // The `-1` default is not a colour in hex: the number stays, and differs from it.
        let inactive = fx_option("group:groupbar:text_color_inactive");
        assert_eq!((inactive.default.as_str(), inactive.value.as_deref(), inactive.is_default()), ("-1", Some("4294041560"), false));
    }

    #[test]
    fn only_a_32_bit_non_negative_integer_against_a_hex_text_default_is_turned_into_hex() {
        let value = |default: &str, reply: &str| {
            let descriptions = format!(r#"[{{"name":"a","default":{default}}}]"#);
            option_list(&descriptions, reply).unwrap().remove(0).value
        };
        assert_eq!(value(r#""ff111111""#, r#"{"int":0,"set":true}"#).as_deref(), Some("00000000"));
        assert_eq!(value(r#""ff111111""#, r#"{"int":4294967295,"set":true}"#).as_deref(), Some("ffffffff"));
        assert_eq!(value(r#""ff111111""#, r#"{"int":4294967296,"set":true}"#).as_deref(), Some("4294967296"));
        assert_eq!(value(r#""ff111111""#, r#"{"int":-5,"set":true}"#).as_deref(), Some("-5"));
        assert_eq!(value(r#""ff111111""#, r#"{"int":5,"set":true}"#).as_deref(), Some("00000005"));
        // A default that is a number, or text that is not 8 hex digits, leaves the value alone.
        assert_eq!(value("12345678", r#"{"int":12345678,"set":true}"#).as_deref(), Some("12345678"));
        assert_eq!(value(r#""-1""#, r#"{"int":5,"set":true}"#).as_deref(), Some("5"));
        assert_eq!(value(r#""ffffffff 0deg""#, r#"{"int":5,"set":true}"#).as_deref(), Some("5"));
        assert_eq!(value(r#""fffffff""#, r#"{"int":5,"set":true}"#).as_deref(), Some("5"));
        // A reply that is not an integer is left alone.
        assert_eq!(value(r#""ff111111""#, r#"{"str":"abc","set":true}"#).as_deref(), Some("abc"));
    }

    #[test]
    fn a_kind_of_value_not_seen_yet_still_shows() {
        let descriptions = r#"[{"name":"a"},{"name":"b"},{"name":"c"}]"#;
        let replies = replies(&[
            r#"{"option":"a","custom":"some text","set":true}"#,
            r#"{"option":"b","strange":{"x":[1,2.5]},"set":false}"#,
            r#"{"option":"c","set":true}"#,
        ]);
        let options = option_list(descriptions, &replies).unwrap();
        assert_eq!(options[0].value.as_deref(), Some("some text"));
        assert_eq!(options[1].value.as_deref(), Some(r#"{"x":[1,2.5]}"#));
        assert_eq!(options[2].value, None, "no value field at all");
    }

    #[test]
    fn a_reply_that_is_not_an_object_gives_a_value_of_none_and_not_set() {
        let option = fx_option("group:groupbar:font_weight_active");
        assert_eq!((option.value.as_deref(), option.set), (None, false));
        assert_eq!(option.default, "400");
        assert!(!option.is_default());
    }

    #[test]
    fn a_value_equals_its_default_when_both_print_the_same() {
        let eq = |default: &str, reply: &str| {
            let descriptions = format!(r#"[{{"name":"a","default":{default}}}]"#);
            option_list(&descriptions, reply).unwrap()[0].is_default()
        };
        assert!(eq("2", r#"{"float":2.0,"set":true}"#), "a float with no fraction");
        assert!(eq("false", r#"{"bool":false,"set":true}"#));
        assert!(eq(r#""5 5 5 5""#, r#"{"css":"5 5 5 5","set":true}"#));
        assert!(eq(r#""[[EMPTY]]""#, r#"{"str":"","set":true}"#));
        assert!(eq("[0, 0.5]", r#"{"vec2":[0.0,0.5],"set":true}"#));
        assert!(eq("0.1696", r#"{"float":0.169600,"set":true}"#));
        assert!(!eq("1", r#"{"int":2,"set":true}"#));
        assert!(!eq(r#""ffffffff 0deg""#, r#"{"gradient":"ffffb596 0deg","set":true}"#));
        assert!(!eq("true", r#"{"bool":false,"set":true}"#));
        assert!(!eq("1", "invalid type (internal error)"), "no value is not the default");
    }

    #[test]
    fn options_with_the_wrong_number_of_replies_or_descriptions_that_do_not_parse_are_none() {
        assert_eq!(option_list(FX_DESCRIPTIONS, r#"{"set":true}"#), None);
        assert_eq!(option_list("not json", FX_GETOPTIONS), None);
        assert_eq!(option_list("[]", ""), None);
    }

    fn fx_binds() -> Vec<BindInfo> {
        bind_list(FX_BINDS).unwrap()
    }

    #[test]
    fn a_binds_modifiers_are_decoded_in_a_fixed_order() {
        assert_eq!(modifiers(64), ["SUPER"]);
        assert_eq!(modifiers(64 | 1 | 4 | 8), ["SUPER", "CTRL", "ALT", "SHIFT"]);
        assert_eq!(modifiers(0), Vec::<&str>::new());
        assert_eq!(modifiers(255), ["SUPER", "CTRL", "ALT", "SHIFT", "CAPS", "MOD2", "MOD3", "MOD5"]);
        let binds = fx_binds();
        assert_eq!(binds[0].modifiers, ["SUPER"]);
        assert_eq!(binds[1].modifiers, ["SUPER", "ALT"]);
        assert_eq!(binds[2].modifiers, ["SUPER", "SHIFT"]);
        assert_eq!(binds[3].modifiers, Vec::<&str>::new());
    }

    #[test]
    fn a_key_code_and_a_mouse_button_read_as_the_spec_writes_them() {
        let binds = fx_binds();
        assert_eq!(binds[0].key, "RETURN");
        let mouse: Vec<&str> = binds.iter().map(|b| b.key.as_str()).filter(|k| k.starts_with("mouse")).collect();
        assert_eq!(mouse, ["mouse_up", "mouse_down", "mouse:272", "mouse:273"]);
        let reply = r#"[{"submap":"","key":"36","keycode":36,"modmask":64,"dispatcher":"x","arg":""}]"#;
        assert_eq!(bind_list(reply).unwrap()[0].key, "code:36");
        let reply = r#"[{"submap":"","key":"","keycode":0,"modmask":0,"dispatcher":"x","arg":""}]"#;
        assert_eq!(bind_list(reply).unwrap()[0].key, "");
    }

    #[test]
    fn each_flag_hyprland_reports_is_carried_by_name() {
        let binds = fx_binds();
        assert_eq!(binds[0].flags, Vec::<&str>::new());
        assert_eq!(binds[5].flags, ["locked", "repeat"]);
        let reply = r#"[{"submap":"s","key":"a","modmask":0,"dispatcher":"x","arg":"",
            "locked":true,"release":true,"repeat":true,"longPress":true,"mouse":true,
            "non_consuming":true,"auto_consuming":true,"catch_all":true}]"#;
        assert_eq!(
            bind_list(reply).unwrap()[0].flags,
            ["locked", "release", "repeat", "long press", "mouse", "non-consuming", "auto-consuming", "catch-all"]
        );
    }

    #[test]
    fn a_binds_submap_description_and_dispatcher_are_kept_and_a_lua_function_has_none() {
        let binds = fx_binds();
        assert_eq!(binds[0].submap, "");
        assert_eq!(binds[0].description.as_deref(), Some("Terminal"));
        assert_eq!(binds[3].submap, "resize");
        assert_eq!(binds[0].dispatcher, None, "__lua is a Lua function");
        let reply = r#"[{"submap":"","key":"a","modmask":0,"dispatcher":"exec","arg":"kitty","has_description":false,"description":""},
            {"submap":"","key":"b","modmask":0,"dispatcher":"workspace","arg":3,"has_description":true,"description":"Go"}]"#;
        let binds = bind_list(reply).unwrap();
        assert_eq!((binds[0].dispatcher.as_deref(), binds[0].arg.as_str()), (Some("exec"), "kitty"));
        assert_eq!(binds[0].description, None);
        assert_eq!((binds[1].dispatcher.as_deref(), binds[1].arg.as_str()), (Some("workspace"), "3"));
        assert_eq!(binds[1].description.as_deref(), Some("Go"));
    }

    #[test]
    fn an_animation_has_its_name_on_speed_curve_style_and_overridden() {
        let animations = animation_lists(FX_ANIMATIONS).unwrap();
        assert_eq!(animations.list.len(), 7);
        let first = &animations.list[0];
        assert_eq!(first.name, "specialWorkspaceOut");
        assert_eq!((first.enabled, first.speed, first.overridden), (true, 0.0, false));
        assert_eq!((first.curve.as_str(), first.style.as_str()), ("", ""));
        let workspaces = animations.list.iter().find(|a| a.name == "workspacesIn").unwrap();
        assert_eq!((workspaces.enabled, workspaces.speed, workspaces.overridden), (true, 1.21, true));
        assert_eq!((workspaces.curve.as_str(), workspaces.style.as_str()), ("almostLinear", "fade"));
    }

    #[test]
    fn a_curve_has_its_name_and_two_points() {
        let animations = animation_lists(FX_ANIMATIONS).unwrap();
        assert_eq!(animations.curves.len(), 3);
        assert_eq!(animations.curves[0], Curve { name: "quick".into(), points: [(0.15, 0.0), (0.1, 1.0)] });
        assert_eq!(animations.curves[1].points, [(0.5, 0.5), (0.75, 1.0)]);
        // `default` is in the list here; a spring such as `spring:easy` or an empty
        // name is named by animations without being in it.
        assert!(animations.curves.iter().any(|c| c.name == "default"));
        assert!(animations.list.iter().any(|a| a.curve == "spring:easy"));
    }

    #[test]
    fn devices_are_grouped_by_kind() {
        let devices = device_lists(FX_DEVICES).unwrap();
        assert_eq!(devices.mice.len(), 2);
        assert_eq!(devices.mice[0], Mouse { name: "example-mouse".into(), speed: 0.0 });
        assert_eq!(devices.keyboards.len(), 2);
        assert_eq!(
            devices.keyboards[1],
            Keyboard {
                name: "example-keyboard".into(),
                layout: "us".into(),
                variant: "".into(),
                keymap: "English (US)".into(),
                main: false
            }
        );
        assert_eq!(devices.touch, ["example-touchscreen"]);
        assert_eq!(devices.tablets, Vec::<String>::new());
        assert_eq!(devices.switches, ["Lid Switch"]);
        let reply = r#"{"mice":[],"keyboards":[{"name":"k","layout":"de","variant":"nodeadkeys","active_keymap":"German","main":true}],
            "tablets":[{"name":"pen","address":"0x1"}],"touch":[],"switches":[]}"#;
        let devices = device_lists(reply).unwrap();
        assert_eq!((devices.keyboards[0].variant.as_str(), devices.keyboards[0].main), ("nodeadkeys", true));
        assert_eq!(devices.tablets, ["pen"]);
    }

    #[test]
    fn a_malformed_list_gives_none_for_its_own_part() {
        assert_eq!(bind_list(r#"[{"key":"a"}]"#), None);
        assert_eq!(bind_list("not json"), None);
        assert_eq!(animation_lists("[1,2]"), None);
        assert_eq!(animation_lists("[[],[{\"name\":\"c\"}]]"), None);
        assert_eq!(device_lists("[]"), None);
        assert_eq!(device_lists(r#"{"mice":[{"name":"m"}]}"#), None);
        for cut in 0..FX_ANIMATIONS.len().min(300) {
            assert_eq!(animation_lists(&FX_ANIMATIONS[..cut]), None, "animations cut at {cut}");
        }
    }

    /// A `Config` that has not fetched the option names; `details` does not need them.
    fn bare() -> Config {
        Config { names: None }
    }

    const DETAILS_BATCH: &str = "[[BATCH]]j/binds;j/animations;j/devices";

    /// Answers `details` with the captured fixtures.
    fn fixtures(command: &str) -> Option<String> {
        Some(match command {
            "j/descriptions" => FX_DESCRIPTIONS.into(),
            DETAILS_BATCH => replies(&[FX_BINDS, FX_ANIMATIONS, FX_DEVICES]),
            c if c.starts_with("[[BATCH]]j/getoption ") => FX_GETOPTIONS.into(),
            other => panic!("unexpected command {other}"),
        })
    }

    #[test]
    fn details_sends_descriptions_the_option_batch_and_one_batch_for_the_rest() {
        let (_dir, hypr, seen) = fake("details", 3, fixtures);
        let details = bare().details(&hypr);
        let names: Vec<String> = fx_options().into_iter().map(|o| o.name).collect();
        let batch = format!("[[BATCH]]{}", names.iter().map(|n| format!("j/getoption {n}")).collect::<Vec<_>>().join(";"));
        assert_eq!(*seen.lock().unwrap(), ["j/descriptions".to_string(), batch, DETAILS_BATCH.to_string()]);
        assert_eq!(details.options, Some(fx_options()));
        assert_eq!(details.binds, Some(fx_binds()));
        assert_eq!(details.animations, animation_lists(FX_ANIMATIONS));
        assert_eq!(details.devices, device_lists(FX_DEVICES));
        assert!(details.options.is_some() && details.binds.is_some() && details.animations.is_some() && details.devices.is_some());
    }

    #[test]
    fn a_dropped_reply_leaves_only_its_own_parts_empty() {
        let (_dir, hypr, _) = fake("details-drop-batch", 3, |c| if c == DETAILS_BATCH { None } else { fixtures(c) });
        let details = bare().details(&hypr);
        assert_eq!(details.options, Some(fx_options()));
        assert_eq!((details.binds, details.animations, details.devices), (None, None, None));

        let (_dir, hypr, _) = fake("details-drop-opts", 3, |c| if c.starts_with("[[BATCH]]j/getoption") { None } else { fixtures(c) });
        let details = bare().details(&hypr);
        assert_eq!(details.options, None);
        assert_eq!(details.binds, Some(fx_binds()));

        let (_dir, hypr, seen) = fake("details-drop-desc", 2, |c| if c == "j/descriptions" { None } else { fixtures(c) });
        let details = bare().details(&hypr);
        assert_eq!(details.options, None);
        assert_eq!(details.devices, device_lists(FX_DEVICES));
        assert_eq!(seen.lock().unwrap().len(), 2, "no option batch without names");
    }

    #[test]
    fn one_malformed_reply_in_the_batch_leaves_only_that_part_empty() {
        let (_dir, hypr, _) = fake("details-bad", 3, |c| {
            if c == DETAILS_BATCH {
                Some(replies(&[FX_BINDS, "garbage", FX_DEVICES]))
            } else {
                fixtures(c)
            }
        });
        let details = bare().details(&hypr);
        assert_eq!(details.binds, Some(fx_binds()));
        assert_eq!(details.animations, None);
        assert_eq!(details.devices, device_lists(FX_DEVICES));
    }

    #[test]
    fn a_missing_socket_gives_no_details_and_refresh_still_sends_what_it_did() {
        let hypr = Hypr::at(std::env::temp_dir().join(format!("hyprscope-cfgdet-{}/.socket.sock", std::process::id())));
        assert_eq!(bare().details(&hypr), Details::default());
        let (_dir, hypr, seen) = fake("refresh-same", 3, hyprland);
        let mut config = Config::new(&hypr);
        config.refresh(&hypr);
        assert_eq!(seen.lock().unwrap().len(), 3);
        assert_eq!(seen.lock().unwrap()[1], "[[BATCH]]j/configerrors;j/binds;j/animations");
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
