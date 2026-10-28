//! 页面自己跳转后,宿主把 `PageLoaded` 的网址回灌给池:池不得把同一页再导航一遍。
use bytehost_core::LogicalRect;
use bytehost_desktop::web_child::{WebEvent, WebPool, WebTabSpec};
use iced::{Task, window};
use std::sync::atomic::AtomicU64;
use std::sync::mpsc::{Receiver, channel};
use std::sync::Arc;
use std::time::Duration;

struct App { win: Option<window::Id>, pool: WebPool, rx: Receiver<WebEvent>, last: Option<String>, loads: usize }
#[derive(Debug, Clone)]
enum Msg { Win(Option<window::Id>), Step(u32), Done, Failed(String) }

fn rect() -> LogicalRect { LogicalRect { x: 20.0, y: 20.0, width: 300.0, height: 200.0 } }
fn sleep(ms: u64) -> Task<()> { Task::perform(async move { tokio::time::sleep(Duration::from_millis(ms)).await }, |_| ()) }
fn drain(app: &mut App) {
    while let Ok(ev) = app.rx.try_recv() {
        println!("PROBE event {ev:?}");
        if let WebEvent::PageLoaded { url, .. } = ev { app.last = Some(url); app.loads += 1; }
    }
}
fn boot() -> (App, Task<Msg>) {
    let _ = std::fs::create_dir_all("/tmp/browser-probe");
    let _ = std::fs::write(
        "/tmp/browser-probe/self.html",
        "<!doctype html><title>self</title><body>self<script>setTimeout(()=>{location.href='file:///tmp/browser-probe/self2.html'},400)</script>",
    );
    let _ = std::fs::write(
        "/tmp/browser-probe/self2.html",
        "<!doctype html><title>self2</title><body>self2",
    );
    let (tx, rx) = channel();
    (App { win: None, pool: WebPool::new(Arc::new(AtomicU64::new(1)), tx), rx, last: None, loads: 0 }, window::oldest().map(Msg::Win))
}
fn update(app: &mut App, msg: Msg) -> Task<Msg> {
    match msg {
        Msg::Win(w) => { app.win = w; Task::done(Msg::Step(1)) }
        Msg::Failed(e) => { println!("PROBE SYNC FAILED {e}"); Task::none() }
        Msg::Done => Task::none(),
        Msg::Step(n) => {
            let Some(id) = app.win else { return Task::none() };
            let sync = |app: &App, url: &str| app.pool.sync(id, 1, vec![WebTabSpec { id: 1, url: url.into(), visible: true }], rect(), 1.0, Msg::Failed, Msg::Done);
            match n {
                1 => sync(app, "file:///tmp/browser-probe/self.html").chain(sleep(2500).map(|_| Msg::Step(2))),
                2 => {
                    drain(app);
                    let url = app.last.clone().unwrap_or_default();
                    println!("PROBE page ended at {url}; loads so far = {}", app.loads);
                    app.loads = 0;
                    // 宿主把页面实际所在的地址回灌成期望网址。
                    sync(app, &url).chain(sleep(2500).map(|_| Msg::Step(3)))
                }
                _ => {
                    drain(app);
                    println!("PROBE extra loads after feeding the page's own url back = {} (应为 0)", app.loads);
                    iced::exit()
                }
            }
        }
    }
}
fn view(_: &App) -> iced::Element<'_, Msg> { iced::widget::text("selfnav").into() }
fn main() -> iced::Result {
    iced::application(boot, update, view).window_size((360.0, 260.0)).run()
}
