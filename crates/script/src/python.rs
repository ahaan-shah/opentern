//! The Python host: the SDK is embedded in the app and unpacked to the
//! workspace; each script gets a `python -u _runner.py <script>` process
//! speaking JSON lines.

use crossbeam_channel::{Receiver, Sender, unbounded};
use opentern_core::Bar;
use serde::Deserialize;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

const SDK: [(&str, &str); 3] = [
    ("__init__.py", include_str!("../../../python/opentern/__init__.py")),
    ("ta.py", include_str!("../../../python/opentern/ta.py")),
    ("_runner.py", include_str!("../../../python/opentern/_runner.py")),
];

/// Writes the SDK to `<dir>/opentern/` (only files that changed) and
/// returns the runner's path.
pub fn install_sdk(dir: &Path) -> std::io::Result<PathBuf> {
    let pkg = dir.join("opentern");
    std::fs::create_dir_all(&pkg)?;
    for (name, src) in SDK {
        let p = pkg.join(name);
        if std::fs::read_to_string(&p).ok().as_deref() != Some(src) {
            std::fs::write(&p, src)?;
        }
    }
    Ok(pkg.join("_runner.py"))
}

/// The Python to use: `OPENTERN_PYTHON`, else `python3`, else `python`.
pub fn interpreter() -> Option<String> {
    if let Ok(p) = std::env::var("OPENTERN_PYTHON") {
        return Some(p);
    }
    ["python3", "python"].into_iter().find_map(|c| {
        Command::new(c)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .ok()
            .filter(|s| s.success())
            .map(|_| c.to_string())
    })
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct InputDesc {
    pub id: String,
    pub title: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub default: serde_json::Value,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub step: Option<f64>,
    #[serde(default)]
    pub options: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Meta {
    pub name: String,
    #[serde(default)]
    pub short: String,
    #[serde(default)]
    pub overlay: bool,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub precision: Option<u8>,
    #[serde(default)]
    pub inputs: Vec<InputDesc>,
    /// Strategy properties (`strategy(...)` arguments).
    #[serde(default)]
    pub props: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct PlotOut {
    pub title: String,
    pub values: Vec<Option<f64>>,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub width: u8,
    #[serde(default)]
    pub style: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct HLineOut {
    pub title: String,
    pub value: f64,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub style: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Output {
    #[serde(default)]
    pub plots: Vec<PlotOut>,
    #[serde(default)]
    pub hlines: Vec<HLineOut>,
    /// Strategy orders, one per bar where a `when` was true.
    #[serde(default)]
    pub intents: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct Reply {
    ok: bool,
    #[serde(default)]
    meta: Option<Meta>,
    #[serde(default)]
    out: Option<Output>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    log: String,
}

struct Process {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Process {
    fn spawn(python: &str, runner: &Path, script: &Path) -> std::io::Result<Process> {
        let mut child = Command::new(python)
            .arg("-u")
            .arg(runner)
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| std::io::Error::other("no stdin"))?;
        let stdout = BufReader::new(child.stdout.take().ok_or_else(|| std::io::Error::other("no stdout"))?);
        Ok(Process { child, stdin, stdout })
    }

    fn ask(&mut self, req: &serde_json::Value) -> std::io::Result<Reply> {
        let mut line = req.to_string();
        line.push('\n');
        self.stdin.write_all(line.as_bytes())?;
        self.stdin.flush()?;
        let mut answer = String::new();
        if self.stdout.read_line(&mut answer)? == 0 {
            return Err(std::io::Error::other("the Python process ended"));
        }
        serde_json::from_str(&answer).map_err(std::io::Error::other)
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// What a runner reports back.
#[derive(Clone, Debug)]
pub enum Event {
    /// The script's declarations (after load and every reload).
    Described(Meta),
    /// Results for the run numbered `seq`, computed on `bars` bars.
    Output {
        seq: u64,
        meta: Meta,
        out: Output,
        bars: usize,
    },
    /// Something went wrong; the message is for people.
    Error(String),
    /// What the script printed.
    Log(String),
}

struct Job {
    seq: u64,
    bars: Arc<Vec<Bar>>,
    inputs: serde_json::Value,
}

/// A script's worker: send it bars with [`Runner::run`]; read results with
/// [`Runner::poll`]. Only the newest pending request is run.
pub struct Runner {
    pub path: PathBuf,
    jobs: Sender<Job>,
    events: Receiver<Event>,
    seq: u64,
}

impl Runner {
    /// Starts the worker. `sdk_dir` is where the SDK is unpacked; `wake`
    /// asks the UI to look for results.
    pub fn start(script: PathBuf, sdk_dir: PathBuf, wake: Arc<dyn Fn() + Send + Sync>) -> Runner {
        let (jobs, job_rx) = unbounded();
        let (ev_tx, events) = unbounded();
        let path = script.clone();
        let _ = std::thread::Builder::new()
            .name("python".into())
            .spawn(move || worker(script, sdk_dir, job_rx, ev_tx, wake));
        Runner {
            path,
            jobs,
            events,
            seq: 0,
        }
    }

    /// Asks for a run on these bars; returns its number.
    pub fn run(&mut self, bars: Arc<Vec<Bar>>, inputs: serde_json::Value) -> u64 {
        self.seq += 1;
        let _ = self.jobs.send(Job {
            seq: self.seq,
            bars,
            inputs,
        });
        self.seq
    }

    pub fn poll(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }
}

fn bars_json(bars: &[Bar]) -> serde_json::Value {
    serde_json::json!({
        "time": bars.iter().map(|b| b.time).collect::<Vec<_>>(),
        "open": bars.iter().map(|b| b.open).collect::<Vec<_>>(),
        "high": bars.iter().map(|b| b.high).collect::<Vec<_>>(),
        "low": bars.iter().map(|b| b.low).collect::<Vec<_>>(),
        "close": bars.iter().map(|b| b.close).collect::<Vec<_>>(),
        "volume": bars.iter().map(|b| b.volume).collect::<Vec<_>>(),
    })
}

fn modified(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn worker(
    script: PathBuf,
    sdk_dir: PathBuf,
    jobs: Receiver<Job>,
    out: Sender<Event>,
    wake: Arc<dyn Fn() + Send + Sync>,
) {
    let send = |e: Event| {
        if out.send(e).is_ok() {
            wake();
        }
    };
    let Some(python) = interpreter() else {
        send(Event::Error(
            "Python isn't installed (or isn't on the PATH). Install Python 3 to run Python scripts.".into(),
        ));
        return;
    };
    let runner = match install_sdk(&sdk_dir) {
        Ok(r) => r,
        Err(e) => {
            send(Event::Error(format!("Couldn't set up the Python SDK: {e}")));
            return;
        }
    };
    let mut proc: Option<Process> = None;
    let mut stamp = modified(&script);
    let mut last: Option<Job> = None;
    let mut described = false;
    loop {
        // The newest job wins; check the file every second meanwhile.
        let job = match jobs.recv_timeout(Duration::from_secs(1)) {
            Ok(j) => Some(j),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => None,
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return,
        };
        let job = job.map(|mut j| {
            while let Ok(newer) = jobs.try_recv() {
                j = newer;
            }
            j
        });
        let now = modified(&script);
        let reloaded = now != stamp;
        if reloaded {
            stamp = now;
            described = false;
        }
        if proc.is_none() {
            match Process::spawn(&python, &runner, &script) {
                Ok(p) => proc = Some(p),
                Err(e) => {
                    send(Event::Error(format!("Couldn't start Python: {e}")));
                    std::thread::sleep(Duration::from_secs(2));
                    continue;
                }
            }
        }
        let p = proc.as_mut().expect("started");
        if !described {
            match p.ask(&serde_json::json!({"cmd": "describe"})) {
                Ok(r) => {
                    if !r.log.is_empty() {
                        send(Event::Log(r.log));
                    }
                    match (r.ok, r.meta) {
                        (true, Some(m)) => {
                            described = true;
                            send(Event::Described(m));
                        }
                        _ => send(Event::Error(
                            r.error.unwrap_or_else(|| "The script didn't load.".into()),
                        )),
                    }
                }
                Err(e) => {
                    send(Event::Error(format!("Python stopped: {e}")));
                    proc = None;
                    continue;
                }
            }
        }
        let job = match (job, reloaded) {
            (Some(j), _) => Some(j),
            // An edit re-runs the last request.
            (None, true) => last.take(),
            _ => None,
        };
        let Some(job) = job else { continue };
        let started = Instant::now();
        let req = serde_json::json!({"cmd": "run", "inputs": job.inputs, "bars": bars_json(&job.bars)});
        match p.ask(&req) {
            Ok(r) => {
                if !r.log.is_empty() {
                    send(Event::Log(r.log));
                }
                match (r.ok, r.meta, r.out) {
                    (true, Some(meta), Some(out)) => send(Event::Output {
                        seq: job.seq,
                        meta,
                        out,
                        bars: job.bars.len(),
                    }),
                    _ => send(Event::Error(r.error.unwrap_or_else(|| "The script failed.".into()))),
                }
            }
            Err(e) => {
                send(Event::Error(format!("Python stopped: {e}")));
                proc = None;
            }
        }
        let _ = started;
        last = Some(job);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bars(n: usize) -> Arc<Vec<Bar>> {
        Arc::new(
            (0..n)
                .map(|i| {
                    let c = 100.0 + (i as f64 * 0.2).sin() * 5.0;
                    Bar {
                        time: i as i64 * 60_000,
                        open: c,
                        high: c + 1.0,
                        low: c - 1.0,
                        close: c,
                        volume: 10.0,
                    }
                })
                .collect(),
        )
    }

    fn wait(r: &Runner, f: impl Fn(&Event) -> bool) -> Event {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            for e in r.poll() {
                if let Event::Error(m) = &e {
                    panic!("script error: {m}");
                }
                if f(&e) {
                    return e;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out");
    }

    #[test]
    fn runs_an_indicator_and_reloads_on_save() {
        if interpreter().is_none() {
            eprintln!("no Python here; skipping");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("sma.py");
        std::fs::write(
            &script,
            "from opentern import *\nindicator('Two SMAs', overlay=True)\nn = input.int(5, 'Length')\n\
             def compute(bars):\n    print('hi')\n    plot(ta.sma(bars.close, n), 'SMA')\n    hline(100, 'Mid')\n",
        )
        .unwrap();
        let mut r = Runner::start(script.clone(), dir.path().join("sdk"), Arc::new(|| {}));
        let b = bars(50);
        r.run(b.clone(), serde_json::json!({"length": 3}));
        let Event::Output { meta, out, .. } = wait(&r, |e| matches!(e, Event::Output { .. })) else {
            unreachable!()
        };
        assert_eq!(meta.name, "Two SMAs");
        assert!(meta.overlay);
        assert_eq!(meta.inputs[0].id, "length");
        assert_eq!(out.plots[0].values.len(), 50);
        assert!(out.plots[0].values[1].is_none());
        let want = (b[0].close + b[1].close + b[2].close) / 3.0;
        assert!(
            (out.plots[0].values[2].unwrap() - want).abs() < 1e-9,
            "input 3 was used"
        );
        assert_eq!(out.hlines[0].value, 100.0);
        // An edit re-runs the last request on its own.
        std::thread::sleep(Duration::from_millis(1100));
        std::fs::write(
            &script,
            "from opentern import *\nindicator('Renamed')\ndef compute(bars):\n    plot(bars.close, 'C')\n",
        )
        .unwrap();
        let Event::Output { meta, .. } = wait(
            &r,
            |e| matches!(e, Event::Output { meta, .. } if meta.name == "Renamed"),
        ) else {
            unreachable!()
        };
        assert!(!meta.overlay);
    }

    #[test]
    fn reports_errors_and_strategy_orders() {
        if interpreter().is_none() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.py");
        std::fs::write(&bad, "from opentern import *\ndef compute(bars):\n    1/0\n").unwrap();
        let mut r = Runner::start(bad, dir.path().join("sdk"), Arc::new(|| {}));
        r.run(bars(10), serde_json::json!({}));
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut msg = None;
        while Instant::now() < deadline && msg.is_none() {
            for e in r.poll() {
                if let Event::Error(m) = e {
                    msg = Some(m);
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(msg.unwrap().contains("ZeroDivisionError"));

        let st = dir.path().join("st.py");
        std::fs::write(
            &st,
            "from opentern import *\nstrategy('X', initial_capital=1000)\n\
             def compute(bars):\n    up = ta.crossover(bars.close, ta.sma(bars.close, 3))\n    strategy.entry('L', strategy.long, when=up)\n",
        )
        .unwrap();
        let mut r = Runner::start(st, dir.path().join("sdk"), Arc::new(|| {}));
        r.run(bars(60), serde_json::json!({}));
        let Event::Output { meta, out, .. } = wait(&r, |e| matches!(e, Event::Output { .. })) else {
            unreachable!()
        };
        assert_eq!(meta.kind, "strategy");
        assert_eq!(meta.props["initial_capital"], 1000.0);
        assert!(!out.intents.is_empty());
        assert_eq!(out.intents[0]["type"], "entry");
    }
}
