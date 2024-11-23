use std::{collections::HashMap, env, sync::Arc};

use chrono::{DateTime, FixedOffset, Local, NaiveDateTime, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use curl::easy::{Auth, Easy2, Handler, List, WriteError};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, instrument, warn};

use crate::error::ServiceError;
use crate::ServiceResult;
use team_bot_entity::project::ProjectRow;

mod constants {
  use std::time::Duration;

  pub const CONNECTION_TIMEOUT: Duration = Duration::from_secs(1024);
  pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
  pub const QUERY_INTERVAL: Duration = Duration::from_secs(15);

  pub const EVENT_SHEBANG: &str = "[PLATFORM_BOT]";
  pub const EVENT_SHEBANG_REGEXP: &str = r"(?:\[PLATFORM_BOT\])([\s\S]*)(?:\[PLATFORM_BOT\])";
  pub const USER_AGENT: &str = "Platform-Bot";
}

#[derive(Debug)]
struct ResponseCollector(Vec<u8>);

impl Handler for ResponseCollector {
  fn write(&mut self, data: &[u8]) -> Result<usize, WriteError> {
    self.0.extend_from_slice(data);
    Ok(data.len())
  }
}

#[derive(Deserialize, Serialize, Debug)]
struct EventLocation {
  #[serde(alias = "DisplayName")]
  display_name: String,
}

#[derive(Deserialize, Serialize, Debug)]
struct EventBody {
  #[serde(alias = "Content")]
  content: String,
  #[serde(alias = "ContentType")]
  content_type: String,
}

#[derive(Deserialize, Serialize, Debug)]
struct EventDate {
  #[serde(alias = "DateTime")]
  date_time: String,
  #[serde(alias = "TimeZone")]
  timezone: String,
}

#[derive(Deserialize, Serialize, Debug)]
struct ExchangeEvent {
  #[serde(alias = "Id")]
  id: String,
  #[serde(alias = "Subject")]
  subject: String,
  #[serde(alias = "Location")]
  location: EventLocation,
  #[serde(alias = "Start")]
  start: EventDate,
  #[serde(alias = "LastModifiedDateTime")]
  last_modified: String,
  #[serde(alias = "Body")]
  body: EventBody,
}

#[derive(Deserialize, Serialize, Debug)]
struct ExchangeResponse {
  value: Vec<ExchangeEvent>,
}

#[derive(Debug)]
struct ExchangeClient {
  client: Easy2<ResponseCollector>,
  base_url: String,
}

impl ExchangeClient {
  #[instrument(level = "debug", skip(password))]
  fn new(server: String, username: String, password: String) -> ServiceResult<Self> {
    let mut auth = Auth::new();
    auth.ntlm(true);
    auth.gssnegotiate(false);

    let mut headers = List::new();
    headers
      .append("Prefer: outlook.body-content-type=\"text\"")
      .map_err(|e| ServiceError::config_error(format!("Failed to create request headers: {}", e)))?;
    headers
      .append("Prefer: outlook.timezone=\"Europe/Moscow\"")
      .map_err(|e| ServiceError::config_error(format!("Failed to create request headers: {}", e)))?;

    let mut client = Easy2::new(ResponseCollector(Vec::new()));
    client
      .http_auth(&auth)
      .map_err(|e| ServiceError::config_error(format!("Failed to set http auth: {}", e)))?;
    client
      .username(&username)
      .map_err(|e| ServiceError::config_error(format!("Failed to set username: {}", e)))?;
    client
      .password(&password)
      .map_err(|e| ServiceError::config_error(format!("Failed to set password: {}", e)))?;
    client
      .connect_timeout(constants::CONNECTION_TIMEOUT)
      .map_err(|e| ServiceError::config_error(format!("Failed to set connection timeout: {}", e)))?;
    client
      .timeout(constants::REQUEST_TIMEOUT)
      .map_err(|e| ServiceError::config_error(format!("Failed to set request timeout: {}", e)))?;
    client
      .useragent(constants::USER_AGENT)
      .map_err(|e| ServiceError::config_error(format!("Failed to set user agent: {}", e)))?;
    client
      .http_headers(headers)
      .map_err(|e| ServiceError::config_error(format!("Failed to set headers: {}", e)))?;

    Ok(Self {
      client,
      base_url: server,
    })
  }

