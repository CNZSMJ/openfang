//! Telegram Bot API adapter for the OpenFang channel bridge.
//!
//! Uses long-polling via `getUpdates` with exponential backoff on failures.
//! No external Telegram crate — just `reqwest` for full control over error handling.

use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelType, ChannelUser,
};
use async_trait::async_trait;
use futures::Stream;
use openfang_types::inbound::InboundAttachment;
use openfang_types::media::{MediaSource, MediaType};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tracing::{debug, info, warn};
use zeroize::Zeroizing;

/// Maximum backoff duration on API failures.
const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// Initial backoff duration on API failures.
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
/// Telegram long-polling timeout (seconds) — sent as the `timeout` parameter to getUpdates.
const LONG_POLL_TIMEOUT: u64 = 30;

/// Telegram Bot API adapter using long-polling.
pub struct TelegramAdapter {
    /// SECURITY: Bot token is zeroized on drop to prevent memory disclosure.
    token: Zeroizing<String>,
    client: reqwest::Client,
    allowed_users: Vec<i64>,
    allowed_chats: Vec<i64>,
    max_image_bytes: u64,
    staging_dir: PathBuf,
    poll_interval: Duration,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl TelegramAdapter {
    /// Create a new Telegram adapter.
    ///
    /// `token` is the raw bot token (read from env by the caller).
    /// `allowed_users` is the list of Telegram user IDs allowed to interact (empty = allow all).
    pub fn new(
        token: String,
        allowed_users: Vec<i64>,
        allowed_chats: Vec<i64>,
        poll_interval: Duration,
        max_image_bytes: u64,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            token: Zeroizing::new(token),
            client: reqwest::Client::new(),
            allowed_users,
            allowed_chats,
            max_image_bytes,
            staging_dir: std::env::temp_dir().join("openfang_attachment_staging/telegram"),
            poll_interval,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }

    /// Validate the bot token by calling `getMe`.
    pub async fn validate_token(&self) -> Result<String, Box<dyn std::error::Error>> {
        let url = format!("https://api.telegram.org/bot{}/getMe", self.token.as_str());
        let resp: serde_json::Value = self.client.get(&url).send().await?.json().await?;

        if resp["ok"].as_bool() != Some(true) {
            let desc = resp["description"].as_str().unwrap_or("unknown error");
            return Err(format!("Telegram getMe failed: {desc}").into());
        }

        let bot_name = resp["result"]["username"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();
        Ok(bot_name)
    }

    /// Call `sendMessage` on the Telegram API.
    async fn api_send_message(
        &self,
        chat_id: i64,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!(
            "https://api.telegram.org/bot{}/sendMessage",
            self.token.as_str()
        );

        // Sanitize: strip unsupported HTML tags so Telegram doesn't reject with 400.
        // Telegram only allows: b, i, u, s, tg-spoiler, a, code, pre, blockquote.
        // Any other tag (e.g. <name>, <thinking>) causes a 400 Bad Request.
        let sanitized = sanitize_telegram_html(text);

        // Telegram has a 4096 character limit per message — split if needed
        let chunks = split_message(&sanitized, 4096);
        for chunk in chunks {
            let body = serde_json::json!({
                "chat_id": chat_id,
                "text": chunk,
                "parse_mode": "HTML",
            });

            let resp = self.client.post(&url).json(&body).send().await?;
            let status = resp.status();
            if !status.is_success() {
                let body_text = resp.text().await.unwrap_or_default();
                warn!("Telegram sendMessage failed ({status}): {body_text}");
            }
        }
        Ok(())
    }

    /// Call `sendPhoto` on the Telegram API.
    async fn api_send_photo(
        &self,
        chat_id: i64,
        photo_url: &str,
        caption: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!(
            "https://api.telegram.org/bot{}/sendPhoto",
            self.token.as_str()
        );
        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "photo": photo_url,
        });
        if let Some(cap) = caption {
            body["caption"] = serde_json::Value::String(cap.to_string());
            body["parse_mode"] = serde_json::Value::String("HTML".to_string());
        }
        let resp = self.client.post(&url).json(&body).send().await?;
        if !resp.status().is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            warn!("Telegram sendPhoto failed: {body_text}");
        }
        Ok(())
    }

    /// Call `sendDocument` on the Telegram API.
    async fn api_send_document(
        &self,
        chat_id: i64,
        document_url: &str,
        filename: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!(
            "https://api.telegram.org/bot{}/sendDocument",
            self.token.as_str()
        );
        let body = serde_json::json!({
            "chat_id": chat_id,
            "document": document_url,
            "caption": filename,
        });
        let resp = self.client.post(&url).json(&body).send().await?;
        if !resp.status().is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            warn!("Telegram sendDocument failed: {body_text}");
        }
        Ok(())
    }

    /// Call `sendVoice` on the Telegram API.
    async fn api_send_voice(
        &self,
        chat_id: i64,
        voice_url: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!(
            "https://api.telegram.org/bot{}/sendVoice",
            self.token.as_str()
        );
        let body = serde_json::json!({
            "chat_id": chat_id,
            "voice": voice_url,
        });
        let resp = self.client.post(&url).json(&body).send().await?;
        if !resp.status().is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            warn!("Telegram sendVoice failed: {body_text}");
        }
        Ok(())
    }

    /// Call `sendLocation` on the Telegram API.
    async fn api_send_location(
        &self,
        chat_id: i64,
        lat: f64,
        lon: f64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!(
            "https://api.telegram.org/bot{}/sendLocation",
            self.token.as_str()
        );
        let body = serde_json::json!({
            "chat_id": chat_id,
            "latitude": lat,
            "longitude": lon,
        });
        let resp = self.client.post(&url).json(&body).send().await?;
        if !resp.status().is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            warn!("Telegram sendLocation failed: {body_text}");
        }
        Ok(())
    }

    /// Call `sendChatAction` to show "typing..." indicator.
    async fn api_send_typing(&self, chat_id: i64) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!(
            "https://api.telegram.org/bot{}/sendChatAction",
            self.token.as_str()
        );
        let body = serde_json::json!({
            "chat_id": chat_id,
            "action": "typing",
        });
        let _ = self.client.post(&url).json(&body).send().await?;
        Ok(())
    }
}

