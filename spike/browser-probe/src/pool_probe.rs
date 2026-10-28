//! Task 2 的真实窗口冒烟:驱动 `bytehost_desktop::web_child::WebPool`。
use bytehost_core::LogicalRect;
use bytehost_desktop::native_child::Probe;
use bytehost_desktop::web_child::{WebEvent, WebNav, WebPool, WebTabSpec, install_standard_edit_menu};
use iced::{Task, window};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::Arc;
use std::time::{Duration, Instant};

const DIR: &str = "/tmp/browser-probe";

struct App {
    win: Option<window::Id>,
    pool: WebPool,
    rx: Receiver<WebEvent>,
    epoch: Arc<AtomicU64>,
}

#[derive(Debug, Clone)]
enum Msg {
    Win(Option<window::Id>),
    Step(u32),
    Probed(String, Probe),
    Synced(&'static str),
    Failed(String),
    Poll,
}

fn out(s: impl AsRef<str>) {
    println!("PROBE {}", s.as_ref());
}
fn sleep(ms: u64) -> Task<()> {
    Task::perform(async move { tokio::time::sleep(Duration::from_millis(ms)).await }, |_| ())
}
fn rect() -> LogicalRect {
    LogicalRect { x: 20.0, y: 120.0, width: 420.0, height: 160.0 }
}
fn spec(id: usize, url: &str, visible: bool) -> WebTabSpec {
    WebTabSpec { id, url: url.into(), visible }
}

fn boot() -> (App, Task<Msg>) {
    let _ = std::fs::create_dir_all(DIR);
    for name in ["a.html", "My File.html"] {
        let _ = std::fs::write(
            format!("{DIR}/{name}"),
            format!("<!doctype html><meta charset=\"utf-8\"><title>{name}</title><body>{name}"),
        );
    }
    let epoch = Arc::new(AtomicU64::new(1));
    let (tx, rx) = channel();
    (
        App { win: None, pool: WebPool::new(epoch.clone(), tx), rx, epoch },
        window::oldest().map(Msg::Win),
    )
}

fn drain(app: &App) {
    while let Ok(ev) = app.rx.try_recv() {
        out(format!("event {ev:?}"));
    }
}

fn update(app: &mut App, msg: Msg) -> Task<Msg> {
    let Some(id) = app.win.or(match &msg { Msg::Win(w) => *w, _ => None }) else { return Task::none() };
    match msg {
        Msg::Win(w) => {
            app.win = w;
            out(format!("edit-menu first install = {}", install_standard_edit_menu()));
            out(format!("edit-menu second install = {} (幂等应为 false)", install_standard_edit_menu()));
            Task::done(Msg::Step(1))
        }
        Msg::Poll => {
            app.pool.poll_stalled(Instant::now());
            drain(app);
            Task::none()
        }
        Msg::Failed(e) => {
            out(format!("SYNC FAILED {e}"));
            Task::none()
        }
        Msg::Synced(label) => {
            out(format!("synced: {label}"));
            Task::none()
        }
        Msg::Probed(label, p) => {
            out(format!("probe {label}: exists={} visible={} bounds={:?} handles={}", p.exists, p.visible, p.bounds, p.handles));
            Task::none()
        }
        Msg::Step(n) => {
            let g = app.epoch.load(Ordering::SeqCst);
            let sync = |app: &App, label: &'static str, desired: Vec<WebTabSpec>| {
                app.pool.sync(id, g, desired, rect(), 1.0, Msg::Failed, Msg::Synced(label))
            };
            let probe = |app: &App, label: String, tab: usize| {
                let l = label.clone();
                app.pool.probe(id, g, tab, move |p| Msg::Probed(l, p))
            };
            match n {
                1 => Task::batch([sync(app, "3 tabs (1 visible, a hidden file, a javascript: tab)", vec![
                    spec(1, &format!("file://{DIR}/a.html"), true),
                    spec(2, &format!("file://{DIR}/My File.html"), false),
                    spec(3, "javascript:alert(1)", false),
                ])])
                .chain(sleep(2500).map(|_| Msg::Step(2))),
                2 => { drain(app); Task::batch([probe(app, "tab1".into(), 1), probe(app, "tab2".into(), 2), probe(app, "tab3(blocked)".into(), 3)]).chain(Task::done(Msg::Step(3))) }
                3 => sync(app, "switch active to tab 2", vec![
                        spec(1, &format!("file://{DIR}/a.html"), false),
                        spec(2, &format!("file://{DIR}/My File.html"), true),
                        spec(3, "javascript:alert(1)", false),
                    ]).chain(sleep(800).map(|_| Msg::Step(4))),
                4 => { drain(app); Task::batch([probe(app, "tab1 after switch".into(), 1), probe(app, "tab2 after switch".into(), 2)]).chain(Task::done(Msg::Step(5))) }
                5 => app.pool.nav(id, g, 2, WebNav::Refresh, Msg::Synced("refresh tab2")).chain(sleep(1500).map(|_| Msg::Step(6))),
                6 => { drain(app); sync(app, "add an unreachable tab", vec![
                        spec(1, &format!("file://{DIR}/a.html"), false),
                        spec(2, &format!("file://{DIR}/My File.html"), false),
                        spec(3, "javascript:alert(1)", false),
                        spec(4, "http://127.0.0.1:9/never", true),
                    ]).chain(sleep(17000).map(|_| Msg::Step(7))) }
                7 => { app.pool.poll_stalled(Instant::now()); drain(app); out("(上面应有 NavigationStalled{id:4})"); Task::done(Msg::Step(8)) }
                8 => { app.epoch.store(2, Ordering::SeqCst); sync_gen2(app, id) }
                9 => { drain(app); Task::batch([
                        app.pool.probe(id, 2, 1, |p| Msg::Probed("gen2 tab1".into(), p)),
                        app.pool.probe(id, 1, 2, |p| Msg::Probed("old gen1 tab2 (应 exists=false)".into(), p)),
                    ]).chain(Task::done(Msg::Step(10))) }
                10 => app.pool.destroy_all(id, Msg::Synced("destroy_all")).chain(sleep(500).map(|_| Msg::Step(11))),
                11 => app.pool.probe(id, 2, 1, |p| Msg::Probed("after destroy_all tab1".into(), p)).chain(sleep(300).map(|_| Msg::Step(12))),
                _ => { drain(app); out("DONE"); iced::exit() }
            }
        }
    }
}

fn sync_gen2(app: &App, id: window::Id) -> Task<Msg> {
    app.pool
        .sync(id, 2, vec![spec(1, &format!("file://{DIR}/a.html"), true)], rect(), 1.0, Msg::Failed, Msg::Synced("generation 2: only tab1 (A/B 撞号的 id=1)"))
        .chain(sleep(2500).map(|_| Msg::Step(9)))
}

fn view(_: &App) -> iced::Element<'_, Msg> {
    iced::widget::text("pool-probe 运行中…").into()
}

fn main() -> iced::Result {
    iced::application(boot, update, view)
        .subscription(|_| iced::time::every(Duration::from_millis(300)).map(|_| Msg::Poll))
        .window_size((480.0, 330.0))
        .run()
}
