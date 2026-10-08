//! What Hyprland says about one window: its properties and decorations.

use crate::hypr::Hypr;
use serde_json::Value;

/// The property names to ask for. Hyprland has no request that lists them, so hyprscope asks
/// for every name it knows and shows the ones Hyprland answers; these are the 40 that
/// Hyprland 0.56 answers, and a name a version does not know is answered `prop not found`.
pub const PROPERTIES: [&str; 40] = [
    "opacity", "opacity_inactive", "opacity_fullscreen", "opacity_override",
    "opacity_inactive_override", "opacity_fullscreen_override", "rounding", "rounding_power",
    "border_size", "active_border_color", "inactive_border_color", "animation", "no_anim",
    "no_blur", "no_dim", "no_focus", "no_follow_mouse", "no_max_size", "no_shadow",
    "no_shortcuts_inhibit", "allows_input", "dim_around", "opaque", "force_rgbx",
    "sync_fullscreen", "immediate", "xray", "render_unfocused", "scroll_mouse",
    "scroll_touchpad", "stay_focused", "max_size", "min_size", "idle_inhibit",
    "persistent_size", "decorate", "nearest_neighbor", "focus_on_activate", "no_screen_share",
    "no_vrr",
];

/// One property: its name, its value as shown, and whether it is the value a window has
/// with no rule applied.
#[derive(Debug, Clone, PartialEq)]
pub struct Property {
    pub name: &'static str,
    pub value: String,
    pub plain: bool,
}

/// Everything fetched about one window; empty when Hyprland could not be asked.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Details {
    /// In `PROPERTIES` order, without the names Hyprland does not know.
    pub properties: Vec<Property>,
    /// Decoration names in the order Hyprland lists them.
    pub decorations: Vec<String>,
}

/// Separator Hyprland puts between the replies of a `[[BATCH]]` request.
const SEPARATOR: &str = "\n\n\n";

/// The names whose value is a switch, and so is plain when off, except the two that are on
/// by default.
const SWITCHES: [&str; 26] = [
    "opacity_override", "opacity_inactive_override", "opacity_fullscreen_override", "no_anim",
    "no_blur", "no_dim", "no_focus", "no_follow_mouse", "no_max_size", "no_shadow",
    "no_shortcuts_inhibit", "allows_input", "dim_around", "opaque", "force_rgbx",
    "sync_fullscreen", "immediate", "xray", "render_unfocused", "stay_focused",
    "persistent_size", "decorate", "nearest_neighbor", "focus_on_activate", "no_screen_share",
    "no_vrr",
];
const ON_BY_DEFAULT: [&str; 2] = ["sync_fullscreen", "decorate"];
const COLOURS: [&str; 2] = ["active_border_color", "inactive_border_color"];

/// Asks Hyprland for the window's properties and decorations in one batch. A request that
/// fails, or an address that is not hexadecimal digits (it goes into a `;`-joined batch),
/// gives empty details.
pub fn fetch(hypr: &Hypr, address: &str) -> Details {
    let digits = address.strip_prefix("0x").unwrap_or(address);
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return Details::default();
    }
    let address = format!("0x{digits}");
    let mut commands: Vec<String> = PROPERTIES.iter().map(|n| format!("j/getprop address:{address} {n}")).collect();
    commands.push(format!("j/decorations address:{address}"));
    match hypr.request(&format!("[[BATCH]]{}", commands.join(";"))) {
        Ok(reply) => parse(&reply),
        Err(_) => Details::default(),
    }
}

/// Reads the reply to the batch `fetch` sends.
fn parse(reply: &str) -> Details {
    let parts: Vec<&str> = reply.split(SEPARATOR).map(str::trim).collect();
    if parts.len() != PROPERTIES.len() + 1 {
        return Details::default();
    }
    let properties = PROPERTIES.iter().zip(&parts).filter_map(|(name, part)| property(name, part)).collect();
    let decorations = serde_json::from_str::<Vec<serde_json::Value>>(parts[PROPERTIES.len()])
        .unwrap_or_default()
        .iter()
        .filter_map(|d| d["decorationName"].as_str().map(str::to_string))
        .collect();
    Details { properties, decorations }
}