#[async_trait]
impl ChannelAdapter for TelegramAdapter {
    fn name(&self) -> &str {
        "telegram"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Telegram
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, Box<dyn std::error::Error>>
    {
        // Validate token first (fail fast)
        let bot_name = self.validate_token().await?;
        info!("Telegram bot @{bot_name} connected");

        // Clear any existing webhook to avoid 409 Conflict during getUpdates polling.
        // This is necessary when the daemon restarts — the old polling session may
        // still be active on Telegram's side for ~30s, causing 409 errors.
        {
            let delete_url = format!(
                "https://api.telegram.org/bot{}/deleteWebhook",
                self.token.as_str()
            );
            match self
                .client
                .post(&delete_url)
                .json(&serde_json::json!({"drop_pending_updates": true}))
                .send()
                .await
            {
                Ok(_) => info!("Telegram: cleared webhook, polling mode active"),
                Err(e) => tracing::warn!("Telegram: deleteWebhook failed (non-fatal): {e}"),
            }
        }

        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);

        let token = self.token.clone();
        let client = self.client.clone();
        let allowed_users = self.allowed_users.clone();
        let allowed_chats = self.allowed_chats.clone();
        let max_image_bytes = self.max_image_bytes;
        let staging_dir = self.staging_dir.clone();
        let poll_interval = self.poll_interval;
        let mut shutdown = self.shutdown_rx.clone();

        tokio::spawn(async move {
            let mut offset: Option<i64> = None;
            let mut backoff = INITIAL_BACKOFF;

            loop {
                // Check shutdown
                if *shutdown.borrow() {
                    break;
                }

                // Build getUpdates request
                let url = format!("https://api.telegram.org/bot{}/getUpdates", token.as_str());
                let mut params = serde_json::json!({
                    "timeout": LONG_POLL_TIMEOUT,
                    "allowed_updates": ["message", "edited_message", "channel_post", "edited_channel_post"],
                });
                if let Some(off) = offset {
                    params["offset"] = serde_json::json!(off);
                }

                // Make the request with a timeout slightly longer than the long-poll timeout
                let request_timeout = Duration::from_secs(LONG_POLL_TIMEOUT + 10);
                let result = tokio::select! {
                    res = async {
                        client
                            .get(&url)
                            .json(&params)
                            .timeout(request_timeout)
                            .send()
                            .await
                    } => res,
                    _ = shutdown.changed() => {
                        break;
                    }
                };

                let resp = match result {
                    Ok(resp) => resp,
                    Err(e) => {
                        warn!("Telegram getUpdates network error: {e}, retrying in {backoff:?}");
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(MAX_BACKOFF);
                        continue;
                    }
                };

                let status = resp.status();

                // Handle rate limiting
                if status.as_u16() == 429 {
                    let body: serde_json::Value = resp.json().await.unwrap_or_default();
                    let retry_after = body["parameters"]["retry_after"].as_u64().unwrap_or(5);
                    warn!("Telegram rate limited, retry after {retry_after}s");
                    tokio::time::sleep(Duration::from_secs(retry_after)).await;
                    continue;
                }

                // Handle conflict (another bot instance or stale session polling).
                // On daemon restart, the old long-poll may still be active on Telegram's
                // side for up to 30s. Retry with backoff instead of stopping permanently.
                if status.as_u16() == 409 {
                    warn!("Telegram 409 Conflict — stale polling session, retrying in {backoff:?}");
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(MAX_BACKOFF);
                    continue;
                }

                if !status.is_success() {
                    let body_text = resp.text().await.unwrap_or_default();
                    warn!("Telegram getUpdates failed ({status}): {body_text}, retrying in {backoff:?}");
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(MAX_BACKOFF);
                    continue;
                }

                // Parse response
                let body: serde_json::Value = match resp.json().await {
                    Ok(v) => v,
                    Err(e) => {
                        warn!("Telegram getUpdates parse error: {e}");
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(MAX_BACKOFF);
                        continue;
                    }
                };

                // Reset backoff on success
                backoff = INITIAL_BACKOFF;

                if body["ok"].as_bool() != Some(true) {
                    warn!("Telegram getUpdates returned ok=false");
                    tokio::time::sleep(poll_interval).await;
                    continue;
                }

                let updates = match body["result"].as_array() {
                    Some(arr) => arr,
                    None => {
                        tokio::time::sleep(poll_interval).await;
                        continue;
                    }
                };

                for update in updates {
                    // Track offset for dedup
                    if let Some(update_id) = update["update_id"].as_i64() {
                        offset = Some(update_id + 1);
                    }

                    // Parse the message
                    let msg = match parse_telegram_update(
                        update,
                        &client,
                        token.as_str(),
                        &allowed_users,
                        &allowed_chats,
                        max_image_bytes,
                        &staging_dir,
                    )
                    .await
                    {
                        Some(m) => m,
                        None => continue, // filtered out or unparseable
                    };

                    debug!(
                        "Telegram message from {}: {:?}",
                        msg.sender.display_name, msg.content
                    );

                    if tx.send(msg).await.is_err() {
                        // Receiver dropped — bridge is shutting down
                        return;
                    }
                }

                // Small delay between polls even on success to avoid tight loops
                tokio::time::sleep(poll_interval).await;
            }

            info!("Telegram polling loop stopped");
        });

        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }

    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let chat_id: i64 = user
            .platform_id
            .parse()
            .map_err(|_| format!("Invalid Telegram chat_id: {}", user.platform_id))?;

