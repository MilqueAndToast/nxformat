//! Output existence checks run off the UI thread and are bound to the exact
//! current output plan. Existence never certifies archive integrity/equivalence.
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant},
};
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum OutputState {
    #[default]
    Checking,
    Missing,
    Exists,
    Blocked(String),
}
fn inspect(path: &Path) -> OutputState {
    match std::fs::metadata(path) {
        Ok(m) if m.is_file() => OutputState::Exists,
        Ok(_) => OutputState::Blocked(
            "The output path is occupied by a directory or another non-file entry.".into(),
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if std::fs::symlink_metadata(path).is_ok() {
                return OutputState::Blocked(
                    "The output path is occupied by a broken symbolic link.".into(),
                );
            }
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            match std::fs::metadata(parent) {
                Ok(m) if m.is_dir() => OutputState::Missing,
                Ok(_) => OutputState::Blocked("The output folder is not a directory.".into()),
                Err(e) => OutputState::Blocked(format!("Cannot access output folder: {e}")),
            }
        }
        Err(e) => OutputState::Blocked(format!("Cannot check output: {e}")),
    }
}
pub(super) fn collision_key(path: &Path) -> String {
    let mut normal = PathBuf::new();
    for c in path.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if normal
                    .components()
                    .next_back()
                    .is_some_and(|c| matches!(c, std::path::Component::Normal(_)))
                {
                    normal.pop();
                } else if !normal.has_root() {
                    normal.push("..");
                }
            }
            _ => normal.push(c),
        }
    }
    let text = normal.to_string_lossy().into_owned();
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    }
}
struct ResultSet {
    generation: u64,
    plan: Vec<(String, PathBuf)>,
    states: HashMap<PathBuf, OutputState>,
}
#[derive(Default)]
pub(super) struct OutputMonitor {
    plan: Vec<(String, PathBuf)>,
    generation: u64,
    rx: Option<mpsc::Receiver<ResultSet>>,
    states: HashMap<PathBuf, OutputState>,
    needed: bool,
    checked: Option<Instant>,
}
impl OutputMonitor {
    pub fn state(&self, path: &Path) -> OutputState {
        self.states.get(path).cloned().unwrap_or_default()
    }
    pub fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.states.clear();
        self.needed = true;
        self.checked = None;
    }
    pub fn update(&mut self, plan: Vec<(String, PathBuf)>, ctx: &eframe::egui::Context) {
        if plan != self.plan {
            self.plan = plan;
            self.invalidate();
        }
        if let Some(rx) = &self.rx {
            match rx.try_recv() {
                Ok(result) => {
                    self.rx = None;
                    if result.generation == self.generation && result.plan == self.plan {
                        self.states = result.states;
                        self.checked = Some(Instant::now());
                        self.needed = false;
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.rx = None;
                    self.needed = true;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.rx.is_none()
            && !self.plan.is_empty()
            && (self.needed
                || self
                    .checked
                    .is_some_and(|at| at.elapsed() > Duration::from_secs(2)))
        {
            self.generation = self.generation.wrapping_add(1);
            let generation = self.generation;
            let plan = self.plan.clone();
            let repaint = ctx.clone();
            let (tx, rx) = mpsc::channel();
            self.rx = Some(rx);
            std::thread::spawn(move || {
                let states = plan.iter().map(|(_, p)| (p.clone(), inspect(p))).collect();
                let _ = tx.send(ResultSet {
                    generation,
                    plan,
                    states,
                });
                repaint.request_repaint();
            });
        }
        if !self.plan.is_empty() {
            ctx.request_repaint_after(Duration::from_secs(2));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn collision_keys_preserve_relative_parent_directories() {
        assert_eq!(
            collision_key(Path::new("../a/./b/../c.nx")),
            collision_key(Path::new("../a/c.nx"))
        );
        assert_ne!(
            collision_key(Path::new("../a/c.nx")),
            collision_key(Path::new("a/c.nx"))
        );
    }
    fn wait(monitor: &mut OutputMonitor, plan: Vec<(String, PathBuf)>, path: &Path) {
        let ctx = eframe::egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            monitor.update(plan.clone(), &ctx);
            if monitor.state(path) != OutputState::Checking {
                return;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn stale_results_cannot_overwrite_new_folder_plan_or_post_conversion_refresh() {
        let root = std::env::temp_dir().join(format!("nx-output-stale-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let old = root.join("old.nx");
        let new = root.join("new.nx");
        std::fs::write(&old, b"exists").unwrap();
        for same_plan in [false, true] {
            let oldplan = vec![("game".into(), old.clone())];
            let plan = if same_plan {
                oldplan.clone()
            } else {
                vec![("game".into(), new.clone())]
            };
            let (tx, rx) = mpsc::channel();
            let mut monitor = OutputMonitor {
                plan: oldplan.clone(),
                generation: 10,
                rx: Some(rx),
                ..Default::default()
            };
            if same_plan {
                monitor.invalidate();
            }
            let ctx = eframe::egui::Context::default();
            monitor.update(plan.clone(), &ctx);
            tx.send(ResultSet {
                generation: 10,
                plan: oldplan,
                states: HashMap::from([(
                    old.clone(),
                    if same_plan {
                        OutputState::Missing
                    } else {
                        OutputState::Exists
                    },
                )]),
            })
            .unwrap();
            monitor.update(plan.clone(), &ctx);
            let path = if same_plan { &old } else { &new };
            assert_eq!(monitor.state(path), OutputState::Checking);
            wait(&mut monitor, plan, path);
            assert_eq!(
                monitor.state(path),
                if same_plan {
                    OutputState::Exists
                } else {
                    OutputState::Missing
                }
            );
            if !same_plan {
                assert_eq!(monitor.state(&old), OutputState::Checking);
            }
        }
        std::fs::remove_file(old).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn removed_queue_ignores_old_pending_results() {
        let path = PathBuf::from("removed.nx");
        let plan = vec![("removed".into(), path.clone())];
        let (tx, rx) = mpsc::channel();
        let mut monitor = OutputMonitor {
            plan: plan.clone(),
            generation: 3,
            rx: Some(rx),
            ..Default::default()
        };
        let ctx = eframe::egui::Context::default();
        monitor.update(vec![], &ctx);
        tx.send(ResultSet {
            generation: 3,
            plan,
            states: HashMap::from([(path.clone(), OutputState::Exists)]),
        })
        .unwrap();
        monitor.update(vec![], &ctx);
        assert_eq!(monitor.state(&path), OutputState::Checking);
        assert!(monitor.states.is_empty());
    }
}