/// The property from the reply to `j/getprop`; `None` for `prop not found` or anything else
/// that is not `{"<name>": <value>}`.
fn property(name: &'static str, reply: &str) -> Option<Property> {
    let value = serde_json::from_str::<serde_json::Value>(reply.trim()).ok()?.get_mut(name)?.take();
    Some(Property { name, value: show(name, &value), plain: is_plain(name, &value) })
}

/// The value as shown.
fn show(name: &str, value: &Value) -> String {
    match value {
        Value::Bool(on) => if *on { "on" } else { "off" }.to_string(),
        Value::Number(n) => n.as_f64().map_or_else(|| n.to_string(), |f| f.to_string()),
        Value::String(s) if s.is_empty() => "none".to_string(),
        Value::String(s) if COLOURS.contains(&name) => {
            s.split(' ').filter(|w| !w.ends_with("deg")).collect::<Vec<_>>().join(" ")
        }
        Value::String(s) => s.clone(),
        Value::Array(parts) => match &parts[..] {
            [Value::Null, Value::Null] => "none".to_string(),
            [Value::Number(w), Value::Number(h)] => format!("{w}x{h}"),
            _ => value.to_string(),
        },
        other => other.to_string(),
    }
}

/// Whether the value is the one a window has with no rule applied. A value of a type the
/// property does not have is not plain.
fn is_plain(name: &str, value: &Value) -> bool {
    match (name, value) {
        ("opacity" | "opacity_inactive" | "opacity_fullscreen" | "scroll_mouse" | "scroll_touchpad", Value::Number(n)) => {
            n.as_f64() == Some(1.0)
        }
        ("idle_inhibit", Value::Number(n)) => n.as_f64() == Some(0.0),
        ("animation", Value::String(s)) => s.is_empty(),
        ("max_size", Value::Array(parts)) => matches!(&parts[..], [Value::Null, Value::Null]),
        (_, Value::Bool(on)) if SWITCHES.contains(&name) => *on == ON_BY_DEFAULT.contains(&name),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TempDir;
    use std::io::{Read, Write};
    use std::os::unix::net::UnixListener;

    /// The property made from the reply to `name`, which must be one Hyprland answered.
    fn prop(name: &'static str, reply: &str) -> Property {
        property(name, reply).unwrap()
    }

    fn shown(name: &'static str, value: &str) -> String {
        prop(name, &format!("{{\"{name}\": {value}}}")).value
    }

    fn plain(name: &'static str, value: &str) -> bool {
        prop(name, &format!("{{\"{name}\": {value}}}")).plain
    }

    #[test]
    fn a_switch_reads_on_or_off() {
        assert_eq!(shown("no_blur", "true"), "on");
        assert_eq!(shown("no_blur", "false"), "off");
    }

    #[test]
    fn a_number_reads_as_hyprland_gives_it() {
        assert_eq!(shown("opacity", "1"), "1");
        assert_eq!(shown("opacity", "1.0"), "1");
        assert_eq!(shown("opacity_inactive", "0.9"), "0.9");
        assert_eq!(shown("rounding", "10"), "10");
        assert_eq!(shown("opacity", "0.500000"), "0.5");
    }

    #[test]
    fn a_size_reads_wxh_or_none() {
        assert_eq!(shown("min_size", "[20, 20]"), "20x20");
        assert_eq!(shown("max_size", "[null, null]"), "none");
        assert_eq!(shown("max_size", "[800, 600]"), "800x600");
    }

    #[test]
    fn a_colour_reads_as_its_hex_value() {
        assert_eq!(shown("active_border_color", "\"ffffb2b8 0deg\""), "ffffb2b8");
        assert_eq!(shown("active_border_color", "\"ffffb2b8 ff112233 45deg\""), "ffffb2b8 ff112233");
        assert_eq!(shown("inactive_border_color", "\"ff1a1112 0deg\""), "ff1a1112");
    }

    #[test]
    fn an_empty_animation_reads_none_and_other_text_as_it_is() {
        assert_eq!(shown("animation", "\"\""), "none");
        assert_eq!(shown("animation", "\"slide\""), "slide");
    }

    #[test]
    fn an_unexpected_type_is_shown_as_its_text_and_is_not_plain() {
        let p = prop("opacity", "{\"opacity\": \"high\"}");
        assert_eq!((p.value.as_str(), p.plain), ("high", false));
        let p = prop("no_blur", "{\"no_blur\": 1}");
        assert_eq!((p.value.as_str(), p.plain), ("1", false));
        let p = prop("max_size", "{\"max_size\": [1, null]}");
        assert!(!p.plain);
    }

    #[test]
    fn prop_not_found_and_unparseable_replies_give_no_property() {
        assert_eq!(property("alpha", "prop not found"), None);
        assert_eq!(property("opacity", "{broken"), None);
    }

    #[test]
    fn every_plain_value_is_plain_and_one_step_away_is_not() {
        for (name, plain_value, other) in [
            ("no_blur", "false", "true"),
            ("opacity_override", "false", "true"),
            ("allows_input", "false", "true"),
            ("sync_fullscreen", "true", "false"),
            ("decorate", "true", "false"),
            ("opacity", "1", "0.9"),
            ("opacity_inactive", "1", "0.9"),
            ("opacity_fullscreen", "1.0", "1.1"),
            ("scroll_mouse", "1", "2"),
            ("scroll_touchpad", "1", "0.5"),
            ("animation", "\"\"", "\"popin\""),
            ("max_size", "[null, null]", "[10, 10]"),
            ("idle_inhibit", "0", "1"),
        ] {
            assert!(plain(name, plain_value), "{name} {plain_value}");
            assert!(!plain(name, other), "{name} {other}");
        }
    }

    #[test]
    fn values_from_the_configuration_are_never_plain() {
        assert!(!plain("rounding", "0"));
        assert!(!plain("rounding_power", "2"));
        assert!(!plain("border_size", "0"));
        assert!(!plain("active_border_color", "\"\""));
        assert!(!plain("inactive_border_color", "\"ff1a1112 0deg\""));
        assert!(!plain("min_size", "[20, 20]"));
        assert!(!plain("min_size", "[null, null]"));
    }

    #[test]
    fn the_list_holds_forty_distinct_names_none_of_them_unknown_to_hyprland() {
        let mut names = PROPERTIES.to_vec();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 40);
        for unknown in ["alpha", "border_color", "no_border", "tag"] {
            assert!(!PROPERTIES.contains(&unknown), "{unknown}");
        }
    }

    const SEP: &str = "\n\n\n";

    /// A reply to the batch: one part per name in `PROPERTIES` (made by `answer`), then the
    /// decorations.
    fn batch_reply(answer: impl Fn(&str) -> String, decorations: &str) -> String {
        let mut parts: Vec<String> = PROPERTIES.iter().map(|n| answer(n)).collect();
        parts.push(decorations.to_string());
        parts.join(SEP)
    }

    fn answer_all(name: &str) -> String {
        format!("{{\"{name}\": false}}\n")
    }

    const DECORATIONS: &str = r#"[{"decorationName": "Drop Shadow", "priority": 10}, {"decorationName": "Border", "priority": 10000}, {"decorationName": "Inner Glow", "priority": 10}]"#;

    #[test]
    fn unknown_names_are_left_out_and_the_rest_keep_their_order() {
        let reply = batch_reply(
            |n| if n == "no_dim" || n == "opacity" { "prop not found".into() } else { answer_all(n) },
            "[]",
        );
        let d = parse(&reply);
        let names: Vec<_> = d.properties.iter().map(|p| p.name).collect();
        let expected: Vec<_> = PROPERTIES.iter().copied().filter(|n| *n != "no_dim" && *n != "opacity").collect();
        assert_eq!(names, expected);
    }

    #[test]
    fn decorations_are_listed_by_name_in_hyprlands_order() {
        let d = parse(&batch_reply(answer_all, DECORATIONS));
        assert_eq!(d.decorations, ["Drop Shadow", "Border", "Inner Glow"]);
        assert_eq!(d.properties.len(), 40);
    }

    #[test]
    fn the_wrong_number_of_parts_gives_empty_details() {
        let short = batch_reply(answer_all, "[]").rsplit_once(SEP).unwrap().0.to_string();
        assert_eq!(parse(&short), Details::default());
        assert_eq!(parse(""), Details::default());
        assert_eq!(parse(&format!("{}{SEP}[]", batch_reply(answer_all, "[]"))), Details::default());
    }

    /// Serves one connection; yields the request received.
    fn serve(name: &str, reply: Option<String>) -> (TempDir, Hypr, std::thread::JoinHandle<String>) {
        let dir = TempDir::new(name);
        let path = dir.join(".socket.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let handle = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = [0u8; 8192];
            let n = conn.read(&mut buf).unwrap();
            if let Some(reply) = reply {
                conn.write_all(reply.as_bytes()).unwrap();
            }
            String::from_utf8_lossy(&buf[..n]).into_owned()
        });
        (dir, Hypr::at(path), handle)
    }

    #[test]
    fn fetch_sends_one_batch_naming_the_address_in_every_part() {
        let (_dir, hypr, request) = serve("fetch", Some(batch_reply(answer_all, DECORATIONS)));
        let d = fetch(&hypr, "0x5609f1386480");
        assert_eq!(d.properties.len(), 40);
        assert_eq!(d.decorations.len(), 3);
        let request = request.join().unwrap();
        let commands = request.strip_prefix("[[BATCH]]").unwrap();
        let parts: Vec<_> = commands.split(';').collect();
        assert_eq!(parts.len(), 41);
        for (part, name) in parts.iter().zip(PROPERTIES) {
            assert_eq!(*part, format!("j/getprop address:0x5609f1386480 {name}"));
        }
        assert_eq!(parts[40], "j/decorations address:0x5609f1386480");
    }

    #[test]
    fn the_address_is_sent_with_exactly_one_0x() {
        for given in ["5609f1386480", "0x5609f1386480"] {
            let (_dir, hypr, request) = serve("addr", Some(batch_reply(answer_all, "[]")));
            fetch(&hypr, given);
            let request = request.join().unwrap();
            assert!(request.contains("address:0x5609f1386480 "), "{given}: {request}");
            assert!(!request.contains("0x0x"), "{given}");
        }
    }

    #[test]
    fn a_bad_address_gives_empty_details_and_sends_nothing() {
        for bad in ["5609f1; dispatch exit", "", "0x", "0xzz", "5609f1 ", "0x0x5609f1"] {
            let dir = TempDir::new("bad");
            let path = dir.join(".socket.sock");
            let listener = UnixListener::bind(&path).unwrap();
            listener.set_nonblocking(true).unwrap();
            assert_eq!(fetch(&Hypr::at(path), bad), Details::default(), "{bad:?}");
            let err = listener.accept().err().map(|e| e.kind());
            assert_eq!(err, Some(std::io::ErrorKind::WouldBlock), "{bad:?} reached the listener");
        }
    }

    #[test]
    fn a_dropped_reply_gives_empty_details() {
        let (_dir, hypr, request) = serve("drop", None);
        assert_eq!(fetch(&hypr, "0x1"), Details::default());
        request.join().unwrap();
    }

    #[test]
    fn no_socket_gives_empty_details() {
        let path = std::env::temp_dir().join(format!("hyprscope-inspect-none-{}/.socket.sock", std::process::id()));
        assert_eq!(fetch(&Hypr::at(path), "0x1"), Details::default());
    }

    #[test]
    fn only_getprop_and_decorations_are_sent() {
        let (_dir, hypr, request) = serve("only", Some(batch_reply(answer_all, "[]")));
        fetch(&hypr, "0x1");
        let request = request.join().unwrap();
        for part in request.strip_prefix("[[BATCH]]").unwrap().split(';') {
            assert!(part.starts_with("j/getprop ") || part.starts_with("j/decorations "), "{part}");
        }
    }
}