        match content {
            ChannelContent::Text(text) => {
                self.api_send_message(chat_id, &text).await?;
            }
            ChannelContent::Image { url, caption } => {
                self.api_send_photo(chat_id, &url, caption.as_deref())
                    .await?;
            }
            ChannelContent::File { url, filename } => {
                self.api_send_document(chat_id, &url, &filename).await?;
            }
            ChannelContent::Voice { url, .. } => {
                self.api_send_voice(chat_id, &url).await?;
            }
            ChannelContent::Location { lat, lon } => {
                self.api_send_location(chat_id, lat, lon).await?;
            }
            ChannelContent::Command { name, args } => {
                let text = format!("/{name} {}", args.join(" "));
                self.api_send_message(chat_id, text.trim()).await?;
            }
        }
        Ok(())
    }

    async fn send_typing(&self, user: &ChannelUser) -> Result<(), Box<dyn std::error::Error>> {
        let chat_id: i64 = user
            .platform_id
            .parse()
            .map_err(|_| format!("Invalid Telegram chat_id: {}", user.platform_id))?;
        self.api_send_typing(chat_id).await
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error>> {
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct ParsedTelegramMessage {
    raw_message: serde_json::Value,
    platform_message_id: String,
    sender: ChannelUser,
    content: ChannelContent,
    timestamp: chrono::DateTime<chrono::Utc>,
    is_group: bool,
    metadata: HashMap<String, serde_json::Value>,
}

/// Parse a Telegram update into metadata and optionally resolve image attachments.
async fn parse_telegram_update(
    update: &serde_json::Value,
    client: &reqwest::Client,
    token: &str,
    allowed_users: &[i64],
    allowed_chats: &[i64],
    max_image_bytes: u64,
    staging_dir: &Path,
) -> Option<ChannelMessage> {
    let parsed = parse_telegram_update_metadata(update, allowed_users, allowed_chats)?;
    let mut attachments = Vec::new();

    if let Some(photo_sizes) = parsed.raw_message["photo"].as_array() {
        if let Some(photo) = select_telegram_photo(photo_sizes, max_image_bytes) {
            match download_attachment_from_file_id(
                client,
                token,
                photo["file_id"].as_str()?,
                "image/jpeg",
                photo["file_size"].as_u64().unwrap_or(0),
                None,
                staging_dir,
            )
            .await
            {
                Ok(attachment) => attachments.push(attachment),
                Err(err) => {
                    warn!("Telegram photo download failed: {err}");
                    return None;
                }
            }
        } else if !photo_sizes.is_empty() {
            warn!("Telegram photo rejected: no size variant within configured limit");
            return None;
        }
    }

    if attachments.is_empty() {
        if let Some(document) = parsed.raw_message.get("document") {
            let mime_type = document["mime_type"].as_str().unwrap_or("");
            if mime_type.starts_with("image/") {
                match download_attachment_from_file_id(
                    client,
                    token,
                    document["file_id"].as_str()?,
                    mime_type,
                    document["file_size"].as_u64().unwrap_or(0),
                    document["file_name"].as_str(),
                    staging_dir,
                )
                .await
                {
                    Ok(attachment) => attachments.push(attachment),
                    Err(err) => {
                        warn!("Telegram document download failed: {err}");
                        return None;
                    }
                }
            }
        }
    }

    Some(ChannelMessage {
        channel: ChannelType::Telegram,
        platform_message_id: parsed.platform_message_id,
        sender: parsed.sender,
        content: parsed.content,
        target_agent: None,
        timestamp: parsed.timestamp,
        is_group: parsed.is_group,
        thread_id: None,
        attachments,
        metadata: parsed.metadata,
    })
}

/// Parse a Telegram update JSON into a message envelope, or `None` if filtered/unparseable.
fn parse_telegram_update_metadata(
    update: &serde_json::Value,
    allowed_users: &[i64],
    allowed_chats: &[i64],
) -> Option<ParsedTelegramMessage> {
    let (update_kind, message) = if let Some(message) = update.get("message") {
        ("message", message)
    } else if let Some(message) = update.get("edited_message") {
        ("edited_message", message)
    } else if let Some(message) = update.get("channel_post") {
        ("channel_post", message)
    } else if let Some(message) = update.get("edited_channel_post") {
        ("edited_channel_post", message)
    } else {
        return None;
    };

    let chat = message.get("chat")?;
    let chat_id = chat["id"].as_i64()?;
    if !allowed_chats.is_empty() && !allowed_chats.contains(&chat_id) {
        debug!("Telegram: ignoring message from unlisted chat {chat_id}");
        return None;
    }

    let from = message.get("from");
    let sender_chat = message.get("sender_chat");
    if let Some(user_id) = from.and_then(|from| from["id"].as_i64()) {
        if !allowed_users.is_empty() && !allowed_users.contains(&user_id) {
            debug!("Telegram: ignoring message from unlisted user {user_id}");
            return None;
        }
    } else if !allowed_users.is_empty() && allowed_chats.is_empty() {
        debug!("Telegram: rejecting channel post without allowed_chats override");
        return None;
    }

    let display_name = if let Some(sender_chat) = sender_chat {
        sender_chat["title"]
            .as_str()
            .or_else(|| chat["title"].as_str())
            .or_else(|| sender_chat["username"].as_str())
            .unwrap_or("Telegram Channel")
            .to_string()
    } else if let Some(from) = from {
        let first_name = from["first_name"].as_str().unwrap_or("Unknown");
        let last_name = from["last_name"].as_str().unwrap_or("");
        if last_name.is_empty() {
            first_name.to_string()
        } else {
            format!("{first_name} {last_name}")
        }
    } else {
        chat["title"]
            .as_str()
            .or_else(|| chat["username"].as_str())
            .unwrap_or("Telegram")
            .to_string()
    };

    let chat_type = chat["type"].as_str().unwrap_or("private");
    let is_group = matches!(chat_type, "group" | "supergroup" | "channel");
    let text = message["text"]
        .as_str()
        .or_else(|| message["caption"].as_str())
        .unwrap_or("")
        .to_string();
    let has_image_attachment = message["photo"].is_array()
        || message
            .get("document")
            .and_then(|doc| doc["mime_type"].as_str())
            .map(|mime| mime.starts_with("image/"))
            .unwrap_or(false);
    if text.is_empty() && !has_image_attachment {
        return None;
    }

    let message_id = message["message_id"].as_i64().unwrap_or(0);
    let timestamp = message["edit_date"]
        .as_i64()
        .or_else(|| message["date"].as_i64())
        .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
        .unwrap_or_else(chrono::Utc::now);

    let entities = message
        .get("entities")
        .and_then(|entities| entities.as_array())
        .or_else(|| {
            message
                .get("caption_entities")
                .and_then(|entities| entities.as_array())
        });
    let content = if let Some(entities) = entities {
        let is_bot_command = entities
            .iter()
            .any(|e| e["type"].as_str() == Some("bot_command") && e["offset"].as_i64() == Some(0));
        if is_bot_command {
            let parts: Vec<&str> = text.splitn(2, ' ').collect();
            let cmd_name = parts[0].trim_start_matches('/');
            // Strip @botname from command (e.g. /agents@mybot -> agents)
            let cmd_name = cmd_name.split('@').next().unwrap_or(cmd_name);
            let args = if parts.len() > 1 {
                parts[1].split_whitespace().map(String::from).collect()
            } else {
                vec![]
            };
            ChannelContent::Command {
                name: cmd_name.to_string(),
                args,
            }
        } else {
            ChannelContent::Text(text.to_string())
        }
    } else {
        ChannelContent::Text(text.to_string())
    };

    let mut metadata = HashMap::new();
    metadata.insert(
        "telegram_update_kind".to_string(),
        serde_json::Value::String(update_kind.to_string()),
    );
    metadata.insert("telegram_chat_id".to_string(), serde_json::json!(chat_id));
    if let Some(user_id) = from.and_then(|from| from["id"].as_i64()) {
        metadata.insert("telegram_user_id".to_string(), serde_json::json!(user_id));
    }
    if let Some(media_group_id) = message["media_group_id"].as_str() {
        metadata.insert(
            "telegram_media_group_id".to_string(),
            serde_json::Value::String(media_group_id.to_string()),
        );
    }
    if matches!(update_kind, "edited_message" | "edited_channel_post") {
        metadata.insert("edited".to_string(), serde_json::Value::Bool(true));
    }

    Some(ParsedTelegramMessage {
        raw_message: message.clone(),
        platform_message_id: message_id.to_string(),
        sender: ChannelUser {
            platform_id: chat_id.to_string(),
            display_name,
            openfang_user: None,
        },
        content,
        timestamp,
        is_group,
        metadata,
    })
}

fn select_telegram_photo<'a>(
    photo_sizes: &'a [serde_json::Value],
    max_image_bytes: u64,
) -> Option<&'a serde_json::Value> {
    photo_sizes.iter().rev().find(|photo| {
        photo["file_size"]
            .as_u64()
            .map(|size| size <= max_image_bytes)
            .unwrap_or(true)
    })
}

