use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::sync::{
  mpsc::{self, Receiver, Sender},
  oneshot::{self, error::RecvError},
  Mutex, Semaphore,
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, instrument};

use super::{Action, ActionAtom};

pub const ZULIP: &str = "zulip";
const CHANNEL_BUFFER_SIZE: usize = 500;
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(1024);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

type MessageChannel = (Msg, oneshot::Sender<Result<()>>);
type MessageReceiver = Arc<Mutex<Receiver<MessageChannel>>>;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Msg {
  pub topic: Option<String>,
  pub channel: String,
  pub message: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Options {
  url: String,
  login: String,
  token: String,
  max_request: usize,
}

#[derive(Deserialize, Serialize, Debug)]
struct PostMessageResponse {
  id: i32,
  msg: String,
  result: String,
}

struct ZulipClient {
  http_client: Client,
  options: Options,
}

pub struct ZulipOutput {
  client: Arc<ZulipClient>,
  receiver: MessageReceiver,
  sender: Sender<MessageChannel>,
}

#[derive(Debug, Error)]
pub enum RequestError {
  #[error("Failed to submit chat message: Queue is full")]
  QueueError,
  #[error("Request returned an invalid status code: {0}")]
  StatusCodeError(u16),
  #[error("Failed to parse response: {0}")]
  ParseError(reqwest::Error),
  #[error("Request failed: {0}")]
  ReqwestError(#[from] reqwest::Error),
  #[error("Message submission failed: {0}")]
  SendError(String),
  #[error("Failed to receive response: {0}")]
  RecvError(#[from] RecvError),
  #[error("Failed to acquire request lock: {0}")]
  SemaphoreError(tokio::sync::AcquireError),
  #[error("Max retries exceeded")]
  MaxRetriesExceeded,
}

impl ZulipClient {
  pub fn new(options: Value) -> Result<Self> {
    let options = serde_json::from_value(options).map_err(|_| anyhow!("Invalid Zulip configuration"))?;

    let http_client = Client::builder()
      .connect_timeout(CONNECTION_TIMEOUT)
      .timeout(REQUEST_TIMEOUT)
      .user_agent("Platform-Bot")
      .build()?;

    Ok(Self { options, http_client })
  }

  #[instrument(skip(self))]
  async fn send_with_retry(&self, msg: Msg, max_retries: u32) -> Result<(), RequestError> {
    for attempt in 0..max_retries {
      match self.send_message(msg.clone()).await {
        Ok(_) => return Ok(()),
        Err(_) if attempt < max_retries - 1 => {
          tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
          continue;
        },
        Err(e) => return Err(e),
      }
    }
    Err(RequestError::MaxRetriesExceeded)
  }

  #[instrument(skip(self))]
  async fn send_message(&self, msg: Msg) -> Result<(), RequestError> {
    let topic = msg.topic.unwrap_or_default();
    let query = [
      ("type", "stream"),
      ("to", &msg.channel),
      ("topic", &topic),
      ("content", &msg.message),
    ];

    debug!("Connect to zulip server with options {:?}", self.options);

    let response = self
      .http_client
      .post(&self.options.url)
      .basic_auth(&self.options.login, Some(&self.options.token))
      .query(&query)
      .send()
      .await?;

    if !response.status().is_success() {
      return Err(RequestError::StatusCodeError(response.status().as_u16()));
    }

    let resp: PostMessageResponse = response.json().await?;
    if resp.result != "success" {
      return Err(RequestError::SendError(resp.msg));
    }

    Ok(())
  }
}

impl ZulipOutput {
  pub fn new(options: Value) -> Result<Self> {
    let client = Arc::new(ZulipClient::new(options)?);
    let (sender, receiver) = mpsc::channel(CHANNEL_BUFFER_SIZE);

    Ok(Self {
      client,
      sender,
      receiver: Arc::new(Mutex::new(receiver)),
    })
  }

  async fn handle_message(
    client: Arc<ZulipClient>,
    msg: Msg,
    response_channel: oneshot::Sender<Result<()>>,
    semaphore: Arc<Semaphore>,
  ) {
    let _permit = match semaphore.acquire_owned().await {
      Ok(permit) => permit,
      Err(e) => {
        response_channel
          .send(Err(anyhow!(RequestError::SemaphoreError(e))))
          .ok();
        return;
      },
    };

    debug!("Processing message: {:?}", &msg);
    let result = client.send_message(msg).await;

    if let Err(e) = response_channel.send(result.map_err(Into::into)) {
      error!("Failed to send response: {:?}", e);
    }
  }
}

#[async_trait]
impl ActionAtom for ZulipOutput {
  #[instrument(skip(self, cancel_token))]
  async fn run(&self, cancel_token: CancellationToken) -> Result<()> {
    let semaphore = Arc::new(Semaphore::new(self.client.options.max_request));
    let receiver = self.receiver.clone();
    let client = Arc::clone(&self.client);

    tokio::spawn(async move {
      debug!("Starting Zulip output processor");

      loop {
        let mut rx = receiver.lock().await;

        tokio::select! {
            Some((msg, response_channel)) = rx.recv() => {
                tokio::spawn(Self::handle_message(
                    client.clone(),
                    msg,
                    response_channel,
                    semaphore.clone(),
                ));
            }
            _ = cancel_token.cancelled() => break,
        }
      }

      debug!("Shutting down Zulip output processor");
    });

    Ok(())
  }

  #[instrument(skip(self, action))]
  async fn process_action(&self, action: Action) -> Result<()> {
    let (tx, rx) = oneshot::channel();
    let msg: Msg = serde_json::from_str(&action.options)?;

    info!("Processing message: {:?}", &msg);

    self
      .sender
      .send((msg, tx))
      .await
      .map_err(|_| anyhow!(RequestError::QueueError))?;

    rx.await?
  }
}
