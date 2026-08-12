use super::pricing::PricingTable;
use super::walker;
use crate::store::Db;
use anyhow::Result;
use notify::RecursiveMode;
use notify_debouncer_full::{new_debouncer, DebouncedEvent};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

pub struct WatcherHandle {
    _debouncer: notify_debouncer_full::Debouncer<
        notify::RecommendedWatcher,
        notify_debouncer_full::RecommendedCache,
    >,
}

pub fn start(
    db: Arc<Db>,
    pricing: Arc<PricingTable>,
    root: PathBuf,
    tx: mpsc::UnboundedSender<(PathBuf, usize)>,
) -> Result<WatcherHandle> {
    let (notify_tx, mut notify_rx) = mpsc::unbounded_channel::<Vec<DebouncedEvent>>();
    let mut debouncer = new_debouncer(Duration::from_millis(500), None, move |res| {
        if let Ok(events) = res {
            let _ = notify_tx.send(events);
        }
    })?;
    debouncer.watch(&root, RecursiveMode::Recursive)?;

    let db_clone = db.clone();
    let pricing_clone = pricing.clone();
    let root_clone = root.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(events) = notify_rx.recv().await {
            let mut touched = std::collections::HashSet::<PathBuf>::new();
            for e in &events {
                for p in &e.paths {
                    if p.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                        touched.insert(p.clone());
                    }
                }
            }
            for p in touched {
                match walker::ingest_file(&db_clone, &pricing_clone, &p, &root_clone) {
                    Ok(n) if n > 0 => {
                        let _ = tx.send((p.clone(), n));
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!("ingest {} failed: {}", p.display(), e),
                }
            }
        }
    });

    Ok(WatcherHandle {
        _debouncer: debouncer,
    })
}
