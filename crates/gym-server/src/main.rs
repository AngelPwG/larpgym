use anyhow::{Context, Result};
use gym_storage::{read_secret, Role, Store};
use std::{env, path::PathBuf};

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.first().is_some_and(|a| a == "backup") {
        anyhow::ensure!(args.len() == 3, "Uso: gym-server backup ORIGEN DESTINO");
        anyhow::ensure!(
            PathBuf::from(&args[1]).is_file(),
            "La base de origen no existe"
        );
        Store::open(PathBuf::from(&args[1]), Role::Server)?.backup(&PathBuf::from(&args[2]))?;
        println!("Copia SQLite consistente creada");
        return Ok(());
    }
    if !args.is_empty() {
        println!("gym-server: configuración mediante GYM_BIND, GYM_SERVER_DB y GYM_TOKEN_FILE\ngym-server backup ORIGEN DESTINO");
        return Ok(());
    }
    let bind = env::var("GYM_BIND").unwrap_or_else(|_| "127.0.0.1:8743".into());
    let db = env::var_os("GYM_SERVER_DB")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    PathBuf::from(env::var_os("HOME").unwrap_or_default()).join(".local/share")
                })
                .join("larpgym-server/server.sqlite")
        });
    let token = read_secret(&PathBuf::from(
        env::var_os("GYM_TOKEN_FILE")
            .context("Configura GYM_TOKEN_FILE con un archivo secreto de permisos 0600")?,
    ))?;
    let store = Store::open(db, Role::Server)?;
    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .context("No se pudo escuchar en GYM_BIND; elige una dirección y puerto disponibles")?;
    eprintln!(
        "gym-server: escuchando en {} (API v1 autenticada)",
        listener.local_addr()?
    );
    axum::serve(listener, gym_server::router(store, &token))
        .with_graceful_shutdown(shutdown())
        .await?;
    eprintln!("gym-server: cierre ordenado");
    Ok(())
}
async fn shutdown() {
    #[cfg(unix)]
    {
        if let Ok(mut term) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! { _=tokio::signal::ctrl_c()=>{}, _=term.recv()=>{} };
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}
