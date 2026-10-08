//! What each application with windows costs: its process tree from `/proc`, and the figures of
//! the processes in it added up.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::hypr::Window;
use crate::usage::{Sample, Sampler, parse_stat};

/// Each process's parent, by process ID.
pub type Parents = HashMap<u32, u32>;

/// An application: the process that owns windows, and it with its descendants.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub owner: u32,
    /// Ascending, and including `owner`.
    pub pids: Vec<u32>,
}

/// Every process under `proc_root` (`/proc`) and its parent. Entries that are not processes,
/// and processes that vanish or cannot be read, are left out.
pub fn tree(proc_root: &Path) -> Parents {
    let Ok(entries) = std::fs::read_dir(proc_root) else { return Parents::new() };
    entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
            let stat = parse_stat(&std::fs::read_to_string(entry.path().join("stat")).ok()?)?;
            Some((pid, stat.ppid))
        })
        .collect()
}

/// One group for each of `owners`: the owner and its descendants, except that a descendant
/// that is itself an owner starts its own group and takes its descendants with it. A process
/// whose line of parents breaks off, loops, or reaches process 0 or 1 without meeting an
/// owner belongs to no group. Ordered by owner.
pub fn groups(parents: &Parents, owners: &HashSet<u32>) -> Vec<Group> {
    let mut groups: Vec<Group> = owners.iter().map(|&owner| Group { owner, pids: vec![owner] }).collect();
    groups.sort_by_key(|g| g.owner);
    for &pid in parents.keys() {
        match nearest_owner(pid, parents, owners) {
            Some(owner) if owner != pid => {
                if let Ok(i) = groups.binary_search_by_key(&owner, |g| g.owner) {
                    groups[i].pids.push(pid);
                }
            }
            _ => {}
        }
    }
    for group in &mut groups {
        group.pids.sort_unstable();
    }
    groups
}

/// The first owner on the way up from `pid`, itself included. The way up ends at a process
/// with no entry, at process 0 or 1, and, in a cycle, after as many steps as there are processes.
fn nearest_owner(pid: u32, parents: &Parents, owners: &HashSet<u32>) -> Option<u32> {
    let mut at = pid;
    for _ in 0..=parents.len() {
        if owners.contains(&at) {
            return Some(at);
        }
        at = *parents.get(&at).filter(|&&parent| parent > 1)?;
    }
    None
}

/// `total` and `figure` added, where a missing one counts as nothing but two missing stay missing.
fn add<T: std::ops::Add<Output = T>>(total: Option<T>, figure: Option<T>) -> Option<T> {
    match (total, figure) {
        (Some(a), Some(b)) => Some(a + b),
        (a, b) => a.or(b),
    }
}

/// One row of the process table.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// The commonest class among the process's windows; `Hyprland` for the first row.
    pub name: String,
    /// Windows in the snapshot that belong to the application; 0 for the Hyprland row.
    pub windows: usize,
    /// Processes in the application's tree that are running.
    pub procs: usize,
    /// Percent of one core; `None` until every figure has a second reading to compare with.
    pub cpu: Option<f64>,
    /// Resident bytes; `None` when no process in the row could be read.
    pub mem: Option<u64>,
    /// Percent, at most 100; `None` when no process in the row has figures.
    pub gpu: Option<f64>,
}

/// Reads what the applications with windows cost, one reading each time `refresh` is called.
pub struct Costs {
    proc_root: PathBuf,
    /// One sampler for each process in a group, made when it first appears and dropped when it goes.
    samplers: HashMap<u32, Sampler>,
}

impl Costs {
    /// `proc_root` is `/proc`, or a directory laid out like it.
    pub fn new(proc_root: impl Into<PathBuf>) -> Self {
        Costs { proc_root: proc_root.into(), samplers: HashMap::new() }
    }