async fn download_attachment_from_file_id(
    client: &reqwest::Client,
    token: &str,
    file_id: &str,
    mime_type: &str,
    declared_size: u64,
    filename: Option<&str>,
    staging_dir: &Path,
) -> Result<InboundAttachment, String> {
    let file_path = fetch_telegram_file_path(client, token, file_id).await?;
    tokio::fs::create_dir_all(staging_dir)
        .await
        .map_err(|e| format!("failed to create staging dir: {e}"))?;

    let extension = Path::new(&file_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(String::from)
        .or_else(|| image_extension_for_mime(mime_type).map(String::from));
    let staged_name = if let Some(ext) = extension {
        format!("{}.{}", uuid::Uuid::new_v4(), ext)
    } else {
        uuid::Uuid::new_v4().to_string()
    };
    let staged_path = staging_dir.join(staged_name);

    let url = format!("https://api.telegram.org/file/bot{token}/{file_path}");
    let bytes = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("download request failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("download request failed: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("failed to read download bytes: {e}"))?;

    tokio::fs::write(&staged_path, &bytes)
        .await
        .map_err(|e| format!("failed to write staged file: {e}"))?;

    let mut metadata = HashMap::new();
    metadata.insert(
        "telegram_file_id".to_string(),
        serde_json::Value::String(file_id.to_string()),
    );

    Ok(InboundAttachment {
        kind: MediaType::Image,
        mime_type: mime_type.to_string(),
        filename: filename.map(String::from),
        source: MediaSource::FilePath {
            path: staged_path.display().to_string(),
        },
        size_bytes: declared_size.max(bytes.len() as u64),
        metadata,
    })
}

async fn fetch_telegram_file_path(
    client: &reqwest::Client,
    token: &str,
    file_id: &str,
) -> Result<String, String> {
    let url = format!("https://api.telegram.org/bot{token}/getFile");
    let body = serde_json::json!({ "file_id": file_id });
    let response: serde_json::Value = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("getFile request failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("getFile request failed: {e}"))?
        .json()
        .await
        .map_err(|e| format!("invalid getFile response: {e}"))?;

    if response["ok"].as_bool() != Some(true) {
        let description = response["description"]
            .as_str()
            .unwrap_or("unknown Telegram getFile error");
        return Err(description.to_string());
    }

    response["result"]["file_path"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| "Telegram getFile response missing file_path".to_string())
}

fn image_extension_for_mime(mime_type: &str) -> Option<&'static str> {
    match mime_type {
        "image/jpeg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        _ => None,
    }
}

/// Calculate exponential backoff capped at MAX_BACKOFF.
pub fn calculate_backoff(current: Duration) -> Duration {
    (current * 2).min(MAX_BACKOFF)
}

/// Sanitize text for Telegram HTML parse mode.
///
/// Escapes angle brackets that are NOT part of Telegram-allowed HTML tags.
/// Allowed tags: b, i, u, s, tg-spoiler, a, code, pre, blockquote.
/// Everything else (e.g. `<name>`, `<thinking>`) gets escaped to `&lt;...&gt;`.
fn sanitize_telegram_html(text: &str) -> String {
    const ALLOWED: &[&str] = &[
        "b", "i", "u", "s", "em", "strong", "a", "code", "pre", "blockquote", "tg-spoiler",
        "tg-emoji",
    ];

    let mut result = String::with_capacity(text.len());
    let mut chars = text.char_indices().peekable();

    while let Some(&(i, ch)) = chars.peek() {
        if ch == '<' {
            // Try to parse an HTML tag
            if let Some(end_offset) = text[i..].find('>') {
                let tag_end = i + end_offset;
                let tag_content = &text[i + 1..tag_end]; // content between < and >
                let tag_name = tag_content
                    .trim_start_matches('/')
                    .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
                    .next()
                    .unwrap_or("")
                    .to_lowercase();

                if !tag_name.is_empty() && ALLOWED.contains(&tag_name.as_str()) {
                    // Allowed tag — keep as-is
                    result.push_str(&text[i..tag_end + 1]);
                } else {
                    // Unknown tag — escape both brackets
                    result.push_str("&lt;");
                    result.push_str(tag_content);
                    result.push_str("&gt;");
                }
                // Advance past the whole tag
                while let Some(&(j, _)) = chars.peek() {
                    chars.next();
                    if j >= tag_end {
                        break;
                    }
                }
            } else {
                // No closing > — escape the lone <
                result.push_str("&lt;");
                chars.next();
            }
        } else {
            result.push(ch);
            chars.next();
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_telegram_update() {
        let update = serde_json::json!({
            "update_id": 123456,
            "message": {
                "message_id": 42,
                "from": {
                    "id": 111222333,
                    "first_name": "Alice",
                    "last_name": "Smith"
                },
                "chat": {
                    "id": 111222333,
                    "type": "private"
                },
                "date": 1700000000,
                "text": "Hello, agent!"
            }
        });

        let msg = parse_telegram_update_metadata(&update, &[], &[]).unwrap();
        assert_eq!(msg.sender.display_name, "Alice Smith");
        assert_eq!(msg.sender.platform_id, "111222333");
        assert!(matches!(msg.content, ChannelContent::Text(ref t) if t == "Hello, agent!"));
    }

    #[test]
    fn test_parse_telegram_command() {
        let update = serde_json::json!({
            "update_id": 123457,
            "message": {
                "message_id": 43,
                "from": {
                    "id": 111222333,
                    "first_name": "Alice"
                },
                "chat": {
                    "id": 111222333,
                    "type": "private"
                },
                "date": 1700000001,
                "text": "/agent hello-world",
                "entities": [{
                    "type": "bot_command",
                    "offset": 0,
                    "length": 6
                }]
            }
        });

        let msg = parse_telegram_update_metadata(&update, &[], &[]).unwrap();
        match &msg.content {
            ChannelContent::Command { name, args } => {
                assert_eq!(name, "agent");
                assert_eq!(args, &["hello-world"]);
            }
            other => panic!("Expected Command, got {other:?}"),
        }
    }

    #[test]
    fn test_allowed_users_filter() {
        let update = serde_json::json!({
            "update_id": 123458,
            "message": {
                "message_id": 44,
                "from": {
                    "id": 999,
                    "first_name": "Bob"
                },
                "chat": {
                    "id": 999,
                    "type": "private"
                },
                "date": 1700000002,
                "text": "blocked"
            }
        });

        // Empty allowed_users = allow all
        let msg = parse_telegram_update_metadata(&update, &[], &[]);
        assert!(msg.is_some());

        // Non-matching allowed_users = filter out
        let msg = parse_telegram_update_metadata(&update, &[111, 222], &[]);
        assert!(msg.is_none());

        // Matching allowed_users = allow
        let msg = parse_telegram_update_metadata(&update, &[999], &[]);
        assert!(msg.is_some());
    }

    #[test]
    fn test_parse_telegram_edited_message() {
        let update = serde_json::json!({
            "update_id": 123459,
            "edited_message": {
                "message_id": 42,
                "from": {
                    "id": 111222333,
                    "first_name": "Alice",
                    "last_name": "Smith"
                },
                "chat": {
                    "id": 111222333,
                    "type": "private"
                },
                "date": 1700000000,
                "edit_date": 1700000060,
                "text": "Edited message!"
            }
        });

        let msg = parse_telegram_update_metadata(&update, &[], &[]).unwrap();
        assert_eq!(msg.sender.display_name, "Alice Smith");
        assert!(matches!(msg.content, ChannelContent::Text(ref t) if t == "Edited message!"));
        assert_eq!(msg.metadata.get("edited"), Some(&serde_json::Value::Bool(true)));
    }

    #[test]
    fn test_backoff_calculation() {
        let b1 = calculate_backoff(Duration::from_secs(1));
        assert_eq!(b1, Duration::from_secs(2));

        let b2 = calculate_backoff(Duration::from_secs(2));
        assert_eq!(b2, Duration::from_secs(4));

        let b3 = calculate_backoff(Duration::from_secs(32));
        assert_eq!(b3, Duration::from_secs(60)); // capped

        let b4 = calculate_backoff(Duration::from_secs(60));
        assert_eq!(b4, Duration::from_secs(60)); // stays at cap
    }

    #[test]
    fn test_parse_command_with_botname() {
        let update = serde_json::json!({
            "update_id": 100,
            "message": {
                "message_id": 1,
                "from": { "id": 123, "first_name": "X" },
                "chat": { "id": 123, "type": "private" },
                "date": 1700000000,
                "text": "/agents@myopenfangbot",
                "entities": [{ "type": "bot_command", "offset": 0, "length": 17 }]
            }
        });

        let msg = parse_telegram_update_metadata(&update, &[], &[]).unwrap();
        match &msg.content {
            ChannelContent::Command { name, args } => {
                assert_eq!(name, "agents");
                assert!(args.is_empty());
            }
            other => panic!("Expected Command, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_channel_post_with_caption_and_media_group() {
        let update = serde_json::json!({
            "update_id": 123460,
            "channel_post": {
                "message_id": 99,
                "sender_chat": {
                    "id": -1001234567890i64,
                    "title": "Release Channel"
                },
                "chat": {
                    "id": -1001234567890i64,
                    "type": "channel",
                    "title": "Release Channel"
                },
                "date": 1700000100,
                "caption": "Check this image",
                "media_group_id": "album-1",
                "photo": [{ "file_id": "small" }]
            }
        });

        let msg = parse_telegram_update_metadata(&update, &[], &[-1001234567890]).unwrap();
        assert_eq!(msg.sender.platform_id, "-1001234567890");
        assert_eq!(msg.sender.display_name, "Release Channel");
        assert!(matches!(msg.content, ChannelContent::Text(ref t) if t == "Check this image"));
        assert_eq!(
            msg.metadata.get("telegram_media_group_id"),
            Some(&serde_json::Value::String("album-1".to_string()))
        );
    }

    #[test]
    fn test_select_telegram_photo_prefers_largest_within_limit() {
        let photos = vec![
            serde_json::json!({ "file_id": "a", "file_size": 1024 }),
            serde_json::json!({ "file_id": "b", "file_size": 4096 }),
            serde_json::json!({ "file_id": "c", "file_size": 16384 }),
        ];

        let selected = select_telegram_photo(&photos, 5000).unwrap();
        assert_eq!(selected["file_id"].as_str(), Some("b"));

        let selected = select_telegram_photo(&photos, 1000);
        assert!(selected.is_none());
    }
}
