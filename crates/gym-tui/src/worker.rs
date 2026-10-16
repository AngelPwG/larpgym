use gym_core::Document;
use gym_storage::{Conflict, Role, Snapshot, Store};
use gym_sync::SyncClient;
use std::{path::PathBuf, sync::mpsc, time::Duration};
pub enum Command {
    Save {
        document: Document,
        original: Option<Document>,
    },
    Resolve {
        conflict: Conflict,
        local: bool,
    },
    Refresh,
    Stop,
}
pub enum Event {
    Loaded(Snapshot),
    Saved(Result<Snapshot, String>),
    Network(String),
    Fatal(String),
}
pub fn storage(path: PathBuf, tx: mpsc::Sender<Event>) -> mpsc::Sender<Command> {
    let (commands, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let opened = (|| -> anyhow::Result<Store> {
            let mut s = Store::open(path, Role::Client)?;
            s.seed_catalogue()?;
            Ok(s)
        })();
        let mut store = match opened {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(Event::Fatal(e.to_string()));
                return;
            }
        };
        match store.snapshot() {
            Ok(s) => {
                let _ = tx.send(Event::Loaded(s));
            }
            Err(e) => {
                let _ = tx.send(Event::Fatal(e.to_string()));
                return;
            }
        }
        while let Ok(command) = rx.recv() {
            let result = match command {
                Command::Save { document, original } => {
                    store.save_checked(document, original.as_ref())
                }
                Command::Resolve { conflict, local } => store.resolve(&conflict, local),
                Command::Refresh => {
                    if let Ok(s) = store.snapshot() {
                        let _ = tx.send(Event::Loaded(s));
                    }
                    continue;
                }
                Command::Stop => break,
            };
            let result = result
                .and_then(|_| store.snapshot())
                .map_err(|e| e.to_string());
            let _ = tx.send(Event::Saved(result));
        }
    });
    commands
}
pub async fn synchronization(
    client: SyncClient,
    path: PathBuf,
    interval: u64,
    mut manual: tokio::sync::mpsc::Receiver<()>,
    tx: mpsc::Sender<Event>,
    storage: mpsc::Sender<Command>,
) {
    let mut failures = 0u32;
    loop {
        let _ = tx.send(Event::Network("Sincronizando…".into()));
        let wait = match client.synchronize(&path).await {
            Ok(()) => {
                failures = 0;
                let _ = tx.send(Event::Network(
                    "Conectado · última comunicación correcta".into(),
                ));
                interval
            }
            Err(e) => {
                failures = (failures + 1).min(7);
                let delay = (5u64 * 2u64.pow(failures - 1)).min(300);
                let _ = tx.send(Event::Network(format!("{e} · reintento en {delay}s")));
                delay
            }
        };
        let _ = storage.send(Command::Refresh);
        tokio::select! { _=tokio::time::sleep(Duration::from_secs(wait))=>{}, command=manual.recv()=>{ if command.is_none() { return; } } }
    }
}