    /// The table: the Hyprland row first, from `hyprland` (the reading the loop already took
    /// of process `hyprland_pid`), then one row for each process that owns windows in `windows`,
    /// with its descendants, by name ignoring case, then by the owner's process ID.
    pub fn refresh(&mut self, now: Instant, windows: &[Window], hyprland_pid: u32, hyprland: &Sample) -> Vec<Row> {
        // Class counts by process that owns windows; the compositor is its own row.
        let mut classes: HashMap<u32, BTreeMap<&str, usize>> = HashMap::new();
        for window in windows {
            match u32::try_from(window.pid) {
                Ok(pid) if pid > 0 && pid != hyprland_pid => {
                    *classes.entry(pid).or_default().entry(window.class.as_str()).or_default() += 1;
                }
                _ => {}
            }
        }
        let parents = tree(&self.proc_root);
        let owners: HashSet<u32> = classes.keys().copied().collect();

        let mut rows: Vec<(u32, Row)> = Vec::new();
        let mut seen = HashSet::new();
        for group in groups(&parents, &owners) {
            let mut row = Row { name: String::new(), windows: 0, procs: 0, cpu: None, mem: None, gpu: None };
            for pid in group.pids.iter().filter(|pid| parents.contains_key(pid)) {
                seen.insert(*pid);
                let sampler = self.samplers.entry(*pid).or_insert_with(|| {
                    Sampler::new(self.proc_root.join(pid.to_string()), self.proc_root.join("uptime"))
                });
                let sample = sampler.sample(now);
                row.procs += 1;
                row.cpu = add(row.cpu, sample.cpu);
                row.mem = add(row.mem, sample.mem);
                row.gpu = add(row.gpu, sample.gpu.and_then(|g| g.percent));
            }
            row.gpu = row.gpu.map(|gpu| gpu.min(100.0));
            // Counts are in class order, so the first of equal counts sorts first.
            let counts = &classes[&group.owner];
            row.windows = counts.values().sum();
            row.name = counts.iter().rev().max_by_key(|(_, n)| **n).map(|(c, _)| c.to_string()).unwrap_or_default();
            rows.push((group.owner, row));
        }
        self.samplers.retain(|pid, _| seen.contains(pid));

        rows.sort_by_cached_key(|(owner, row)| (row.name.to_lowercase(), *owner));
        let first = Row {
            name: "Hyprland".into(),
            windows: 0,
            procs: 1,
            cpu: hyprland.cpu,
            mem: hyprland.mem,
            gpu: hyprland.gpu.as_ref().and_then(|g| g.percent),
        };
        std::iter::once(first).chain(rows.into_iter().map(|(_, row)| row)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TempDir;
    use std::fs;

    fn parents(pairs: &[(u32, u32)]) -> Parents {
        pairs.iter().copied().collect()
    }

    fn owners(pids: &[u32]) -> HashSet<u32> {
        pids.iter().copied().collect()
    }

    fn pids_of(groups: &[Group], owner: u32) -> Vec<u32> {
        groups.iter().find(|g| g.owner == owner).map(|g| g.pids.clone()).unwrap_or_default()
    }

    #[test]
    fn descendants_at_any_depth_are_in_the_owners_group() {
        let p = parents(&[(10, 1), (11, 10), (12, 11), (13, 12), (14, 10)]);
        let g = groups(&p, &owners(&[10]));
        assert_eq!(g, [Group { owner: 10, pids: vec![10, 11, 12, 13, 14] }]);
    }

    #[test]
    fn a_descendant_that_owns_windows_starts_its_own_group_with_its_descendants() {
        let p = parents(&[(10, 1), (11, 10), (12, 11), (13, 12), (14, 10)]);
        let g = groups(&p, &owners(&[10, 12]));
        assert_eq!(pids_of(&g, 10), [10, 11, 14]);
        assert_eq!(pids_of(&g, 12), [12, 13]);
    }

    #[test]
    fn sibling_owners_share_nothing() {
        let p = parents(&[(5, 1), (10, 5), (11, 10), (20, 5), (21, 20)]);
        let g = groups(&p, &owners(&[10, 20]));
        assert_eq!(pids_of(&g, 10), [10, 11]);
        assert_eq!(pids_of(&g, 20), [20, 21]);
    }

    #[test]
    fn a_process_whose_parent_is_missing_stands_alone() {
        let p = parents(&[(10, 1), (11, 10), (30, 99), (31, 30)]);
        let g = groups(&p, &owners(&[10, 30]));
        assert_eq!(pids_of(&g, 10), [10, 11]);
        // The owner whose parent is gone still has its own group and its children.
        assert_eq!(pids_of(&g, 30), [30, 31]);
        // A child of a missing, non-owning parent belongs to nobody.
        let g = groups(&parents(&[(10, 1), (40, 99)]), &owners(&[10]));
        assert_eq!(g, [Group { owner: 10, pids: vec![10] }]);
    }

    #[test]
    fn a_cycle_or_a_self_parent_does_not_loop() {
        let p = parents(&[(10, 10), (20, 21), (21, 22), (22, 20), (23, 20)]);
        let g = groups(&p, &owners(&[10, 20]));
        assert_eq!(pids_of(&g, 10), [10]);
        assert_eq!(pids_of(&g, 20), [20, 21, 22, 23]);
        // A cycle with no owner in it.
        assert_eq!(groups(&parents(&[(1000, 1001), (1001, 1000)]), &owners(&[5])), [Group { owner: 5, pids: vec![5] }]);
    }

    #[test]
    fn process_zero_and_one_are_never_a_parent_in_a_group() {
        let p = parents(&[(1, 0), (10, 1), (11, 1), (12, 0)]);
        // Even an owner at 1 does not gather the processes that are everyone's orphans.
        let g = groups(&p, &owners(&[1, 10]));
        assert_eq!(pids_of(&g, 1), [1]);
        assert_eq!(pids_of(&g, 10), [10]);
        assert_eq!(groups(&p, &owners(&[0])), [Group { owner: 0, pids: vec![0] }]);
    }

    #[test]
    fn an_owner_that_is_not_running_still_has_a_group() {
        assert_eq!(groups(&Parents::new(), &owners(&[7])), [Group { owner: 7, pids: vec![7] }]);
    }

    fn stat_line(pid: u32, comm: &str, ppid: u32) -> String {
        stat_with_ticks(pid, comm, ppid, 7)
    }

    /// A `stat` whose `utime + stime` is `ticks`.
    fn stat_with_ticks(pid: u32, comm: &str, ppid: u32, ticks: u64) -> String {
        format!("{pid} ({comm}) S {ppid} {pid} {pid} 0 -1 0 0 0 0 0 {ticks} 0 0 0 20 0 1 0 99 0 0\n")
    }

    #[test]
    fn tree_reads_each_processs_parent_and_skips_what_is_not_a_process() {
        let root = TempDir::new("processes-tree");
        for (pid, comm, ppid) in [(100, "alpha", 1), (101, "my ) odd (name", 100)] {
            fs::create_dir(root.join(pid.to_string())).unwrap();
            fs::write(root.join(pid.to_string()).join("stat"), stat_line(pid, comm, ppid)).unwrap();
        }
        // Not a process: other names, a file named like one, a directory with no stat, a broken stat.
        fs::create_dir(root.join("sys")).unwrap();
        fs::write(root.join("sys").join("stat"), stat_line(5, "x", 1)).unwrap();
        fs::write(root.join("102"), "file").unwrap();
        fs::create_dir(root.join("103")).unwrap();
        fs::create_dir(root.join("104")).unwrap();
        fs::write(root.join("104").join("stat"), "104 (x) S").unwrap();
        fs::write(root.join("uptime"), "1.0 1.0").unwrap();
        assert_eq!(tree(&root), parents(&[(100, 1), (101, 100)]));
        assert_eq!(tree(&root.join("missing")), Parents::new());
    }

    // -- Costs --

    use crate::hypr::Window;
    use crate::usage::Gpu;
    use std::time::Duration;

    fn window(pid: i64, class: &str) -> Window {
        Window {
            address: format!("0x{pid}{class}"), mapped: true, hidden: false, floating: false,
            fullscreen: false, pinned: false, xwayland: false, pid, class: class.into(),
            title: "t".into(), at: (0, 0), size: (10, 20), workspace_id: 1, workspace_name: "1".into(),
            monitor_id: 1, fullscreen_state: 0, fullscreen_client: 0, grouped: vec![], tags: vec![],
            focus_history_id: None, inhibiting_idle: false, initial_class: String::new(),
            content_type: String::new(),
        }
    }

    /// A process directory with these figures; `rss_kb` of 0 writes no memory line.
    fn make(root: &Path, pid: u32, ppid: u32, ticks: u64, rss_kb: u64) {
        let dir = root.join(pid.to_string());
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("stat"), stat_with_ticks(pid, "app", ppid, ticks)).unwrap();
        let status = if rss_kb > 0 { format!("Name:\tapp\nVmRSS:\t{rss_kb} kB\nThreads:\t1\n") } else { String::new() };
        fs::write(dir.join("status"), status).unwrap();
    }

    fn ticks(root: &Path, pid: u32, ticks: u64) {
        let ppid = tree(root)[&pid];
        fs::write(root.join(pid.to_string()).join("stat"), stat_with_ticks(pid, "app", ppid, ticks)).unwrap();
    }

    /// Gives the process a DRM client whose time-form engine has been busy for `busy_ms`.
    fn gpu_busy(root: &Path, pid: u32, busy_ms: u64) {
        let dir = root.join(pid.to_string()).join("fdinfo");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("5"), format!("drm-client-id:\t3\ndrm-engine-gfx:\t{} ns\n", busy_ms * 1_000_000)).unwrap();
    }