  #[instrument(level = "debug")]
  async fn fetch_events(&mut self, start: DateTime<Utc>, end: DateTime<Utc>) -> ServiceResult<ExchangeResponse> {
    let url = format!(
      "{}/api/v2.0/me/calendarview?startDateTime={}&endDateTime={}&$select=Subject,Start,Location,Body,LastModifiedDateTime",
      self.base_url,
      start.format("%Y-%m-%dT%H:%M:%S"),
      end.format("%Y-%m-%dT%H:%M:%S"),
    );

    info!("Request url: {}", url);

    self
      .client
      .url(&url)
      .map_err(|e| ServiceError::config_error(format!("Failed to set request url: {}", e)))?;
    self
      .client
      .perform()
      .map_err(|e| ServiceError::InternalError(e.into()))?;

    let response_code = self.client.response_code()?;
    if response_code != 200 {
      let error_content = String::from_utf8_lossy(&self.client.get_ref().0);
      return Err(ServiceError::exchange_api_error(
        response_code,
        error_content.to_string(),
      ));
    }

    let content = String::from_utf8_lossy(&self.client.get_ref().0);
    let events = serde_json::from_str(&content)
      .map_err(|e| ServiceError::parse_error(format!("Failed to parse exchange response: {}", e)))?;

    // Clear buffer after reading
    self.client.get_mut().0.clear();

    Ok(events)
  }
}

struct EventProcessor<'a> {
  pool: Arc<SqlitePool>,
  regex: Regex,
  phantom: std::marker::PhantomData<&'a ()>,
}

impl<'a> EventProcessor<'a> {
  fn new(pool: Arc<SqlitePool>) -> ServiceResult<Self> {
    let regex = Regex::new(constants::EVENT_SHEBANG_REGEXP)
      .map_err(|e| ServiceError::parse_error(format!("Failed to parse bot options regexp: {}", e)))?;

    Ok(Self {
      pool,
      regex,
      phantom: std::marker::PhantomData,
    })
  }

  #[instrument(level = "debug", skip(self))]
  async fn process_events(&self, events: Vec<ExchangeEvent>) -> ServiceResult<()> {
    let projects = self.load_projects().await?;

    for event in events {
      if !event.body.content.contains(constants::EVENT_SHEBANG) {
        debug!("Found exchange event without shebang. Event subject: {}", event.subject);
        continue;
      }

      if let Err(e) = self.process_single_event(&event, &projects).await {
        error!("Failed to process event {}: {}", event.subject, e);
      }
    }

    self.cleanup_old_tasks().await?;
    Ok(())
  }

