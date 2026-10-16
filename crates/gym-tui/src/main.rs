mod config;
mod ui;
mod worker;
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use gym_storage::{private_file, read_secret, Role, Store};
use gym_sync::SyncClient;
use std::{io, sync::mpsc, time::Duration};

struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, crossterm::cursor::Show);
    }
}
fn main() -> Result<()> {
    let (cfg, offline, backup) = config::load()?;
    if let Some(dest) = backup {
        anyhow::ensure!(
            cfg.database.is_file(),
            "La base de origen no existe; comprueba --db"
        );
        Store::open(&cfg.database, Role::Client)?.backup(&dest)?;
        println!("Copia SQLite consistente creada");
        return Ok(());
    }
    let timezone: chrono_tz::Tz = cfg
        .timezone
        .parse()
        .map_err(|_| anyhow::anyhow!("Zona horaria IANA inválida"))?;
    let lock = private_file(&cfg.database.with_extension("ui.lockfile"))?;
    fs2::FileExt::try_lock_exclusive(&lock)
        .map_err(|_| anyhow::anyhow!("Ya hay una TUI usando esta base"))?;
    let connection = if offline || cfg.server_url.is_none() {
        Ok(None)
    } else {
        (|| -> Result<Option<SyncClient>> {
            let token = read_secret(
                cfg.token_file
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("Configura token_file para sincronizar"))?,
            )?;
            Ok(Some(SyncClient::new(
                cfg.server_url.as_ref().unwrap(),
                token,
            )?))
        })()
    };
    let mut app = ui::App::new(
        chrono::Utc::now().with_timezone(&timezone).date_naive(),
        timezone,
    );
    let client = match connection {
        Ok(c) => {
            if c.is_none() {
                app.network = "Sin conexión configurada · modo local".into();
            }
            c
        }
        Err(e) => {
            app.network = format!("Sincronización deshabilitada: {e}");
            None
        }
    };
    let runtime = tokio::runtime::Runtime::new()?;
    let (events, rx) = mpsc::channel();
    let storage = worker::storage(cfg.database.clone(), events.clone());
    let (manual, manual_rx) = tokio::sync::mpsc::channel(1);
    let mut sync_start = Some((client, manual_rx));
    enable_raw_mode()?;
    let _guard = TerminalGuard;
    execute!(io::stdout(), EnterAlternateScreen, crossterm::cursor::Hide)?;
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, crossterm::cursor::Show);
        previous(info);
    }));
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(io::stdout()))?;
    let result = (|| -> Result<()> {
        loop {
            while let Ok(event) = rx.try_recv() {
                match event {
                    worker::Event::Loaded(snapshot) => {
                        app.snapshot = snapshot;
                        if !app.ready {
                            app.message = String::new();
                        }
                        app.ready = true;
                        if let Some((Some(client), manual_rx)) = sync_start.take() {
                            runtime.spawn(worker::synchronization(
                                client,
                                cfg.database.clone(),
                                cfg.sync_interval_secs,
                                manual_rx,
                                events.clone(),
                                storage.clone(),
                            ));
                        }
                    }
                    worker::Event::Saved(result) => app.saved(result),
                    worker::Event::Network(status) => app.network = status,
                    worker::Event::Fatal(error) => anyhow::bail!("{error}"),
                }
            }
            terminal.draw(|f| app.draw(f))?;
            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Release {
                        continue;
                    }
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('c')
                    {
                        if app.saving {
                            app.message = "Esperando confirmación de la escritura local…".into();
                        } else {
                            break;
                        }
                    } else if app.key(key, &storage, &manual)? {
                        break;
                    }
                }
            }
        }
        Ok(())
    })();
    let _ = storage.send(worker::Command::Stop);
    runtime.shutdown_timeout(Duration::from_secs(1));
    result
}
