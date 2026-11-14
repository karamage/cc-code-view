//! AI 処理をバックグラウンドのスレッドで動かし、進捗と結果を画面へ届ける。
use ccv_ai::runner::{RunError, RunEvent};
use gpui::Context;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// バックグラウンドで AI 処理を動かし、進捗と結果を受け取る。
pub fn run_ai<V: 'static, T: Send + 'static>(
    cx: &mut Context<V>,
    work: impl FnOnce(&mut dyn FnMut(RunEvent)) -> Result<T, RunError> + Send + 'static,
    on_event: impl Fn(&mut V, RunEvent, &mut Context<V>) + 'static,
    on_done: impl FnOnce(&mut V, Result<T, RunError>, &mut Context<V>) + 'static,
) {
    let events: Arc<Mutex<Vec<RunEvent>>> = Default::default();
    let result: Arc<Mutex<Option<Result<T, RunError>>>> = Arc::new(Mutex::new(None));
    {
        let (events, result) = (events.clone(), result.clone());
        std::thread::spawn(move || {
            let mut push = |e: RunEvent| events.lock().unwrap().push(e);
            let r = work(&mut push);
            *result.lock().unwrap() = Some(r);
        });
    }
    cx.spawn(async move |this, cx| {
        let mut on_done = Some(on_done);
        loop {
            cx.background_executor().timer(Duration::from_millis(120)).await;
            let drained: Vec<RunEvent> = std::mem::take(&mut *events.lock().unwrap());
            let finished = result.lock().unwrap().take();
            let done = finished.is_some();
            let alive = this.update(cx, |this, cx| {
                for e in drained {
                    on_event(this, e, cx);
                }
                // 結果が届いたときにだけ取り出す。先に take すると完了処理を失う
                if let Some(r) = finished
                    && let Some(f) = on_done.take()
                {
                    f(this, r, cx);
                }
                // 経過時間の表示を進める
                cx.notify();
            });
            if done || alive.is_err() {
                break;
            }
        }
    })
    .detach();
}
