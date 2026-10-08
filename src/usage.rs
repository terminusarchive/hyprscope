//! Reads what the Hyprland process is using from `/proc`.

use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// One reading of the process. Each field is `None` when its source is missing or malformed.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Sample {
    /// CPU use over the time since the previous sample, as a percentage of one core.
    pub cpu: Option<f64>,
    /// Resident memory in bytes.
    pub mem: Option<u64>,
    pub threads: Option<u32>,
    /// Time since the process started.
    pub uptime: Option<Duration>,
    /// `None` when no DRM client reports engine counters, or they cannot be read.
    pub gpu: Option<Gpu>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Gpu {
    /// Share of the time since the previous sample that the busiest engine was working.
    /// `None` until there is a previous sample to compare with.
    pub percent: Option<f64>,
    /// Graphics memory held, in bytes.
    pub mem: u64,
}

/// The most recent samples, oldest first.
#[derive(Clone)]
pub struct History {
    samples: VecDeque<Sample>,
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

impl History {
    /// More samples than any terminal is wide.
    pub const CAP: usize = 512;

    pub fn new() -> Self {
        History { samples: VecDeque::new() }
    }

    pub fn push(&mut self, sample: Sample) {
        if self.samples.len() == Self::CAP {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    /// Oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &Sample> {
        self.samples.iter()
    }
}

/// The newest `width` values and the scale they are drawn on. Both `spark` and the area graph
/// take their scale from here, so the two cannot disagree.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Scale {
    /// The visible values, a negative or missing one counted as nothing and the lowest taken
    /// away when the scale starts from there.
    pub values: Vec<f64>,
    /// What zero on the scale stands for: zero, or the lowest visible value.
    pub low: f64,
    /// The size of the scale: the largest value, but at least the floor.
    pub span: f64,
    /// The true highest visible value.
    pub peak: f64,
}

/// The scale for the newest `width` of `values`: from zero, or from the lowest visible value
/// when `from_low`, across at least `floor`.
pub(crate) fn scale(values: &[f64], width: usize, floor: f64, from_low: bool) -> Scale {
    let visible = &values[values.len().saturating_sub(width)..];
    // `max` skips a NaN, and a negative value counts as nothing.
    let clean: Vec<f64> = visible.iter().map(|v| v.max(0.0)).collect();
    let low = if from_low { clean.iter().copied().fold(f64::INFINITY, f64::min) } else { 0.0 };
    let low = if low.is_finite() { low } else { 0.0 };
    let values: Vec<f64> = clean.iter().map(|v| v - low).collect();
    let span = values.iter().copied().fold(0.0, f64::max).max(floor);
    let peak = visible.iter().copied().fold(0.0, f64::max);
    Scale { values, low, span, peak }
}

fn bars(scale: &Scale, width: usize) -> String {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let bars = scale.values.iter().map(|v| {
        let level = if scale.span > 0.0 { (v / scale.span * 7.0) as usize } else { 0 };
        BARS[level.min(7)]
    });
    let blanks = std::iter::repeat_n(' ', width - scale.values.len());
    blanks.chain(bars).collect()
}

/// The newest `width` values as bars, right-aligned with blanks to the left, and the largest
/// of those values. Bars are scaled so the tallest is the full block, except that the scale
/// never goes below `floor`.
pub fn spark(values: &[f64], width: usize, floor: f64) -> (String, f64) {
    let scale = scale(values, width, floor, false);
    (bars(&scale, width), scale.peak)
}

/// Like `spark`, but the bars run from the lowest of the newest `width` values up to the
/// highest, across at least `min_span`. The returned peak is the true highest value.
pub fn spark_from_low(values: &[f64], width: usize, min_span: f64) -> (String, f64) {
    let scale = scale(values, width, min_span, true);
    (bars(&scale, width), scale.peak)
}

type Counters = HashMap<(String, u64, String), Counter>;

pub struct Sampler {
    proc_dir: PathBuf,
    uptime_file: PathBuf,
    /// Time and CPU ticks of the previous sample.
    prev: Option<(Instant, u64)>,
    /// Time and counters of the previous sample, by device, client id and engine.
    gpu_prev: Option<(Instant, Counters)>,
    /// When the process's whole fdinfo directory was last read, and which descriptors in it were
    /// DRM clients. Between rescans only those are read.
    drm_fds: Option<(Instant, Vec<OsString>)>,
}

/// How long the remembered DRM descriptors are trusted before the whole fdinfo directory is
/// read again, which is how a client opened later is found.
const RESCAN: Duration = Duration::from_secs(10);

impl Sampler {
    pub fn new(proc_dir: impl Into<PathBuf>, uptime_file: impl Into<PathBuf>) -> Self {
        Sampler { proc_dir: proc_dir.into(), uptime_file: uptime_file.into(), prev: None, gpu_prev: None, drm_fds: None }
    }

    /// The DRM clients of the process: from every descriptor when it is first seen and every
    /// `RESCAN` after, else from the descriptors that were clients at the last scan, which a
    /// descriptor that has gone or is no longer a client leaves quietly. `None` when the
    /// directory cannot be read.
    fn clients(&mut self, now: Instant) -> Option<Vec<Client>> {
        let dir = self.proc_dir.join("fdinfo");
        let remembered = match &self.drm_fds {
            Some((at, fds)) if now.checked_duration_since(*at).is_some_and(|age| age < RESCAN) => Some(fds.clone()),
            _ => None,
        };
        let full = remembered.is_none();
        let names = match remembered {
            Some(names) => names,
            None => {
                let mut names: Vec<OsString> =
                    std::fs::read_dir(&dir).ok()?.filter_map(|e| Some(e.ok()?.file_name())).collect();
                names.sort();
                names
            }
        };
        let found: Vec<(OsString, Client)> = names
            .into_iter()
            .filter_map(|name| {
                let client = std::fs::read_to_string(dir.join(&name)).ok().as_deref().and_then(parse_fdinfo)?;
                Some((name, client))
            })
            .collect();
        let fds = found.iter().map(|(name, _)| name.clone()).collect();
        self.drm_fds = match (&self.drm_fds, full) {
            (Some((at, _)), false) => Some((*at, fds)),
            _ => Some((now, fds)),
        };
        // One client per device and `drm-client-id`: several descriptors can refer to the same one.
        let mut clients: Vec<Client> = Vec::new();
        for (_, client) in found {
            if clients.iter().all(|o| (&o.pdev, o.id) != (&client.pdev, client.id)) {
                clients.push(client);
            }
        }
        Some(clients)
    }

