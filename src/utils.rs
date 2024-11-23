use anyhow::{Error, Result};
use tokio::sync::mpsc::channel;
use tokio_util::sync::CancellationToken;
use tracing_log::log::debug;

pub type Task = futures::future::BoxFuture<'static, Result<()>>;

pub async fn join_all(tasks: Vec<Task>, cancel_token: CancellationToken) -> Result<()> {
  let (sender, mut receiver) = channel::<Error>(1);
  for task in tasks {
    let sender = sender.clone();
    tokio::spawn(async move {
      if let Err(e) = task.await {
        sender
          .send(e)
          .await
          .unwrap_or_else(|_| unreachable!("This channel never closed."));
      }
    });
  }
  tokio::select! {
    res = receiver.recv() => {
      match res {
        Some(err) => Err(err),
        None => unreachable!("This channel never closed."),
      }
    },
    _ = cancel_token.cancelled() => {
      debug!("Receive cancel signal...");

      Ok(())
    },
  }
}
