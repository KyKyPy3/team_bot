use std::{collections::HashMap, sync::Arc};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use uuid::Uuid;
use zulip::ZulipOutput;

pub mod zulip;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Action {
  pub name: String,
  pub task_id: Uuid,
  pub options: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ActionConfig {
  pub name: String,
  pub options: Value,
}

pub struct ActionSystem {
  actions: Arc<HashMap<String, Box<dyn ActionAtom>>>,
}

impl ActionSystem {
  pub async fn new(config: Vec<ActionConfig>) -> Result<Self> {
    let mut actions = HashMap::new();

    for action in config {
      match action.name.as_str() {
        "zulip" => match ZulipOutput::new(action.options.clone()) {
          Ok(zulip) => {
            let zulip: Box<dyn ActionAtom> = Box::new(zulip);
            actions.insert(action.name.clone(), zulip);
          },
          Err(err) => {
            return Err(anyhow!("Can't create {} action. Err: {}", action.name.as_str(), err));
          },
        },
        _ => {
          warn!("Unknown action name: {}", action.name);
        },
      }
    }

    Ok(Self {
      actions: Arc::new(actions),
    })
  }

  pub async fn run(&self, cancel_token: CancellationToken) -> Result<()> {
    for (name, action) in self.actions.iter() {
      if let Err(e) = action.run(cancel_token.clone()).await {
        error!("Can't start action {}. Got error: {}", name, e);
      }
    }

    Ok(())
  }

  pub async fn process(&self, action: Action) -> Result<()> {
    info!("Processing action {:?}", action);

    match self.actions.get(action.name.as_str()) {
      Some(action_atom) => {
        action_atom.process_action(action).await?;

        Ok(())
      },
      None => {
        warn!("Unsupported action type {}", action.name);

        Err(anyhow!("Unsupported action type {}", action.name))
      },
    }
  }
}

#[async_trait]
pub trait ActionAtom: Send + Sync {
  async fn run(&self, cancellation_token: CancellationToken) -> Result<()>;

  async fn process_action(&self, action: Action) -> Result<()>;
}