    pub fn sample(&mut self, now: Instant) -> Sample {
        let read = |name: &str| std::fs::read_to_string(self.proc_dir.join(name)).ok();
        let stat = read("stat").as_deref().and_then(parse_stat);
        let (mem, threads) = read("status").as_deref().map_or((None, None), parse_status);
        let boot_secs = std::fs::read_to_string(&self.uptime_file).ok().as_deref().and_then(parse_uptime);

        let cpu = stat.as_ref().and_then(|stat| {
            let (then, ticks) = self.prev?;
            let gained = stat.cpu_ticks.checked_sub(ticks)?;
            let secs = now.checked_duration_since(then)?.as_secs_f64();
            (secs > 0.0).then(|| gained as f64 / secs)
        });
        if let Some(stat) = &stat {
            self.prev = Some((now, stat.cpu_ticks));
        }
        let uptime = stat.zip(boot_secs).and_then(|(stat, boot)| {
            Duration::try_from_secs_f64(boot - stat.start_ticks as f64 / TICKS_PER_SEC).ok()
        });
        Sample { cpu, mem, threads, uptime, gpu: self.sample_gpu(now) }
    }
}

impl Sampler {
    /// GPU use from the process's DRM clients: for each engine, the busy gain of all clients
    /// over the gain of the time it is measured against, and the busiest engine is the figure.
    fn sample_gpu(&mut self, now: Instant) -> Option<Gpu> {
        let clients: Vec<Client> = self.clients(now)?.into_iter().filter(|c| !c.engines.is_empty()).collect();
        let mut percent = None;
        if let Some((then, prev)) = &self.gpu_prev {
            let elapsed_ns = now.checked_duration_since(*then).map_or(0.0, |d| d.as_nanos() as f64);
            // Per engine: busy gain summed over clients, and the longest gain of its clock.
            let mut engines: HashMap<&str, (f64, f64)> = HashMap::new();
            for client in &clients {
                for (name, now) in &client.engines {
                    let Some(before) = prev.get(&(client.pdev.clone(), client.id, name.clone())) else { continue };
                    let Some(busy) = now.busy.checked_sub(before.busy) else { continue };
                    let clock = match (now.total, before.total) {
                        (Some(t), Some(b)) => t.checked_sub(b).map(|g| g as f64),
                        (None, None) => Some(elapsed_ns),
                        _ => None,
                    };
                    let Some(clock) = clock else { continue };
                    let entry = engines.entry(name).or_default();
                    entry.0 += busy as f64;
                    entry.1 = entry.1.max(clock);
                }
            }
            percent = engines
                .values()
                .filter(|(_, clock)| *clock > 0.0)
                .map(|(busy, clock)| (busy / clock * 100.0).min(100.0))
                .reduce(f64::max);
        }
        let counters = clients
            .iter()
            .flat_map(|c| c.engines.iter().map(|(name, counter)| ((c.pdev.clone(), c.id, name.clone()), *counter)))
            .collect();
        self.gpu_prev = Some((now, counters));
        (!clients.is_empty()).then(|| Gpu { percent, mem: clients.iter().map(|c| c.mem).sum() })
    }
}

/// One engine's counters. `total` is the cycle form's clock; the time form has none.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Counter {
    busy: u64,
    total: Option<u64>,
}

/// What one fdinfo file says about a DRM client.
#[derive(Debug, PartialEq)]
struct Client {
    /// The `drm-pdev` device, empty when the driver does not say; ids are unique only per device.
    pdev: String,
    id: u64,
    /// By the counter's name after `drm-`, such as `cycles-rcs` or `engine-gfx`.
    engines: Vec<(String, Counter)>,
    /// Resident graphics memory in bytes, summed over regions.
    mem: u64,
}

fn parse_fdinfo(text: &str) -> Option<Client> {
    let mut id = None;
    let mut pdev = String::new();
    let mut engines = Vec::new();
    let mut totals: HashMap<&str, u64> = HashMap::new();
    let mut mem = 0u64;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else { continue };
        let Some(key) = key.strip_prefix("drm-") else { continue };
        let value = value.trim();
        if key == "pdev" {
            pdev = value.to_string();
        } else if key == "client-id" {
            id = value.parse().ok();
        } else if let Some(engine) = key.strip_prefix("total-cycles-") {
            if let Ok(n) = value.parse() {
                totals.insert(engine, n);
            }
        } else if key.starts_with("cycles-") {
            if let Ok(busy) = value.parse() {
                engines.push((key, busy, true));
            }
        } else if key.starts_with("engine-") && !key.starts_with("engine-capacity-") {
            if let Some(busy) = value.strip_suffix("ns").and_then(|v| v.trim().parse().ok()) {
                engines.push((key, busy, false));
            }
        } else if key.starts_with("resident-") {
            mem = mem.saturating_add(parse_bytes(value).unwrap_or(0));
        }
    }
    let engines = engines
        .into_iter()
        .filter_map(|(key, busy, cycles)| {
            let total = if cycles { Some(*totals.get(key.strip_prefix("cycles-")?)?) } else { None };
            Some((key.to_string(), Counter { busy, total }))
        })
        .collect();
    Some(Client { pdev, id: id?, engines, mem })
}

/// A size as `fdinfo` writes it: bare bytes, or a number with `KiB` or `MiB`.
fn parse_bytes(value: &str) -> Option<u64> {
    let (number, unit) = match value.split_once(char::is_whitespace) {
        Some((number, unit)) => (number, unit.trim()),
        None => (value, ""),
    };
    let scale = match unit {
        "" => 1,
        "KiB" => 1024,
        "MiB" => 1024 * 1024,
        _ => return None,
    };
    number.parse::<u64>().ok()?.checked_mul(scale)
}

