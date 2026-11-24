//! The controller owns the config and every Bluetooth operation. UIs and the CLI send it
//! `Command`s through a `Handle` and read the latest `Snapshot`; all base stations are handled
//! in parallel, each on its own worker thread with a deadline.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};

use super::ble::{Discovered, Radio};
use super::config::Config;
use super::station::{Mac, PowerState};

/// How long one station may take (connect + write + verify) before it counts as failed.
const STATION_DEADLINE: Duration = Duration::from_secs(20);
const SCAN_TIME: Duration = Duration::from_secs(5);
const VERIFY_DELAY: Duration = Duration::from_millis(500);
const WRITE_ATTEMPTS: usize = 3;
const CONNECT_ATTEMPTS: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    On,
    Off,
    Toggle,
}

pub type ConfigEdit = Box<dyn FnOnce(&mut Config) + Send>;

pub enum Command {
    /// Turn every enabled station on/off/toggle.
    PowerAll(Target),
    /// Turn one station on/off/toggle (even if it is disabled).
    Power(Mac, Target),
    /// Read the current state of every known station.
    Refresh,
    /// Look for base stations and add new ones to the config.
    Scan,
    /// SteamVR started: turn the known stations on and, if configured, scan for new ones.
    SessionStart,
    /// Change settings or the station list; the config is saved afterwards.
    EditConfig(ConfigEdit),
    /// Drop a station from the config.
    Forget(Mac),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StationStatus {
    pub mac: Mac,
    pub name: String,
    pub enabled: bool,
    pub state: PowerState,
    pub busy: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub stations: Vec<StationStatus>,
    /// Text describing the running operation, if any.
    pub activity: Option<String>,
    /// Result line of the last finished operation.
    pub message: Option<String>,
    /// True when the last operation had failures.
    pub message_is_error: bool,
}

impl Snapshot {
    pub fn powered_count(&self) -> (usize, usize) {
        let managed: Vec<_> = self.stations.iter().filter(|s| s.enabled).collect();
        (managed.iter().filter(|s| s.state.is_powered()).count(), managed.len())
    }
}

/// Outcome of one command, used for CLI output and exit codes.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub ok: Vec<(Mac, PowerState)>,
    pub failed: Vec<(Mac, String)>,
    pub discovered: Vec<Discovered>,
    pub new_stations: Vec<Mac>,
    pub error: Option<String>,
}

impl Report {
    pub fn success(&self) -> bool {
        self.failed.is_empty() && self.error.is_none()
    }
}

struct Request {
    cmd: Command,
    reply: Option<Sender<Report>>,
}

type Listener = Box<dyn Fn() + Send + Sync>;

struct Shared {
    snapshot: RwLock<Snapshot>,
    config: RwLock<Config>,
    listeners: Mutex<Vec<Listener>>,
}

/// Cheap to clone; every UI and the IPC server hold one.
#[derive(Clone)]
pub struct Handle {
    tx: Sender<Request>,
    shared: Arc<Shared>,
}

impl Handle {
    /// Queues a command without waiting.
    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(Request { cmd, reply: None });
    }

    /// Queues a command and blocks until it finishes.
    pub fn run(&self, cmd: Command) -> Report {
        let (tx, rx) = mpsc::channel();
        if self.tx.send(Request { cmd, reply: Some(tx) }).is_err() {
            return Report { error: Some("controller stopped".into()), ..Default::default() };
        }
        rx.recv().unwrap_or_else(|_| Report { error: Some("controller stopped".into()), ..Default::default() })
    }

    pub fn snapshot(&self) -> Snapshot {
        self.shared.snapshot.read().unwrap().clone()
    }

    pub fn config(&self) -> Config {
        self.shared.config.read().unwrap().clone()
    }

    /// Called (from the controller thread) whenever the snapshot changes.
    pub fn on_change(&self, f: impl Fn() + Send + Sync + 'static) {
        self.shared.listeners.lock().unwrap().push(Box::new(f));
    }
}