    const HYPR: u32 = 900;

    fn hyprland_sample() -> Sample {
        Sample { cpu: Some(3.5), mem: Some(104 << 20), gpu: Some(Gpu { percent: Some(4.0), mem: 0 }), ..Sample::default() }
    }

    /// Two readings one second apart, the second after `change` has been applied.
    fn two_readings(root: &Path, windows: &[Window], change: impl FnOnce()) -> (Vec<Row>, Vec<Row>) {
        let mut costs = Costs::new(root);
        let t0 = Instant::now();
        let first = costs.refresh(t0, windows, HYPR, &hyprland_sample());
        change();
        let second = costs.refresh(t0 + Duration::from_secs(1), windows, HYPR, &hyprland_sample());
        (first, second)
    }

    fn row<'a>(rows: &'a [Row], name: &str) -> &'a Row {
        rows.iter().find(|r| r.name == name).unwrap()
    }

    #[test]
    fn a_groups_memory_is_the_sum_of_its_processes() {
        let root = TempDir::new("costs-mem");
        make(&root, 100, 1, 0, 2000);
        make(&root, 101, 100, 0, 500);
        make(&root, 102, 101, 0, 40);
        make(&root, 200, 1, 0, 9);
        let windows = [window(100, "alpha"), window(200, "beta")];
        let rows = Costs::new(&*root).refresh(Instant::now(), &windows, HYPR, &hyprland_sample());
        assert_eq!(row(&rows, "alpha").mem, Some(2540 * 1024));
        assert_eq!(row(&rows, "alpha").procs, 3);
        assert_eq!(row(&rows, "beta").mem, Some(9 * 1024));
    }

    #[test]
    fn cpu_is_the_sum_of_the_gains_over_the_elapsed_time_and_missing_at_first() {
        let root = TempDir::new("costs-cpu");
        make(&root, 100, 1, 1000, 1);
        make(&root, 101, 100, 5000, 1);
        let windows = [window(100, "alpha")];
        let (first, second) = two_readings(&root, &windows, || {
            ticks(&root, 100, 1030);
            ticks(&root, 101, 5020);
        });
        assert_eq!(row(&first, "alpha").cpu, None);
        // 30 + 20 ticks of 1/100 s in one second: half a core.
        assert_eq!(row(&second, "alpha").cpu, Some(50.0));
    }

    #[test]
    fn a_process_that_appears_adds_memory_at_once_and_cpu_from_its_second_reading() {
        let root = TempDir::new("costs-appear");
        make(&root, 100, 1, 0, 100);
        let windows = [window(100, "alpha")];
        let mut costs = Costs::new(&*root);
        let t0 = Instant::now();
        costs.refresh(t0, &windows, HYPR, &hyprland_sample());
        ticks(&root, 100, 10);
        make(&root, 101, 100, 5000, 50);
        let second = costs.refresh(t0 + Duration::from_secs(1), &windows, HYPR, &hyprland_sample());
        // The newcomer's 5000 ticks since boot are not a gain.
        assert_eq!(row(&second, "alpha").mem, Some(150 * 1024));
        assert_eq!(row(&second, "alpha").cpu, Some(10.0));
        assert_eq!(row(&second, "alpha").procs, 2);
        ticks(&root, 100, 20);
        ticks(&root, 101, 5025);
        let third = costs.refresh(t0 + Duration::from_secs(2), &windows, HYPR, &hyprland_sample());
        assert_eq!(row(&third, "alpha").cpu, Some(35.0));
    }

    #[test]
    fn a_process_that_disappears_is_dropped_and_never_makes_the_cpu_negative() {
        let root = TempDir::new("costs-vanish");
        make(&root, 100, 1, 1000, 100);
        make(&root, 101, 100, 9000, 50);
        let windows = [window(100, "alpha")];
        let mut costs = Costs::new(&*root);
        let t0 = Instant::now();
        costs.refresh(t0, &windows, HYPR, &hyprland_sample());
        fs::remove_dir_all(root.join("101")).unwrap();
        ticks(&root, 100, 1010);
        let second = costs.refresh(t0 + Duration::from_secs(1), &windows, HYPR, &hyprland_sample());
        assert_eq!(row(&second, "alpha").cpu, Some(10.0));
        assert_eq!(row(&second, "alpha").mem, Some(100 * 1024));
        assert_eq!(row(&second, "alpha").procs, 1);
        // A new process taking the number starts afresh, whatever the old one had used.
        make(&root, 101, 100, 3, 50);
        let third = costs.refresh(t0 + Duration::from_secs(2), &windows, HYPR, &hyprland_sample());
        assert_eq!(row(&third, "alpha").cpu, Some(0.0));
    }

    #[test]
    fn gpu_is_summed_over_the_processes_and_capped_at_a_hundred() {
        let root = TempDir::new("costs-gpu");
        make(&root, 100, 1, 0, 1);
        make(&root, 101, 100, 0, 1);
        make(&root, 200, 1, 0, 1);
        make(&root, 300, 1, 0, 1);
        for pid in [100, 101, 200] {
            gpu_busy(&root, pid, 0);
        }
        let windows = [window(100, "alpha"), window(200, "beta"), window(300, "gamma")];
        let (first, second) = two_readings(&root, &windows, || {
            gpu_busy(&root, 100, 600);
            gpu_busy(&root, 101, 700);
            gpu_busy(&root, 200, 300);
        });
        assert_eq!(row(&first, "alpha").gpu, None);
        assert_eq!(row(&second, "alpha").gpu, Some(100.0));
        assert_eq!(row(&second, "beta").gpu, Some(30.0));
        // No process of it has figures.
        assert_eq!(row(&second, "gamma").gpu, None);
    }

    #[test]
    fn an_unreadable_process_leaves_the_groups_other_figures() {
        let root = TempDir::new("costs-unreadable");
        make(&root, 100, 1, 0, 100);
        make(&root, 101, 100, 0, 0);
        fs::remove_file(root.join("101").join("status")).unwrap();
        let windows = [window(100, "alpha")];
        let (_, second) = two_readings(&root, &windows, || {
            ticks(&root, 100, 25);
            ticks(&root, 101, 25);
        });
        let alpha = row(&second, "alpha");
        assert_eq!((alpha.mem, alpha.cpu, alpha.procs), (Some(100 * 1024), Some(50.0), 2));
    }

    fn names(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|r| r.name.as_str()).collect()
    }

    #[test]
    fn rows_are_hyprland_first_then_by_name_whatever_their_figures() {
        let root = TempDir::new("costs-order");
        for (pid, ticks, rss) in [(100, 0, 10), (200, 0, 30), (300, 0, 20), (400, 0, 5)] {
            make(&root, pid, 1, ticks, rss);
        }
        let windows = [window(100, "low"), window(200, "tie-big"), window(300, "tie-small"), window(400, "busy")];
        let (_, second) = two_readings(&root, &windows, || {
            ticks(&root, 100, 1);
            ticks(&root, 200, 10);
            ticks(&root, 300, 10);
            ticks(&root, 400, 50);
        });
        assert_eq!(names(&second), ["Hyprland", "busy", "low", "tie-big", "tie-small"]);
        let hyprland = &second[0];
        assert_eq!((hyprland.cpu, hyprland.mem, hyprland.gpu), (Some(3.5), Some(104 << 20), Some(4.0)));
        assert_eq!((hyprland.windows, hyprland.procs), (0, 1));
    }

    #[test]
    fn names_are_ordered_ignoring_case() {
        let root = TempDir::new("costs-case");
        make(&root, 100, 1, 0, 1);
        make(&root, 200, 1, 0, 1);
        make(&root, 300, 1, 0, 1);
        let windows = [window(100, "Zed"), window(200, "alacritty"), window(300, "Beta")];
        let rows = Costs::new(&*root).refresh(Instant::now(), &windows, HYPR, &hyprland_sample());
        assert_eq!(names(&rows), ["Hyprland", "alacritty", "Beta", "Zed"]);
    }

    #[test]
    fn rows_with_the_same_name_keep_the_order_of_their_process_ids_as_figures_swap() {
        let root = TempDir::new("costs-same");
        make(&root, 300, 1, 0, 50);
        make(&root, 200, 1, 0, 10);
        let windows = [window(300, "term"), window(200, "term")];
        let (first, second) = two_readings(&root, &windows, || {
            ticks(&root, 300, 1);
            ticks(&root, 200, 90);
        });
        assert_eq!(names(&first), ["Hyprland", "term", "term"]);
        assert_eq!(first[1].mem, Some(10 * 1024));
        assert_eq!(second[1].mem, Some(10 * 1024));
        assert_eq!((second[1].cpu, second[2].cpu), (Some(90.0), Some(1.0)));
        // The figures swap again; the order does not.
        let mut costs = Costs::new(&*root);
        let t0 = Instant::now();
        costs.refresh(t0, &windows, HYPR, &hyprland_sample());
        ticks(&root, 300, 200);
        let third = costs.refresh(t0 + Duration::from_secs(1), &windows, HYPR, &hyprland_sample());
        assert_eq!((third[1].mem, third[2].mem), (Some(10 * 1024), Some(50 * 1024)));
    }

    #[test]
    fn the_name_is_the_commonest_class_and_ties_go_to_the_one_that_sorts_first() {
        let root = TempDir::new("costs-name");
        make(&root, 100, 1, 0, 1);
        make(&root, 200, 1, 0, 1);
        let windows = [
            window(100, "dialog"), window(100, "editor"), window(100, "editor"),
            window(200, "zed"), window(200, "alpha"),
        ];
        let rows = Costs::new(&*root).refresh(Instant::now(), &windows, HYPR, &hyprland_sample());
        let editor = row(&rows, "editor");
        assert_eq!(editor.windows, 3);
        assert_eq!(row(&rows, "alpha").windows, 2);
        assert!(rows.iter().all(|r| r.name != "zed" && r.name != "dialog"));
    }

    #[test]
    fn a_window_with_no_process_belongs_to_no_row() {
        let root = TempDir::new("costs-nopid");
        make(&root, 100, 1, 0, 1);
        let windows = [window(0, "ghost"), window(-1, "ghost"), window(100, "alpha")];
        let rows = Costs::new(&*root).refresh(Instant::now(), &windows, HYPR, &hyprland_sample());
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["Hyprland", "alpha"]);
    }

    #[test]
    fn the_hyprland_process_is_one_row_of_its_own() {
        let root = TempDir::new("costs-hypr");
        make(&root, HYPR, 1, 0, 1);
        make(&root, 100, HYPR, 0, 1);
        let windows = [window(100, "alpha")];
        let rows = Costs::new(&*root).refresh(Instant::now(), &windows, HYPR, &hyprland_sample());
        assert_eq!(rows.iter().filter(|r| r.name == "Hyprland").count(), 1);
        assert_eq!(rows[0].procs, 1);
        assert_eq!(row(&rows, "alpha").procs, 1);
    }
}