  #[instrument(level = "debug", skip(self, projects))]
  async fn process_single_event<'b>(
    &self,
    event: &ExchangeEvent,
    projects: &'b HashMap<String, ProjectRow>,
  ) -> ServiceResult<()> {
    let options = self.parse_event_options(&event)?;
    let project = self.get_project_from_options(&options, projects)?;
    let start_time = self.parse_event_time(&event)?;
    let modified_at: DateTime<FixedOffset> = DateTime::parse_from_rfc3339(&event.last_modified)
      .map_err(|e| ServiceError::parse_error(format!("Invalid datetime format: {}", e)))?;

    let task_params = crate::mutation::tasks::CreateTaskParams {
      name: event.subject.clone(),
      r#type: String::from("notify"),
      schedule: None,
      project_id: project.id,
      external_id: Some(event.id.clone()),
      external_modified_at: Some(modified_at.to_utc()),
      start_at: start_time.timestamp() as i32,
      options: serde_json::to_value(options)
        .map_err(|e| ServiceError::parse_error(format!("Failed to parse task options: {}", e)))?,
    };

    crate::mutation::tasks::create(&self.pool, task_params).await?;
    Ok(())
  }

  fn parse_event_options(&self, event: &ExchangeEvent) -> ServiceResult<HashMap<String, String>> {
    let captures = self
      .regex
      .captures(&event.body.content)
      .ok_or_else(|| ServiceError::invalid_event("No bot options found in event"))?;

    parse_options(&captures[1].trim())
  }

  async fn load_projects(&self) -> ServiceResult<HashMap<String, ProjectRow>> {
    let projects = crate::query::projects::list_all(&self.pool).await?;
    Ok(
      projects
        .into_iter()
        .map(|p| (p.code.clone(), p))
        .collect::<HashMap<String, ProjectRow>>(),
    )
  }

  fn get_project_from_options<'b>(
    &self,
    options: &HashMap<String, String>,
    projects: &'b HashMap<String, ProjectRow>,
  ) -> ServiceResult<&'b ProjectRow> {
    let project_code = options
      .get("project")
      .ok_or_else(|| ServiceError::invalid_event("No project code in options"))?;

    projects
      .get(project_code)
      .ok_or_else(|| ServiceError::ResourceNotFound(format!("Project {}", project_code)))
  }

  fn parse_event_time(&self, event: &ExchangeEvent) -> ServiceResult<DateTime<Tz>> {
    let tz: Tz = event
      .start
      .timezone
      .parse()
      .map_err(|e| ServiceError::parse_error(format!("Invalid timezone: {}", e)))?;

    let naive = NaiveDateTime::parse_from_str(&event.start.date_time, "%Y-%m-%dT%H:%M:%S%.f")
      .map_err(|e| ServiceError::parse_error(format!("Invalid datetime format: {}", e)))?;

    Ok(
      tz.from_local_datetime(&naive)
        .single()
        .ok_or_else(|| ServiceError::parse_error("Invalid timestamp"))?,
    )
  }

  async fn cleanup_old_tasks(&self) -> ServiceResult<()> {
    crate::mutation::tasks::delete_by_update_date(&self.pool).await?;
    Ok(())
  }
}

#[instrument(level = "debug")]
fn parse_options(raw_options: &str) -> ServiceResult<HashMap<String, String>> {
  let mut options = HashMap::new();

  for line in raw_options.lines().filter(|l| !l.is_empty()) {
    let parts: Vec<&str> = line.split(": ").collect();
    if parts.len() == 2 {
      options.insert(parts[0].trim().to_lowercase(), parts[1].trim().to_string());
    } else {
      warn!("Invalid option format: '{}'", line);
    }
  }

  Ok(options)
}

#[instrument(level = "debug")]
pub async fn run(pool: Arc<SqlitePool>, cancel_token: CancellationToken) -> anyhow::Result<()> {
  info!("Starting Exchange calendar poller");

  let server = env::var("EXCHANGE_URL").map_err(|_| ServiceError::env_var_missing("EXCHANGE_URL"))?;
  let username = env::var("EXCHANGE_LOGIN").map_err(|_| ServiceError::env_var_missing("EXCHANGE_LOGIN"))?;
  let password = env::var("EXCHANGE_PASSWORD").map_err(|_| ServiceError::env_var_missing("EXCHANGE_PASSWORD"))?;

  let mut client = ExchangeClient::new(server, username, password)?;
  let processor = EventProcessor::new(pool)?;

  while !cancel_token.is_cancelled() {
    tokio::select! {
      biased;
      _ = cancel_token.cancelled() => {
        info!("Exchange poller stopped");
        break;
      }
      _ = tokio::time::sleep(constants::QUERY_INTERVAL) => {
        let start = Local::now()
          .with_hour(0).unwrap()
          .with_minute(0).unwrap()
          .with_second(0).unwrap();
        let end = Local::now()
          .with_hour(23).unwrap()
          .with_minute(59).unwrap()
          .with_second(59).unwrap();

        match client.fetch_events(start.into(), end.into()).await {
          Ok(response) => {
            if response.value.len() == 0 {
              warn!("Not found events in exchange");
            }

            if let Err(e) = processor.process_events(response.value).await {
              error!("Failed to process events: {}", e);
            }
          }
          Err(e) => error!("Failed to fetch events: {}", e),
        }
      }
    }
  }

  Ok(())
}