/// Starts the controller thread. `config_path = None` keeps the config in memory only (tests).
pub fn spawn(radio: Arc<dyn Radio>, config: Config, config_path: Option<PathBuf>) -> Handle {
    let (tx, rx) = mpsc::channel();
    let shared = Arc::new(Shared {
        snapshot: RwLock::new(Snapshot::default()),
        config: RwLock::new(config),
        listeners: Mutex::new(Vec::new()),
    });
    let mut worker = Worker { radio, shared: shared.clone(), config_path, states: HashMap::new() };
    worker.publish(|_| {});
    thread::Builder::new()
        .name("controller".into())
        .spawn(move || worker.run(rx))
        .expect("spawn controller thread");
    Handle { tx, shared }
}

struct Worker {
    radio: Arc<dyn Radio>,
    shared: Arc<Shared>,
    config_path: Option<PathBuf>,
    /// Last known state/error per station, kept across config edits.
    states: HashMap<Mac, (PowerState, Option<String>, bool)>,
}

impl Worker {
    fn run(&mut self, rx: Receiver<Request>) {
        while let Ok(req) = rx.recv() {
            let report = self.handle(req.cmd);
            let message = summarize(&report);
            let is_error = !report.success();
            self.publish(|s| {
                s.activity = None;
                if message.is_some() {
                    s.message = message;
                    s.message_is_error = is_error;
                }
            });
            if let Some(reply) = req.reply {
                let _ = reply.send(report);
            }
        }
    }

    fn handle(&mut self, cmd: Command) -> Report {
        match cmd {
            Command::PowerAll(target) => {
                let mut report = Report::default();
                if self.config().stations.is_empty() {
                    // Nothing known yet (first run): find the stations first.
                    report = self.scan();
                }
                let macs = self.config().enabled_macs();
                if macs.is_empty() {
                    report.error = Some("no base stations configured — run a scan".into());
                    return report;
                }
                let r = self.power(&macs, target);
                merge(report, r)
            }
            Command::Power(mac, target) => self.power(&[mac], target),
            Command::Refresh => self.refresh(),
            Command::Scan => self.scan(),
            Command::SessionStart => self.session_start(),
            Command::EditConfig(edit) => {
                let mut cfg = self.config();
                edit(&mut cfg);
                self.store_config(cfg)
            }
            Command::Forget(mac) => {
                let mut cfg = self.config();
                cfg.stations.retain(|s| s.mac() != Some(mac));
                self.states.remove(&mac);
                self.store_config(cfg)
            }
        }
    }

    fn config(&self) -> Config {
        self.shared.config.read().unwrap().clone()
    }

    fn store_config(&mut self, cfg: Config) -> Report {
        let mut report = Report::default();
        if let Some(path) = &self.config_path
            && let Err(e) = cfg.save_to(path) {
                tracing::error!("saving config: {e:#}");
                report.error = Some(format!("could not save settings: {e:#}"));
            }
        *self.shared.config.write().unwrap() = cfg;
        self.publish(|_| {});
        report
    }

    /// Rebuilds the station list from the config + known states, applies `f`, then notifies.
    fn publish(&mut self, f: impl FnOnce(&mut Snapshot)) {
        let cfg = self.shared.config.read().unwrap().clone();
        {
            let mut snap = self.shared.snapshot.write().unwrap();
            snap.stations = cfg
                .stations
                .iter()
                .filter_map(|s| {
                    let mac = s.mac()?;
                    let (state, error, busy) = self.states.get(&mac).cloned().unwrap_or((PowerState::Unknown, None, false));
                    Some(StationStatus { mac, name: s.name.clone(), enabled: s.enabled, state, busy, error })
                })
                .collect();
            f(&mut snap);
        }
        for l in self.shared.listeners.lock().unwrap().iter() {
            l();
        }
    }

    fn set_activity(&mut self, text: &str) {
        let text = text.to_string();
        self.publish(|s| s.activity = Some(text));
    }

    fn mark_busy(&mut self, macs: &[Mac]) {
        for m in macs {
            let e = self.states.entry(*m).or_insert((PowerState::Unknown, None, false));
            e.2 = true;
        }
    }

    fn record(&mut self, mac: Mac, result: &Result<PowerState, String>) {
        let prev = self.states.get(&mac).map(|s| s.0).unwrap_or(PowerState::Unknown);
        let entry = match result {
            Ok(state) => (*state, None, false),
            Err(e) => (prev, Some(e.clone()), false),
        };
        self.states.insert(mac, entry);
    }