#[derive(Debug, PartialEq)]
pub(crate) struct Stat {
    /// The parent's process ID.
    pub ppid: u32,
    /// `utime + stime`, in ticks of 1/100 s.
    pub cpu_ticks: u64,
    /// Start time after boot, in ticks of 1/100 s.
    pub start_ticks: u64,
}

/// Linux fixes the unit of the tick counters in `stat` at 1/100 s for user space.
const TICKS_PER_SEC: f64 = 100.0;

pub(crate) fn parse_stat(text: &str) -> Option<Stat> {
    // The name in parentheses may hold spaces and `)`, so count fields after the last `)`.
    // They start at the 3rd field of the line, so utime is index 11 and starttime index 19.
    let rest = text.get(text.rfind(')')? + 1..)?;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let field = |i: usize| fields.get(i)?.parse::<u64>().ok();
    Some(Stat { ppid: field(1)?.try_into().ok()?, cpu_ticks: field(11)?.checked_add(field(12)?)?, start_ticks: field(19)? })
}

fn parse_status(text: &str) -> (Option<u64>, Option<u32>) {
    let value = |key: &str| {
        text.lines().find_map(|line| line.strip_prefix(key)?.strip_prefix(':')).map(str::trim)
    };
    let rss = value("VmRSS").and_then(|v| v.strip_suffix("kB")?.trim().parse::<u64>().ok());
    let threads = value("Threads").and_then(|v| v.parse().ok());
    (rss.and_then(|kb| kb.checked_mul(1024)), threads)
}

fn parse_uptime(text: &str) -> Option<f64> {
    text.split_whitespace().next()?.parse().ok()
}

/// What identifies a process, apart from the figures a `Sampler` reads. Each field is `None`
/// when its source is missing or unreadable.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProcessInfo {
    /// The arguments joined with single spaces.
    pub cmdline: Option<String>,
    /// The working directory, with a leading `home` written as `~`.
    pub cwd: Option<String>,
    /// The name of the parent process, from the parent's `comm` beside this process's directory.
    pub parent: Option<String>,
}

