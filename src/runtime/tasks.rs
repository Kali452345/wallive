//! Background work in short-lived child processes of this same exe:
//! `--pick` (file dialog) and `--import` (transcode, ADR-004). The resident
//! process only keeps one blocked waiter thread per running task; it wakes
//! when the child exits and posts the result to the UI thread. Children
//! belong to a kill-on-close job, so they never outlive the app.

use std::os::windows::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use crate::shell::ChildJob;

/// `CREATE_NO_WINDOW` | `BELOW_NORMAL_PRIORITY_CLASS`: imports run quietly
/// in the background without competing with the user's foreground work.
const IMPORT_FLAGS: u32 = 0x0800_0000 | 0x0000_4000;

#[derive(Debug)]
pub enum Done {
    /// The picker closed; empty when cancelled.
    Picked(Vec<PathBuf>),
    Imported {
        source: PathBuf,
        output: PathBuf,
        result: Result<(), String>,
    },
}

pub struct Tasks {
    exe: PathBuf,
    host: isize,
    job: Option<Arc<ChildJob>>,
    tx: Sender<Done>,
    rx: Receiver<Done>,
    picking: bool,
    imports: usize,
}

impl Tasks {
    pub fn new(exe: PathBuf, host: isize) -> Self {
        let (tx, rx) = channel();
        let job = ChildJob::new()
            .map_err(|e| crate::log!("tasks: job object failed: {e}"))
            .ok()
            .map(Arc::new);
        Self {
            exe,
            host,
            job,
            tx,
            rx,
            picking: false,
            imports: 0,
        }
    }

    /// The file dialog is open.
    pub fn picking(&self) -> bool {
        self.picking
    }

    /// An import is running.
    pub fn importing(&self) -> bool {
        self.imports > 0
    }

    /// Results that arrived since the last call.
    pub fn finished(&mut self) -> Vec<Done> {
        let done: Vec<Done> = self.rx.try_iter().collect();
        for d in &done {
            match d {
                Done::Picked(_) => self.picking = false,
                Done::Imported { .. } => self.imports = self.imports.saturating_sub(1),
            }
        }
        done
    }

    pub fn pick(&mut self) {
        let mut cmd = Command::new(&self.exe);
        cmd.arg("--pick").stdout(Stdio::piped());
        crate::shell::allow_foreground();
        self.picking = self.spawn("picker", cmd, |output| {
            let paths = if output.status.success() {
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(PathBuf::from)
                    .collect()
            } else {
                Vec::new()
            };
            Done::Picked(paths)
        });
    }

    pub fn import(&mut self, source: &Path, output: &Path, size: (u32, u32)) {
        let mut cmd = Command::new(&self.exe);
        cmd.arg("--import")
            .arg(source)
            .arg(output)
            .arg(format!("{}x{}", size.0, size.1))
            .creation_flags(IMPORT_FLAGS);
        let (source, output) = (source.to_path_buf(), output.to_path_buf());
        let started = self.spawn("import", cmd, move |out| Done::Imported {
            source,
            output,
            result: if out.status.success() {
                Ok(())
            } else {
                Err(format!("import exited with {}", out.status))
            },
        });
        if started {
            self.imports += 1;
        }
    }

    fn spawn(
        &mut self,
        name: &'static str,
        mut cmd: Command,
        finish: impl FnOnce(std::process::Output) -> Done + Send + 'static,
    ) -> bool {
        // Children log into our log file, or our stderr when there is none.
        match crate::log_file_clone() {
            Some(file) => cmd.stderr(file),
            None => cmd.stderr(Stdio::inherit()),
        };
        let child = match cmd.stdin(Stdio::null()).spawn() {
            Ok(child) => child,
            Err(e) => {
                crate::log!("tasks: {name} failed to start: {e}");
                return false;
            }
        };
        if let Some(job) = &self.job {
            job.assign(&child);
        }
        crate::log!("tasks: {name} started (pid {})", child.id());
        let (tx, host) = (self.tx.clone(), self.host);
        let spawned = std::thread::Builder::new()
            .name(format!("wallive-{name}-wait"))
            .stack_size(64 * 1024)
            .spawn(move || {
                let done = match child.wait_with_output() {
                    Ok(output) => finish(output),
                    Err(e) => finish(std::process::Output {
                        status: std::process::ExitStatus::from_raw(1),
                        stdout: Vec::new(),
                        stderr: e.to_string().into_bytes(),
                    }),
                };
                if tx.send(done).is_ok() {
                    super::ffi::post_task_done(host);
                }
            });
        match spawned {
            Ok(_) => true,
            Err(e) => {
                crate::log!("tasks: {name} waiter failed to start: {e}");
                false
            }
        }
    }
}