    fn power(&mut self, macs: &[Mac], target: Target) -> Report {
        let off = self.config().off_mode.byte();
        let label = match target {
            Target::On => "Turning on",
            Target::Off => "Turning off",
            Target::Toggle => "Toggling",
        };
        self.set_activity(&format!("{label} {} base station(s)…", macs.len()));
        self.mark_busy(macs);
        self.publish(|_| {});
        tracing::info!("{label}: {}", macs.iter().map(Mac::to_string).collect::<Vec<_>>().join(", "));

        let radio = self.radio.clone();
        let results = run_parallel(macs, move |mac| set_power(radio.as_ref(), mac, target, off));
        self.collect(results)
    }

    fn refresh(&mut self) -> Report {
        let macs: Vec<Mac> = self.config().stations.iter().filter_map(|s| s.mac()).collect();
        if macs.is_empty() {
            return Report::default();
        }
        self.set_activity("Reading base station state…");
        self.mark_busy(&macs);
        self.publish(|_| {});
        let radio = self.radio.clone();
        let results = run_parallel(&macs, move |mac| {
            let mut link = connect(radio.as_ref(), mac)?;
            Ok(PowerState::from_byte(link.read_power()?))
        });
        self.collect(results)
    }

    fn collect(&mut self, results: Vec<(Mac, Result<PowerState, String>)>) -> Report {
        let mut report = Report::default();
        for (mac, r) in results {
            self.record(mac, &r);
            match r {
                Ok(s) => {
                    tracing::info!("{mac}: {s}");
                    report.ok.push((mac, s));
                }
                Err(e) => {
                    tracing::warn!("{mac}: {e}");
                    report.failed.push((mac, e));
                }
            }
        }
        self.publish(|_| {});
        report
    }

    fn scan(&mut self) -> Report {
        self.set_activity("Scanning for base stations…");
        let found = match self.radio.scan(SCAN_TIME) {
            Ok(f) => f,
            Err(e) => {
                tracing::error!("scan failed: {e:#}");
                return Report { error: Some(format!("scan failed: {e:#}")), ..Default::default() };
            }
        };
        self.add_found(found)
    }

    fn add_found(&mut self, found: Vec<Discovered>) -> Report {
        let mut report = Report { discovered: found.clone(), ..Default::default() };
        let mut cfg = self.config();
        for d in &found {
            if cfg.add_discovered(d.mac, &d.name) {
                tracing::info!("new base station {} ({}, {} dBm)", d.mac, d.name, d.rssi);
                report.new_stations.push(d.mac);
            }
        }
        if cfg != self.config() {
            let r = self.store_config(cfg);
            report.error = r.error;
        }
        report
    }

    fn session_start(&mut self) -> Report {
        let cfg = self.config();
        let known = cfg.enabled_macs();
        // Scan in the background while the known stations are being turned on.
        let scan = (cfg.scan_on_start || known.is_empty()).then(|| {
            let radio = self.radio.clone();
            thread::spawn(move || radio.scan(SCAN_TIME))
        });
        let mut report = if known.is_empty() { Report::default() } else { self.power(&known, Target::On) };
        if let Some(scan) = scan {
            self.set_activity("Scanning for new base stations…");
            match scan.join() {
                Ok(Ok(found)) => {
                    let added = self.add_found(found);
                    let enabled_new: Vec<Mac> = {
                        let cfg = self.config();
                        added.new_stations.iter().copied().filter(|m| cfg.find(*m).is_some_and(|s| s.enabled)).collect()
                    };
                    report = merge(report, added);
                    if !enabled_new.is_empty() {
                        let r = self.power(&enabled_new, Target::On);
                        report = merge(report, r);
                    }
                }
                Ok(Err(e)) => tracing::warn!("startup scan failed: {e:#}"),
                Err(_) => tracing::warn!("startup scan panicked"),
            }
        }
        report
    }
}

fn merge(mut a: Report, b: Report) -> Report {
    a.ok.extend(b.ok);
    a.failed.extend(b.failed);
    a.discovered.extend(b.discovered);
    a.new_stations.extend(b.new_stations);
    a.error = a.error.or(b.error);
    a
}

