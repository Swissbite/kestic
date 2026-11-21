// SPDX-FileCopyrightText: 2026 David Däster <david@daester.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Plasma progress notification client via `org.kde.JobViewServerV2` / `org.kde.JobViewV3`.

use std::collections::HashMap;
use std::time::Duration;
use zbus::export::ordered_stream::OrderedStreamExt;
use zbus::{Connection, OwnedObjectPath, Proxy, Variant, block_on};

/// `KJob::Killable` capability flag so Plasma displays the Cancel ("Abbrechen") action.
pub const CAPABILITY_KILLABLE: i32 = 1;

/// Default D-Bus service name for Plasma's job view server.
pub const DEFAULT_JOB_VIEW_DEST: &str = "org.kde.JobViewServer";

/// Object path of the job view server.
pub const JOB_VIEW_SERVER_PATH: &str = "/JobViewServer";

/// Active handle to a registered Plasma job view (`org.kde.JobViewV3`).
pub struct JobViewHandle {
    conn: Connection,
    dest: String,
    view_path: OwnedObjectPath,
    terminated: bool,
}

/// Progress snapshot reported to `org.kde.JobViewV3.update`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobProgress {
    pub percent: u32,
    pub processed_bytes: u64,
    pub total_bytes: u64,
    pub speed_bytes_per_sec: u64,
    pub info_message: String,
    pub elapsed_ms: u64,
}

impl JobViewHandle {
    /// Requests a new job view on the specified D-Bus destination (allows pointing at a test fake).
    pub fn request(
        dest: &str,
        desktop_entry: &str,
        title: &str,
        capabilities: i32,
    ) -> zbus::Result<Self> {
        let dest_owned = dest.to_string();
        let entry_owned = desktop_entry.to_string();
        let title_owned = title.to_string();

        let (conn, view_path) = block_on(async move {
            let conn = Connection::session().await?;
            let proxy = Proxy::new(
                &conn,
                dest_owned.as_str(),
                JOB_VIEW_SERVER_PATH,
                "org.kde.JobViewServerV2",
            )
            .await?;

            let mut hints: HashMap<&str, Variant<'_>> = HashMap::new();
            hints.insert("immediate", Variant::from(true));
            hints.insert("title", Variant::from(title_owned.as_str()));

            let view_path: OwnedObjectPath = proxy
                .call("requestView", &(entry_owned.as_str(), capabilities, hints))
                .await?;
            Ok::<_, zbus::Error>((conn, view_path))
        })?;

        Ok(Self {
            conn,
            dest: dest.to_string(),
            view_path,
            terminated: false,
        })
    }

    /// Pushes a progress update via `org.kde.JobViewV3.update`.
    pub fn update(&self, progress: &JobProgress) -> zbus::Result<()> {
        let conn = self.conn.clone();
        let dest = self.dest.clone();
        let path = self.view_path.clone();
        let progress = progress.clone();

        block_on(async move {
            let proxy =
                Proxy::new(&conn, dest.as_str(), path.as_str(), "org.kde.JobViewV3").await?;

            let mut props: HashMap<&str, Variant<'_>> = HashMap::new();
            props.insert("percent", Variant::from(progress.percent));
            props.insert("processedBytes", Variant::from(progress.processed_bytes));
            props.insert("totalBytes", Variant::from(progress.total_bytes));
            props.insert("speed", Variant::from(progress.speed_bytes_per_sec));
            props.insert("elapsedTime", Variant::from(progress.elapsed_ms));
            props.insert("infoMessage", Variant::from(progress.info_message.as_str()));

            proxy.call_noreply("update", &(props,)).await?;
            Ok::<_, zbus::Error>(())
        })
    }

    /// Terminates the job view cleanly (`error_code = 0` for success).
    pub fn terminate(mut self, error_code: u32, error_message: &str) -> zbus::Result<()> {
        self.terminate_inner(error_code, error_message)
    }

    /// Spawns a background listener for `org.kde.JobViewV3.cancelRequested` that invokes `on_cancel`
    /// if Plasma's notification cancel button is pressed. Call [`JobViewCancelGuard::stop`] when done.
    ///
    /// The subscription is in place when this returns and stays until the guard stops, so no
    /// cancel request is lost between start and stop.
    pub fn start_cancel_watcher<F>(&self, on_cancel: F) -> JobViewCancelGuard
    where
        F: Fn() + Send + 'static,
    {
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let conn = self.conn.clone();
        let dest = self.dest.clone();
        let path = self.view_path.clone();
        let subscribed = block_on(async move {
            let proxy = Proxy::new(&conn, dest, path, "org.kde.JobViewV3").await?;
            proxy.receive_signal("cancelRequested").await
        });
        let mut stream = match subscribed {
            Ok(stream) => stream,
            Err(err) => {
                eprintln!("kestic-daemon: cancel in the Plasma progress view unavailable: {err}");
                return JobViewCancelGuard { stop, handle: None };
            }
        };

        let stop_bg = std::sync::Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            while !stop_bg.load(std::sync::atomic::Ordering::SeqCst) {
                match block_on(next_message_within(stream.next(), CANCEL_POLL_INTERVAL)) {
                    Some(Some(_)) => {
                        on_cancel();
                        break;
                    }
                    Some(None) => {
                        eprintln!(
                            "kestic-daemon: the Plasma progress view stopped sending cancel requests"
                        );
                        break;
                    }
                    None => {}
                }
            }
        });

        JobViewCancelGuard {
            stop,
            handle: Some(handle),
        }
    }

    fn terminate_inner(&mut self, error_code: u32, error_message: &str) -> zbus::Result<()> {
        if self.terminated {
            return Ok(());
        }
        self.terminated = true;
        let conn = self.conn.clone();
        let dest = self.dest.clone();
        let path = self.view_path.clone();
        let msg = error_message.to_string();

        block_on(async move {
            let proxy =
                Proxy::new(&conn, dest.as_str(), path.as_str(), "org.kde.JobViewV3").await?;
            let hints: HashMap<&str, Variant<'_>> = HashMap::new();
            proxy
                .call_noreply("terminate", &(error_code, msg.as_str(), hints))
                .await?;
            Ok::<_, zbus::Error>(())
        })
    }
}

/// Guard for stopping the background `JobViewV3.cancelRequested` listener.
pub struct JobViewCancelGuard {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl JobViewCancelGuard {
    pub fn stop(mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for JobViewCancelGuard {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// How often the cancel watcher checks whether it was stopped.
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(120);

/// Polls `fut` (sleeping briefly between polls) until it is ready or `timeout` passes:
/// `Some(item)` when ready (`Some(None)` = the stream ended), `None` on timeout.
async fn next_message_within<F>(fut: F, timeout: Duration) -> Option<Option<zbus::Message>>
where
    F: std::future::Future<Output = Option<zbus::Message>>,
{
    use std::pin::pin;
    let mut fut = pin!(fut);
    let start = std::time::Instant::now();
    std::future::poll_fn(|cx| {
        if let std::task::Poll::Ready(item) = fut.as_mut().poll(cx) {
            return std::task::Poll::Ready(Some(item));
        }
        if start.elapsed() >= timeout {
            return std::task::Poll::Ready(None);
        }
        std::thread::sleep(Duration::from_millis(10));
        cx.waker().wake_by_ref();
        std::task::Poll::Pending
    })
    .await
}

impl Drop for JobViewHandle {
    fn drop(&mut self) {
        if !self.terminated {
            let _ = self.terminate_inner(0, "");
        }
    }
}