/// Reads `cmdline`, `cwd` and the parent's name for the process directory `proc_dir`, such as
/// `/proc/1234`. The parent's directory is looked up beside it.
pub fn describe(proc_dir: &Path, home: Option<&Path>) -> ProcessInfo {
    let cmdline = std::fs::read(proc_dir.join("cmdline")).ok().and_then(|bytes| {
        let text = String::from_utf8_lossy(&bytes);
        let args: Vec<&str> = text.split('\0').filter(|a| !a.is_empty()).collect();
        (!args.is_empty()).then(|| args.join(" "))
    });
    let cwd = std::fs::read_link(proc_dir.join("cwd")).ok().map(|path| match home.and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => Path::new("~").join(rest).to_string_lossy().into_owned(),
        None => path.to_string_lossy().into_owned(),
    });
    let parent = std::fs::read_to_string(proc_dir.join("status"))
        .ok()
        .and_then(|status| status.lines().find_map(|l| l.strip_prefix("PPid:")?.trim().parse::<u64>().ok()))
        .zip(proc_dir.parent())
        .and_then(|(ppid, root)| std::fs::read_to_string(root.join(ppid.to_string()).join("comm")).ok())
        .map(|name| name.trim_end().to_string())
        .filter(|name| !name.is_empty());
    ProcessInfo { cmdline, cwd, parent }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use crate::testdir::TempDir;

    const XE_ACTIVE: &str = include_str!("../tests/fixtures/fdinfo-xe/52");
    const XE_IDLE: &str = include_str!("../tests/fixtures/fdinfo-xe/42");
    const XE_MEMORY_ONLY: &str = include_str!("../tests/fixtures/fdinfo-xe/45");
    // Written by hand from the kernel's Documentation/gpu/drm-usage-stats.rst.
    const TIME_FORM: &str = include_str!("../tests/fixtures/fdinfo-amdgpu.txt");

    const STAT: &str = include_str!("../tests/fixtures/hyprland/stat");
    const STATUS: &str = include_str!("../tests/fixtures/hyprland/status");
    const UPTIME: &str = include_str!("../tests/fixtures/hyprland/uptime");

    /// A stat line whose `utime + stime` is `ticks`.
    fn stat_with(ticks: u64) -> String {
        format!("4242 (Hyprland) S 1 1 1 0 -1 0 0 0 0 0 {ticks} 0 0 0 20 0 17 0 2413 0 0\n")
    }

    /// A process directory and uptime file, filled as a real system would.
    fn fake_proc(name: &str) -> (TempDir, PathBuf, PathBuf) {
        let root = TempDir::new(&format!("usage-{name}"));
        let proc = root.join("4242");
        fs::create_dir(&proc).unwrap();
        fs::write(proc.join("stat"), STAT).unwrap();
        fs::write(proc.join("status"), STATUS).unwrap();
        fs::write(root.join("uptime"), UPTIME).unwrap();
        let uptime = root.join("uptime");
        (root, proc, uptime)
    }

    #[test]
    fn stat_fixture_gives_ticks_and_start_time() {
        assert_eq!(parse_stat(STAT), Some(Stat { ppid: 1000, cpu_ticks: 23925 + 14672, start_ticks: 2413 }));
    }

    #[test]
    fn stat_with_spaces_and_parenthesis_in_the_name_parses() {
        let text = "42 (my ) S (x) R 1 1 1 0 -1 0 0 0 0 0 7 5 0 0 20 0 3 0 99 0 0\n";
        assert_eq!(parse_stat(text), Some(Stat { ppid: 1, cpu_ticks: 12, start_ticks: 99 }));
    }

    #[test]
    fn malformed_stat_gives_none() {
        assert_eq!(parse_stat(""), None);
        assert_eq!(parse_stat("1 (x) S 1 2"), None);
        assert_eq!(parse_stat("1 (x) S a b c d e f g h i j k l m n o p q r s t u"), None);
    }

    #[test]
    fn status_fixture_gives_resident_bytes_and_threads() {
        assert_eq!(parse_status(STATUS), (Some(145048 * 1024), Some(17)));
    }

    #[test]
    fn status_missing_a_line_gives_none_for_that_field_only() {
        assert_eq!(parse_status("Threads:\t3\n"), (None, Some(3)));
        assert_eq!(parse_status("VmRSS:\t  10 kB\nThreads:\tmany\n"), (Some(10240), None));
    }

    #[test]
    fn uptime_is_the_first_number() {
        assert_eq!(parse_uptime(UPTIME), Some(9409.6));
        assert_eq!(parse_uptime("garbage"), None);
    }

    #[test]
    fn first_sample_has_no_cpu_but_the_other_fields() {
        let (_dir, proc, uptime) = fake_proc("first");
        let s = Sampler::new(proc, uptime).sample(Instant::now());
        assert_eq!(s.cpu, None);
        assert_eq!(s.mem, Some(145048 * 1024));
        assert_eq!(s.threads, Some(17));
        // 9409.60 s since boot, started 24.13 s after boot.
        assert_eq!(s.uptime, Some(Duration::from_secs_f64(9409.6 - 24.13)));
    }

    #[test]
    fn fifty_ticks_in_one_second_is_fifty_percent() {
        let (_dir, proc, uptime) = fake_proc("cpu");
        let mut sampler = Sampler::new(&proc, uptime);
        let t0 = Instant::now();
        fs::write(proc.join("stat"), stat_with(1000)).unwrap();
        sampler.sample(t0);
        fs::write(proc.join("stat"), stat_with(1050)).unwrap();
        let s = sampler.sample(t0 + Duration::from_secs(1));
        assert_eq!(s.cpu, Some(50.0));
    }

    #[test]
    fn two_cores_busy_reads_above_a_hundred() {
        let (_dir, proc, uptime) = fake_proc("multi");
        let mut sampler = Sampler::new(&proc, uptime);
        let t0 = Instant::now();
        fs::write(proc.join("stat"), stat_with(0)).unwrap();
        sampler.sample(t0);
        fs::write(proc.join("stat"), stat_with(100)).unwrap();
        let s = sampler.sample(t0 + Duration::from_millis(500));
        assert_eq!(s.cpu, Some(200.0));
    }

    #[test]
    fn a_missing_file_blanks_its_fields_and_leaves_the_others() {
        let (_dir, proc, uptime) = fake_proc("missing");
        fs::remove_file(proc.join("status")).unwrap();
        let s = Sampler::new(&proc, &uptime).sample(Instant::now());
        assert_eq!((s.mem, s.threads), (None, None));
        assert!(s.uptime.is_some());

        fs::remove_file(proc.join("stat")).unwrap();
        let s = Sampler::new(&proc, &uptime).sample(Instant::now());
        assert_eq!(s.uptime, None);

        let (_dir, proc, uptime) = fake_proc("missing-uptime");
        fs::remove_file(&uptime).unwrap();
        let s = Sampler::new(&proc, &uptime).sample(Instant::now());
        assert_eq!(s.uptime, None);
        assert_eq!(s.mem, Some(145048 * 1024));
    }

    #[test]
    fn a_directory_that_does_not_exist_gives_an_empty_sample() {
        let s = Sampler::new("/nonexistent/1", "/nonexistent/uptime").sample(Instant::now());
        assert_eq!(s, Sample::default());
    }

    /// `text` with the value of `key` replaced.
    fn with_value(text: &str, key: &str, value: &str) -> String {
        let prefix = format!("{key}:");
        text.lines()
            .map(|l| if l.starts_with(&prefix) { format!("{key}:\t{value}") } else { l.to_string() })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    }

    fn counter(client: &Client, name: &str) -> Counter {
        client.engines.iter().find(|(n, _)| n == name).unwrap().1
    }

    /// Replaces the process's fdinfo directory with these files.
    fn set_fdinfo(proc: &Path, files: &[(&str, &str)]) {
        let dir = proc.join("fdinfo");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir(&dir).unwrap();
        for (name, text) in files {
            fs::write(dir.join(name), text).unwrap();
        }
    }

    /// The GPU figures of two samples one second apart, with these fdinfo files before and after.
    fn gpu_over_a_second(before: &[(&str, &str)], after: &[(&str, &str)]) -> Option<Gpu> {
        let (_dir, proc, uptime) = fake_proc("gpu");
        let mut sampler = Sampler::new(&proc, uptime);
        let t0 = Instant::now();
        set_fdinfo(&proc, before);
        sampler.sample(t0);
        set_fdinfo(&proc, after);
        sampler.sample(t0 + Duration::from_secs(1)).gpu
    }

    fn percent(gpu: Option<Gpu>) -> f64 {
        gpu.unwrap().percent.unwrap()
    }

    fn assert_close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} vs {b}");
    }

    #[test]
    fn cycle_form_file_gives_client_engines_and_memory() {
        let c = parse_fdinfo(XE_ACTIVE).unwrap();
        assert_eq!(c.id, 8);
        assert_eq!(counter(&c, "cycles-rcs"), Counter { busy: 7235448854, total: Some(183203949089) });
        assert_eq!(counter(&c, "cycles-vcs").busy, 0);
        assert_eq!(c.engines.len(), 5);
        assert_eq!(c.mem, 1325340 * 1024);
    }

    #[test]
    fn time_form_file_gives_engines_and_memory_in_kib_mib_and_bytes() {
        let c = parse_fdinfo(TIME_FORM).unwrap();
        assert_eq!(c.id, 21);
        assert_eq!(counter(&c, "engine-gfx"), Counter { busy: 1_000_000_000, total: None });
        assert_eq!(counter(&c, "engine-compute").busy, 2_000_000_000);
        assert_eq!(c.engines.len(), 2);
        // The GiB region and the older `drm-memory-` line are left out.
        assert_eq!(c.mem, 512 * 1024 * 1024 + 2048 * 1024 + 4096);
    }

    #[test]
    fn a_file_that_is_not_a_drm_client_gives_none() {
        assert_eq!(parse_fdinfo("pos:\t0\nflags:\t02100002\nmnt_id:\t17\n"), None);
        assert_eq!(parse_fdinfo(""), None);
        assert_eq!(parse_fdinfo("drm-client-id:\tx\ndrm-cycles-rcs:\t1\n"), None);
    }

    #[test]
    fn cycle_form_gives_busy_gain_over_total_gain() {
        let after = with_value(XE_ACTIVE, "drm-cycles-rcs", &(7235448854u64 + 851380).to_string());
        let after = with_value(&after, "drm-total-cycles-rcs", &(183203949089u64 + 19400187).to_string());
        let gpu = gpu_over_a_second(&[("52", XE_ACTIVE)], &[("52", &after)]);
        assert_close(percent(gpu.clone()), 851380.0 / 19400187.0 * 100.0);
        assert_eq!(gpu.unwrap().mem, 1325340 * 1024);
    }

    #[test]
    fn descriptors_sharing_a_client_id_count_once() {
        let bump = |t: &str| {
            let t = with_value(t, "drm-cycles-rcs", "7235448854");
            with_value(&t, "drm-total-cycles-rcs", "183203949089")
        };
        let before = bump(XE_ACTIVE);
        let after = with_value(&before, "drm-cycles-rcs", "7335448854");
        let after = with_value(&after, "drm-total-cycles-rcs", "193203949089");
        let gpu = gpu_over_a_second(
            &[("52", &before), ("53", &before)],
            &[("52", &after), ("53", &after)],
        );
        // 100 million of 10 billion cycles, once; and memory once too.
        assert_close(percent(gpu.clone()), 1.0);
        assert_eq!(gpu.unwrap().mem, 1325340 * 1024);
    }

    /// XE_ACTIVE on the device `pdev`, with its busy counter and clock at these values.
    fn on_device(pdev: &str, busy: u64, total: u64) -> String {
        let t = with_value(XE_ACTIVE, "drm-cycles-rcs", &busy.to_string());
        let t = with_value(&t, "drm-total-cycles-rcs", &total.to_string());
        with_value(&t, "drm-pdev", pdev)
    }

    #[test]
    fn the_same_client_id_on_two_devices_is_two_clients() {
        let (b1, b2) = (on_device("0000:00:02.0", 0, 0), on_device("0000:03:00.0", 0, 0));
        let (a1, a2) = (on_device("0000:00:02.0", 100, 10_000), on_device("0000:03:00.0", 100, 10_000));
        let gpu = gpu_over_a_second(&[("52", &b1), ("53", &b2)], &[("52", &a1), ("53", &a2)]);
        // 200 busy cycles of 10000, and the memory of both.
        assert_close(percent(gpu.clone()), 2.0);
        assert_eq!(gpu.unwrap().mem, 2 * 1325340 * 1024);
    }

    #[test]
    fn the_same_client_id_on_the_same_device_is_one_client() {
        let (b, a) = (on_device("0000:00:02.0", 0, 0), on_device("0000:00:02.0", 100, 10_000));
        let gpu = gpu_over_a_second(&[("52", &b), ("53", &b)], &[("52", &a), ("53", &a)]);
        assert_close(percent(gpu.clone()), 1.0);
        assert_eq!(gpu.unwrap().mem, 1325340 * 1024);
    }

    #[test]
    fn gpu_use_is_capped_at_a_hundred_percent() {
        let before = with_value(&with_value(XE_ACTIVE, "drm-cycles-rcs", "0"), "drm-total-cycles-rcs", "0");
        let after = with_value(&with_value(&before, "drm-cycles-rcs", "300"), "drm-total-cycles-rcs", "100");
        assert_close(percent(gpu_over_a_second(&[("52", &before)], &[("52", &after)])), 100.0);
    }

    #[test]
    fn the_busiest_engine_is_the_one_reported() {
        let step = |t: &str, rcs: u64, ccs: u64, total: u64| {
            let t = with_value(t, "drm-cycles-rcs", &rcs.to_string());
            let t = with_value(&t, "drm-cycles-ccs", &ccs.to_string());
            ["rcs", "bcs", "vcs", "vecs", "ccs"]
                .iter()
                .fold(t, |t, e| with_value(&t, &format!("drm-total-cycles-{e}"), &total.to_string()))
        };
        let before = step(XE_ACTIVE, 1000, 1000, 1_000_000);
        let after = step(XE_ACTIVE, 101_000, 301_000, 2_000_000);
        // rcs worked 10% of the time, ccs 30%.
        assert_close(percent(gpu_over_a_second(&[("52", &before)], &[("52", &after)])), 30.0);
    }

    #[test]
    fn engines_add_up_across_clients_but_the_clock_does_not() {
        let other = with_value(XE_IDLE, "drm-cycles-rcs", "0");
        let other_after = with_value(&other, "drm-cycles-rcs", "30000");
        let a = with_value(XE_ACTIVE, "drm-cycles-rcs", "0");
        let a_after = with_value(&a, "drm-cycles-rcs", "20000");
        let clock = |t: &str, v: u64| with_value(t, "drm-total-cycles-rcs", &v.to_string());
        let gpu = gpu_over_a_second(
            &[("52", &clock(&a, 0)), ("42", &clock(&other, 0))],
            &[("52", &clock(&a_after, 100_000)), ("42", &clock(&other_after, 100_000))],
        );
        // 20000 + 30000 busy cycles over one clock that gained 100000.
        assert_close(percent(gpu), 50.0);
    }

    #[test]
    fn time_form_gives_busy_nanoseconds_over_elapsed_nanoseconds() {
        let after = with_value(TIME_FORM, "drm-engine-gfx", "1250000000 ns");
        let after = with_value(&after, "drm-engine-compute", "2400000000 ns");
        // gfx worked 25% of the second, compute 40%.
        assert_close(percent(gpu_over_a_second(&[("9", TIME_FORM)], &[("9", &after)])), 40.0);
    }

    #[test]
    fn graphics_memory_is_summed_over_clients() {
        let gpu = gpu_over_a_second(
            &[("52", XE_ACTIVE), ("45", XE_MEMORY_ONLY), ("9", TIME_FORM)],
            &[("52", XE_ACTIVE), ("45", XE_MEMORY_ONLY), ("9", TIME_FORM)],
        );
        let expected = (1325340 + 179240) * 1024 + 512 * 1024 * 1024 + 2048 * 1024 + 4096;
        assert_eq!(gpu.unwrap().mem, expected);
    }

    #[test]
    fn the_first_sample_has_memory_but_no_percentage() {
        let (_dir, proc, uptime) = fake_proc("gpu-first");
        set_fdinfo(&proc, &[("52", XE_ACTIVE)]);
        let gpu = Sampler::new(&proc, uptime).sample(Instant::now()).gpu;
        assert_eq!(gpu, Some(Gpu { percent: None, mem: 1325340 * 1024 }));
    }

    #[test]
    fn a_counter_that_goes_backwards_gives_no_percentage() {
        let after = with_value(XE_ACTIVE, "drm-cycles-rcs", "5");
        let gpu = gpu_over_a_second(&[("52", XE_ACTIVE)], &[("52", &after)]).unwrap();
        assert_eq!(gpu.percent, None);
    }

    #[test]
    fn no_drm_client_or_no_readable_directory_gives_no_gpu() {
        let (_dir, proc, uptime) = fake_proc("no-gpu");
        let mut sampler = Sampler::new(&proc, uptime);
        assert_eq!(sampler.sample(Instant::now()).gpu, None); // no fdinfo directory at all
        set_fdinfo(&proc, &[("0", "pos:\t0\nflags:\t02\n"), ("1", "")]);
        assert_eq!(sampler.sample(Instant::now()).gpu, None);
        // A client with a memory line but no engine counters is no source either.
        set_fdinfo(&proc, &[("2", "drm-client-id:\t3\ndrm-resident-gtt:\t5 KiB\n")]);
        assert_eq!(sampler.sample(Instant::now()).gpu, None);
    }

    /// An fdinfo file for client `id` holding `kib` KiB, with an engine that is always at `busy_ms`.
    fn drm_client(id: u32, kib: u64) -> String {
        format!("drm-client-id:\t{id}\ndrm-engine-gfx:\t1000000 ns\ndrm-resident-vram:\t{kib} KiB\n")
    }

    fn graphics_memory(sampler: &mut Sampler, at: Instant) -> Option<u64> {
        sampler.sample(at).gpu.map(|g| g.mem / 1024)
    }

    #[test]
    fn between_rescans_only_the_descriptors_that_were_clients_are_read() {
        let (_dir, proc, uptime) = fake_proc("rescan-known");
        let mut sampler = Sampler::new(&proc, uptime);
        let t0 = Instant::now();
        set_fdinfo(&proc, &[("52", &drm_client(1, 100)), ("3", "pos:\t0\n")]);
        assert_eq!(graphics_memory(&mut sampler, t0), Some(100));
        // Descriptor 3 becomes a client and 52 changes. If 3 were read, the memory would be 350.
        fs::write(proc.join("fdinfo/3"), drm_client(2, 250)).unwrap();
        fs::write(proc.join("fdinfo/52"), drm_client(1, 120)).unwrap();
        assert_eq!(graphics_memory(&mut sampler, t0 + Duration::from_secs(1)), Some(120));
        assert_eq!(graphics_memory(&mut sampler, t0 + Duration::from_secs(9)), Some(120));
    }

    #[test]
    fn a_client_that_appears_later_is_counted_after_ten_seconds_and_not_before() {
        let (_dir, proc, uptime) = fake_proc("rescan-new");
        let mut sampler = Sampler::new(&proc, uptime);
        let t0 = Instant::now();
        set_fdinfo(&proc, &[("52", &drm_client(1, 100))]);
        assert_eq!(graphics_memory(&mut sampler, t0), Some(100));
        fs::write(proc.join("fdinfo/60"), drm_client(2, 40)).unwrap();
        assert_eq!(graphics_memory(&mut sampler, t0 + Duration::from_millis(9999)), Some(100));
        assert_eq!(graphics_memory(&mut sampler, t0 + Duration::from_secs(10)), Some(140));
        // The next ten seconds count from that scan.
        fs::write(proc.join("fdinfo/61"), drm_client(3, 5)).unwrap();
        assert_eq!(graphics_memory(&mut sampler, t0 + Duration::from_secs(19)), Some(140));
        assert_eq!(graphics_memory(&mut sampler, t0 + Duration::from_secs(20)), Some(145));
    }

    #[test]
    fn a_process_with_no_client_is_scanned_again_after_ten_seconds() {
        let (_dir, proc, uptime) = fake_proc("rescan-none");
        let mut sampler = Sampler::new(&proc, uptime);
        let t0 = Instant::now();
        set_fdinfo(&proc, &[("0", "pos:\t0\n")]);
        assert_eq!(graphics_memory(&mut sampler, t0), None);
        fs::write(proc.join("fdinfo/9"), drm_client(1, 7)).unwrap();
        assert_eq!(graphics_memory(&mut sampler, t0 + Duration::from_secs(5)), None);
        assert_eq!(graphics_memory(&mut sampler, t0 + Duration::from_secs(10)), Some(7));
    }

    #[test]
    fn a_remembered_descriptor_that_has_gone_leaves_the_other_figures_intact() {
        let (_dir, proc, uptime) = fake_proc("rescan-gone");
        let mut sampler = Sampler::new(&proc, uptime);
        let t0 = Instant::now();
        set_fdinfo(&proc, &[("52", &drm_client(1, 100)), ("53", &drm_client(2, 30))]);
        assert_eq!(graphics_memory(&mut sampler, t0), Some(130));
        fs::remove_file(proc.join("fdinfo/52")).unwrap();
        assert_eq!(graphics_memory(&mut sampler, t0 + Duration::from_secs(1)), Some(30));
        fs::remove_file(proc.join("fdinfo/53")).unwrap();
        assert_eq!(graphics_memory(&mut sampler, t0 + Duration::from_secs(2)), None);
        // The whole directory going is the same as no client.
        fs::remove_dir_all(proc.join("fdinfo")).unwrap();
        assert_eq!(graphics_memory(&mut sampler, t0 + Duration::from_secs(3)), None);
    }

    #[test]
    fn a_descriptor_number_reused_by_a_file_that_is_not_a_client_stops_counting() {
        let (_dir, proc, uptime) = fake_proc("rescan-reused");
        let mut sampler = Sampler::new(&proc, uptime);
        let t0 = Instant::now();
        set_fdinfo(&proc, &[("52", &drm_client(1, 100)), ("53", &drm_client(2, 30))]);
        assert_eq!(graphics_memory(&mut sampler, t0), Some(130));
        fs::write(proc.join("fdinfo/52"), "pos:\t0\nflags:\t02\n").unwrap();
        assert_eq!(graphics_memory(&mut sampler, t0 + Duration::from_secs(1)), Some(30));
        // It is forgotten: a client that takes the number again is found by the next rescan.
        fs::write(proc.join("fdinfo/52"), drm_client(1, 100)).unwrap();
        assert_eq!(graphics_memory(&mut sampler, t0 + Duration::from_secs(2)), Some(30));
        assert_eq!(graphics_memory(&mut sampler, t0 + Duration::from_secs(10)), Some(130));
    }

    #[test]
    fn a_rescan_gives_what_reading_every_descriptor_gives() {
        let (_dir, proc, uptime) = fake_proc("rescan-same");
        let mut sampler = Sampler::new(&proc, uptime);
        let t0 = Instant::now();
        set_fdinfo(&proc, &[("1", "pos:\t0\n"), ("52", &drm_client(1, 100)), ("53", &drm_client(1, 100)), ("54", &drm_client(2, 30))]);
        for second in [0, 10, 20] {
            // Two descriptors on one client count once.
            assert_eq!(graphics_memory(&mut sampler, t0 + Duration::from_secs(second)), Some(130));
        }
        fs::remove_file(proc.join("fdinfo/52")).unwrap();
        // The other descriptor of client 1 carries it on between rescans.
        assert_eq!(graphics_memory(&mut sampler, t0 + Duration::from_secs(21)), Some(130));
    }

    fn sample_with_cpu(cpu: f64) -> Sample {
        Sample { cpu: Some(cpu), ..Sample::default() }
    }

    #[test]
    fn history_keeps_samples_oldest_first() {
        let mut h = History::new();
        for cpu in [1.0, 2.0, 3.0] {
            h.push(sample_with_cpu(cpu));
        }
        assert_eq!(h.iter().count(), 3);
        let cpus: Vec<_> = h.iter().map(|s| s.cpu.unwrap()).collect();
        assert_eq!(cpus, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn history_drops_its_oldest_sample_at_the_cap() {
        let mut h = History::new();
        for i in 0..History::CAP + 3 {
            h.push(sample_with_cpu(i as f64));
        }
        assert_eq!(h.iter().count(), History::CAP);
        assert_eq!(h.iter().next().unwrap().cpu, Some(3.0));
        assert_eq!(h.iter().last().unwrap().cpu, Some((History::CAP + 2) as f64));
    }

    #[test]
    fn spark_has_one_bar_per_value_newest_at_the_right() {
        let (bars, _) = spark(&[0.0, 50.0, 100.0], 3, 0.0);
        let bars: Vec<char> = bars.chars().collect();
        assert_eq!(bars.len(), 3);
        assert!(bars[0] < bars[1] && bars[1] < bars[2], "{bars:?}");
    }

    #[test]
    fn spark_with_fewer_values_than_the_width_fills_from_the_right() {
        let (bars, _) = spark(&[100.0, 100.0], 5, 0.0);
        assert_eq!(bars, "   ██");
    }

    #[test]
    fn spark_with_more_values_than_the_width_shows_the_newest() {
        let (bars, peak) = spark(&[100.0, 10.0, 20.0, 30.0], 3, 0.0);
        assert_eq!(bars.chars().count(), 3);
        // The 100 is out of sight: it neither shows nor sets the scale.
        assert_eq!(peak, 30.0);
        assert_eq!(bars.chars().last(), Some('█'));
    }

    #[test]
    fn the_tallest_bar_is_the_full_block_and_the_peak_is_the_largest_visible_value() {
        let (bars, peak) = spark(&[5.0, 40.0, 20.0], 3, 0.0);
        assert_eq!(peak, 40.0);
        assert_eq!(bars.chars().nth(1), Some('█'));
        assert_eq!(bars.chars().next(), Some('▁'));
    }

    #[test]
    fn a_floor_keeps_small_values_in_the_lower_half() {
        let (bars, peak) = spark(&[1.0, 2.0, 3.0, 4.0, 5.0], 5, 10.0);
        assert!(bars.chars().all(|c| "▁▂▃▄".contains(c)), "{bars}");
        // The peak is what was seen, not the floor.
        assert_eq!(peak, 5.0);
    }

    #[test]
    fn a_value_above_the_floor_is_scaled_to_itself() {
        let (bars, _) = spark(&[10.0, 20.0], 2, 10.0);
        assert_eq!(bars.chars().last(), Some('█'));
    }

    #[test]
    fn zeros_give_the_lowest_bar_and_nothing_gives_blanks() {
        assert_eq!(spark(&[0.0, 0.0], 2, 0.0), ("▁▁".to_string(), 0.0));
        assert_eq!(spark(&[0.0], 2, 10.0), (" ▁".to_string(), 0.0));
        assert_eq!(spark(&[], 3, 10.0), ("   ".to_string(), 0.0));
        assert_eq!(spark(&[1.0], 0, 0.0), (String::new(), 0.0));
    }

    #[test]
    fn spark_ignores_values_that_are_not_numbers_or_below_zero() {
        let (bars, peak) = spark(&[f64::NAN, -4.0, 8.0], 3, 0.0);
        assert_eq!(bars, "▁▁█");
        assert_eq!(peak, 8.0);
    }

    const MIB: f64 = 1024.0 * 1024.0;

    #[test]
    fn spark_from_low_draws_steady_values_as_the_lowest_bar() {
        let (bars, peak) = spark_from_low(&[500.0 * MIB; 4], 4, 16.0 * MIB);
        assert_eq!(bars, "▁▁▁▁");
        assert_eq!(peak, 500.0 * MIB);
    }

    #[test]
    fn spark_from_low_puts_a_rise_of_the_minimum_span_at_the_full_block() {
        let (bars, _) = spark_from_low(&[500.0 * MIB, 600.0 * MIB, 516.0 * MIB], 3, 16.0 * MIB);
        assert_eq!(bars, "▁█▂");
        let (bars, _) = spark_from_low(&[500.0 * MIB, 516.0 * MIB], 2, 16.0 * MIB);
        assert_eq!(bars, "▁█");
    }

    #[test]
    fn spark_from_low_keeps_a_small_rise_in_the_lower_half() {
        let (bars, _) = spark_from_low(&[500.0 * MIB, 504.0 * MIB], 2, 16.0 * MIB);
        assert_eq!(bars, "▁▂");
    }

    #[test]
    fn spark_from_low_ignores_values_scrolled_out_of_view() {
        let (bars, peak) = spark_from_low(&[100.0 * MIB, 500.0 * MIB, 516.0 * MIB], 2, 16.0 * MIB);
        assert_eq!(bars, "▁█");
        assert_eq!(peak, 516.0 * MIB);
    }

    /// A process directory `<root>/<pid>` with the given files, and its root.
    fn process(name: &str, pid: u32) -> (TempDir, PathBuf) {
        let root = TempDir::new(&format!("describe-{name}"));
        let dir = root.join(pid.to_string());
        fs::create_dir(&dir).unwrap();
        (root, dir)
    }

    /// Gives `<root>/<ppid>/comm` and a `status` with that `PPid` to the process at `dir`.
    fn with_parent(root: &Path, dir: &Path, ppid: u32, comm: &str) {
        fs::write(dir.join("status"), format!("Name:\tdemo\nPPid:\t{ppid}\nThreads:\t1\n")).unwrap();
        fs::create_dir(root.join(ppid.to_string())).unwrap();
        fs::write(root.join(ppid.to_string()).join("comm"), format!("{comm}\n")).unwrap();
    }

    #[test]
    fn the_command_line_arguments_are_joined_with_spaces() {
        let (_root, dir) = process("cmdline", 2000);
        fs::write(dir.join("cmdline"), b"/usr/bin/demo\0--flag\0value\0").unwrap();
        assert_eq!(describe(&dir, None).cmdline.as_deref(), Some("/usr/bin/demo --flag value"));
    }

    #[test]
    fn an_empty_or_missing_command_line_is_none() {
        let (_root, dir) = process("nocmd", 2000);
        assert_eq!(describe(&dir, None).cmdline, None);
        fs::write(dir.join("cmdline"), b"").unwrap();
        assert_eq!(describe(&dir, None).cmdline, None);
    }

    #[test]
    fn the_working_directory_is_read_from_the_link() {
        let (_root, dir) = process("cwd", 2000);
        std::os::unix::fs::symlink("/srv/demo/work", dir.join("cwd")).unwrap();
        assert_eq!(describe(&dir, None).cwd.as_deref(), Some("/srv/demo/work"));
    }

    #[test]
    fn a_home_directory_at_the_start_of_the_working_directory_becomes_a_tilde() {
        let (_root, dir) = process("home", 2000);
        let cwd = |target: &str, home: &str| {
            let _ = fs::remove_file(dir.join("cwd"));
            std::os::unix::fs::symlink(target, dir.join("cwd")).unwrap();
            describe(&dir, Some(Path::new(home))).cwd
        };
        assert_eq!(cwd("/home/user/src/demo", "/home/user").as_deref(), Some("~/src/demo"));
        assert_eq!(cwd("/home/user", "/home/user").as_deref(), Some("~"));
        assert_eq!(cwd("/home/user2/src", "/home/user").as_deref(), Some("/home/user2/src"));
        assert_eq!(cwd("/srv/src", "/home/user").as_deref(), Some("/srv/src"));
    }

    #[test]
    fn the_parents_name_comes_from_the_parents_comm() {
        let (root, dir) = process("parent", 2000);
        with_parent(&root, &dir, 1500, "demo-shell");
        assert_eq!(describe(&dir, None).parent.as_deref(), Some("demo-shell"));
    }

    #[test]
    fn each_field_is_none_when_its_file_is_missing_leaving_the_others() {
        let (root, dir) = process("partial", 2000);
        with_parent(&root, &dir, 1500, "demo-shell");
        fs::write(dir.join("cmdline"), b"demo\0").unwrap();
        // No cwd link.
        let info = describe(&dir, None);
        assert_eq!((info.cmdline.as_deref(), info.cwd.as_deref(), info.parent.as_deref()), (Some("demo"), None, Some("demo-shell")));

        // The parent's comm is gone; a status without PPid also gives no parent.
        fs::remove_file(root.join("1500").join("comm")).unwrap();
        std::os::unix::fs::symlink("/srv/demo", dir.join("cwd")).unwrap();
        let info = describe(&dir, None);
        assert_eq!((info.cmdline.as_deref(), info.cwd.as_deref(), info.parent), (Some("demo"), Some("/srv/demo"), None));
        fs::write(dir.join("status"), "Name:\tdemo\n").unwrap();
        assert_eq!(describe(&dir, None).parent, None);
    }

    #[test]
    fn a_sampler_on_a_second_process_gives_that_process_not_the_first() {
        let (_dir, first, uptime) = fake_proc("first-process");
        let second = first.with_file_name("2000");
        fs::create_dir(&second).unwrap();
        fs::write(second.join("stat"), "2000 (demo) S 1 1 1 0 -1 0 0 0 0 0 5 0 0 0 20 0 3 0 3413 0 0\n").unwrap();
        fs::write(second.join("status"), "Name:\tdemo\nVmRSS:\t2048 kB\nThreads:\t3\n").unwrap();
        let a = Sampler::new(&first, &uptime).sample(Instant::now());
        let b = Sampler::new(&second, &uptime).sample(Instant::now());
        assert_eq!((a.mem, a.threads), (Some(145048 * 1024), Some(17)));
        assert_eq!((b.mem, b.threads), (Some(2048 * 1024), Some(3)));
        // 9409.60 s since boot, started 34.13 s after it.
        assert_eq!(b.uptime, Some(Duration::from_secs_f64(9409.6 - 34.13)));
    }
}