fn summarize(r: &Report) -> Option<String> {
    if let Some(e) = &r.error {
        return Some(e.clone());
    }
    let mut parts = Vec::new();
    if !r.ok.is_empty() || !r.failed.is_empty() {
        parts.push(format!("{} ok, {} failed", r.ok.len(), r.failed.len()));
    }
    if !r.new_stations.is_empty() {
        parts.push(format!("{} new base station(s) found", r.new_stations.len()));
    } else if !r.discovered.is_empty() {
        parts.push(format!("{} base station(s) in range", r.discovered.len()));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// Runs `op` for every MAC on its own thread and waits at most `STATION_DEADLINE` overall.
/// A thread that misses the deadline is abandoned (it finishes on its own later).
fn run_parallel(
    macs: &[Mac],
    op: impl Fn(Mac) -> Result<PowerState> + Send + Sync + 'static,
) -> Vec<(Mac, Result<PowerState, String>)> {
    let op = Arc::new(op);
    let (tx, rx) = mpsc::channel();
    for &mac in macs {
        let tx = tx.clone();
        let op = op.clone();
        thread::spawn(move || {
            let r = op(mac).map_err(|e| format!("{e:#}"));
            let _ = tx.send((mac, r));
        });
    }
    drop(tx);
    let deadline = Instant::now() + STATION_DEADLINE;
    let mut out: Vec<(Mac, Result<PowerState, String>)> = Vec::new();
    while out.len() < macs.len() {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(item) => out.push(item),
            Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    for &mac in macs {
        if !out.iter().any(|(m, _)| *m == mac) {
            out.push((mac, Err("timed out".into())));
        }
    }
    out.sort_by_key(|(m, _)| macs.iter().position(|x| x == m));
    out
}

fn connect(radio: &dyn Radio, mac: Mac) -> Result<Box<dyn super::ble::Link>> {
    let mut last = None;
    for attempt in 1..=CONNECT_ATTEMPTS {
        match radio.connect(mac) {
            Ok(l) => return Ok(l),
            Err(e) => {
                tracing::debug!("{mac}: connect attempt {attempt} failed: {e:#}");
                last = Some(e);
            }
        }
    }
    Err(last.unwrap())
}

/// Connects, writes the wanted value and reads it back, rewriting up to `WRITE_ATTEMPTS` times.
fn set_power(radio: &dyn Radio, mac: Mac, target: Target, off_byte: u8) -> Result<PowerState> {
    let mut link = connect(radio, mac)?;
    let turn_on = match target {
        Target::On => true,
        Target::Off => false,
        Target::Toggle => PowerState::from_byte(link.read_power()?).is_off(),
    };
    let byte = if turn_on { PowerState::BYTE_ON } else { off_byte };
    let mut state = PowerState::Unknown;
    for _ in 0..WRITE_ATTEMPTS {
        link.write_power(byte)?;
        thread::sleep(VERIFY_DELAY);
        state = PowerState::from_byte(link.read_power()?);
        let reached = if turn_on { state.is_powered() } else { state.is_off() };
        if reached {
            return Ok(state);
        }
    }
    bail!("still {state} after {WRITE_ATTEMPTS} attempts")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ble::Link;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Fake radio: each MAC holds a power byte; some MACs are unreachable or ignore writes.
    #[derive(Default)]
    struct FakeRadio {
        power: Arc<Mutex<HashMap<Mac, u8>>>,
        unreachable: Vec<Mac>,
        stubborn: Vec<Mac>,
        in_range: Vec<Discovered>,
        connects: AtomicUsize,
    }

    struct FakeLink {
        mac: Mac,
        power: Arc<Mutex<HashMap<Mac, u8>>>,
        stubborn: bool,
    }

    impl Link for FakeLink {
        fn read_power(&mut self) -> Result<u8> {
            Ok(*self.power.lock().unwrap().get(&self.mac).unwrap_or(&0))
        }
        fn write_power(&mut self, v: u8) -> Result<()> {
            if !self.stubborn {
                self.power.lock().unwrap().insert(self.mac, v);
            }
            Ok(())
        }
    }

    impl Radio for FakeRadio {
        fn connect(&self, mac: Mac) -> Result<Box<dyn Link>> {
            self.connects.fetch_add(1, Ordering::SeqCst);
            if self.unreachable.contains(&mac) {
                bail!("unreachable");
            }
            Ok(Box::new(FakeLink { mac, power: self.power.clone(), stubborn: self.stubborn.contains(&mac) }))
        }
        fn scan(&self, _: Duration) -> Result<Vec<Discovered>> {
            Ok(self.in_range.clone())
        }
    }

    fn cfg_with(macs: &[u64]) -> Config {
        let mut c = Config::default();
        for m in macs {
            c.add_discovered(Mac(*m), &format!("LHB-{m}"));
        }
        c
    }

    #[test]
    fn all_on_then_off_uses_off_mode() {
        let radio = Arc::new(FakeRadio::default());
        let h = spawn(radio.clone(), cfg_with(&[1, 2, 3]), None);
        let r = h.run(Command::PowerAll(Target::On));
        assert!(r.success());
        assert_eq!(r.ok.len(), 3);
        assert!(h.snapshot().stations.iter().all(|s| s.state == PowerState::On && !s.busy));

        h.run(Command::EditConfig(Box::new(|c| c.off_mode = crate::engine::config::OffMode::Standby)));
        let r = h.run(Command::PowerAll(Target::Off));
        assert!(r.success());
        assert!(r.ok.iter().all(|(_, s)| *s == PowerState::Standby));
        assert_eq!(h.snapshot().powered_count(), (0, 3));
    }

    #[test]
    fn one_failure_does_not_block_others() {
        let radio = Arc::new(FakeRadio { unreachable: vec![Mac(2)], stubborn: vec![Mac(3)], ..Default::default() });
        let h = spawn(radio, cfg_with(&[1, 2, 3]), None);
        let r = h.run(Command::PowerAll(Target::On));
        assert!(!r.success());
        assert_eq!(r.ok, vec![(Mac(1), PowerState::On)]);
        assert_eq!(r.failed.len(), 2);
        let snap = h.snapshot();
        assert!(snap.stations.iter().find(|s| s.mac == Mac(2)).unwrap().error.is_some());
    }

    #[test]
    fn disabled_stations_are_skipped() {
        let radio = Arc::new(FakeRadio::default());
        let mut cfg = cfg_with(&[1, 2]);
        cfg.stations[1].enabled = false;
        let h = spawn(radio.clone(), cfg, None);
        let r = h.run(Command::PowerAll(Target::On));
        assert_eq!(r.ok.len(), 1);
        assert!(!radio.power.lock().unwrap().contains_key(&Mac(2)));
    }

    #[test]
    fn toggle_flips_each_station() {
        let radio = Arc::new(FakeRadio::default());
        radio.power.lock().unwrap().insert(Mac(1), 1);
        let h = spawn(radio.clone(), cfg_with(&[1, 2]), None);
        h.run(Command::PowerAll(Target::Toggle));
        let p = radio.power.lock().unwrap();
        assert_eq!(p[&Mac(1)], 0);
        assert_eq!(p[&Mac(2)], 1);
    }

    #[test]
    fn first_run_scans_and_session_start_adds_new() {
        let d = |m: u64| Discovered { mac: Mac(m), name: format!("LHB-{m}"), rssi: -50 };
        let radio = Arc::new(FakeRadio { in_range: vec![d(1), d(2)], ..Default::default() });
        let h = spawn(radio.clone(), Config::default(), None);
        // No stations known: PowerAll scans first.
        let r = h.run(Command::PowerAll(Target::On));
        assert_eq!(r.new_stations.len(), 2);
        assert_eq!(r.ok.len(), 2);

        // A third station appears: SessionStart turns on known ones and the new one.
        let radio2 = Arc::new(FakeRadio { in_range: vec![d(1), d(2), d(3)], ..Default::default() });
        let h2 = spawn(radio2.clone(), h.config(), None);
        let r = h2.run(Command::SessionStart);
        assert_eq!(r.new_stations, vec![Mac(3)]);
        assert_eq!(r.ok.len(), 3);
        assert_eq!(h2.config().stations.len(), 3);
    }

    #[test]
    fn forget_and_empty_power() {
        let radio = Arc::new(FakeRadio::default());
        let h = spawn(radio, cfg_with(&[1]), None);
        h.run(Command::Forget(Mac(1)));
        assert!(h.config().stations.is_empty());
        let r = h.run(Command::PowerAll(Target::On));
        assert!(!r.success(), "no stations anywhere must be reported as an error");
    }
}
