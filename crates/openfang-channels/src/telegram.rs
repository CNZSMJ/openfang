//! Telegram Bot API adapter for the OpenFang channel bridge.
//!
//! Uses long-polling via `getUpdates` with exponential backoff on failures.
//! No external Telegram crate — just `reqwest` for full control over error handling.

use crate::log_sanitize::sanitize_channel_error_for_log;
use crate::types::{ChannelAdapter, ChannelContent, ChannelMessage, ChannelType, ChannelUser};
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
/// Default Telegram Bot API base URL.
const DEFAULT_API_URL: &str = "https://api.telegram.org";
const TELEGRAM_MESSAGE_LIMIT: usize = 4096;
const TELEGRAM_CAPTION_LIMIT: usize = 1024;

/// Telegram Bot API adapter using long-polling.
pub struct TelegramAdapter {
    /// SECURITY: Bot token is zeroized on drop to prevent memory disclosure.
    token: Zeroizing<String>,
    client: reqwest::Client,
    allowed_users: Vec<String>,
    allowed_chats: Vec<String>,
    max_image_bytes: u64,
    staging_dir: PathBuf,
    poll_interval: Duration,
    /// Base URL for Telegram Bot API (supports proxies/mirrors).
    api_base_url: String,
    /// Bot username (without @), populated from `getMe` during `start()`.
    bot_username: Arc<tokio::sync::RwLock<Option<String>>>,
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
        allowed_users: Vec<String>,
        allowed_chats: Vec<String>,
        poll_interval: Duration,
        api_url: Option<String>,
        max_image_bytes: u64,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let api_base_url = api_url
            .unwrap_or_else(|| DEFAULT_API_URL.to_string())
            .trim_end_matches('/')
            .to_string();
        Self {
            token: Zeroizing::new(token),
            client: reqwest::Client::new(),
            allowed_users,
            allowed_chats,
            max_image_bytes,
            staging_dir: std::env::temp_dir().join("openfang_attachment_staging/telegram"),
            poll_interval,
            api_base_url,
            bot_username: Arc::new(tokio::sync::RwLock::new(None)),
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }

    /// Validate the bot token by calling `getMe`.
    pub async fn validate_token(&self) -> Result<String, Box<dyn std::error::Error>> {
        let url = format!("{}/bot{}/getMe", self.api_base_url, self.token.as_str());
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
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!("{}/bot{}/sendMessage", self.api_base_url, self.token.as_str());
        for chunk in split_telegram_html_chunks(text, TELEGRAM_MESSAGE_LIMIT) {
            let mut body = serde_json::json!({
                "chat_id": chat_id,
                "text": chunk,
                "parse_mode": "HTML",
            });
            if let Some(tid) = thread_id {
                body["message_thread_id"] = serde_json::json!(tid);
            }

            let resp = self.client.post(&url).json(&body).send().await?;
            let status = resp.status();
            if !status.is_success() {
                let body_text = resp.text().await.unwrap_or_default();
                let error = format!("Telegram sendMessage failed ({status}): {body_text}");
                warn!("{error}");
                return Err(error.into());
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
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!("{}/bot{}/sendPhoto", self.api_base_url, self.token.as_str());
        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "photo": photo_url,
        });
        let extra_caption_chunks = if let Some(cap) = caption {
            let mut chunks = split_telegram_html_chunks(cap, TELEGRAM_CAPTION_LIMIT);
            if let Some(first) = chunks.first() {
                body["caption"] = serde_json::Value::String(first.clone());
                body["parse_mode"] = serde_json::Value::String("HTML".to_string());
            }
            chunks.drain(1..).collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        if body.get("caption").is_some() {
            body["parse_mode"] = serde_json::Value::String("HTML".to_string());
        }
        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::json!(tid);
        }
        let resp = self.client.post(&url).json(&body).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            let error = format!("Telegram sendPhoto failed ({status}): {body_text}");
            warn!("{error}");
            return Err(error.into());
        }
        for chunk in extra_caption_chunks {
            self.api_send_message(chat_id, &chunk, thread_id).await?;
        }
        Ok(())
    }

    /// Call `sendDocument` on the Telegram API.
    async fn api_send_document(
        &self,
        chat_id: i64,
        document_url: &str,
        filename: &str,
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!(
            "{}/bot{}/sendDocument",
            self.api_base_url,
            self.token.as_str()
        );
        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "document": document_url,
            "caption": filename,
        });
        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::json!(tid);
        }
        let resp = self.client.post(&url).json(&body).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            let error = format!("Telegram sendDocument failed ({status}): {body_text}");
            warn!("{error}");
            return Err(error.into());
        }
        Ok(())
    }

    async fn api_send_document_upload(
        &self,
        chat_id: i64,
        data: Vec<u8>,
        filename: &str,
        mime_type: &str,
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!(
            "{}/bot{}/sendDocument",
            self.api_base_url,
            self.token.as_str()
        );
        let file_part = reqwest::multipart::Part::bytes(data)
            .file_name(filename.to_string())
            .mime_str(mime_type)?;
        let mut form = reqwest::multipart::Form::new()
            .text("chat_id", chat_id.to_string())
            .part("document", file_part);
        if let Some(tid) = thread_id {
            form = form.text("message_thread_id", tid.to_string());
        }
        let resp = self.client.post(&url).multipart(form).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            let error = format!("Telegram sendDocument upload failed ({status}): {body_text}");
            warn!("{error}");
            return Err(error.into());
        }
        Ok(())
    }

    /// Call `sendVoice` on the Telegram API.
    async fn api_send_voice(
        &self,
        chat_id: i64,
        voice_url: &str,
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!("{}/bot{}/sendVoice", self.api_base_url, self.token.as_str());
        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "voice": voice_url,
        });
        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::json!(tid);
        }
        let resp = self.client.post(&url).json(&body).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            let error = format!("Telegram sendVoice failed ({status}): {body_text}");
            warn!("{error}");
            return Err(error.into());
        }
        Ok(())
    }

    /// Call `sendLocation` on the Telegram API.
    async fn api_send_location(
        &self,
        chat_id: i64,
        lat: f64,
        lon: f64,
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!(
            "{}/bot{}/sendLocation",
            self.api_base_url,
            self.token.as_str()
        );
        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "latitude": lat,
            "longitude": lon,
        });
        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::json!(tid);
        }
        let resp = self.client.post(&url).json(&body).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            let error = format!("Telegram sendLocation failed ({status}): {body_text}");
            warn!("{error}");
            return Err(error.into());
        }
        Ok(())
    }

    /// Call `sendChatAction` to show "typing..." indicator.
    async fn api_send_typing(
        &self,
        chat_id: i64,
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!(
            "{}/bot{}/sendChatAction",
            self.api_base_url,
            self.token.as_str()
        );
        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "action": "typing",
        });
        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::json!(tid);
        }
        self.client.post(&url).json(&body).send().await?;
        Ok(())
    }

    async fn send_content(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
        thread_id: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let chat_id: i64 = user
            .platform_id
            .parse()
            .map_err(|_| format!("Invalid Telegram chat_id: {}", user.platform_id))?;

        match content {
            ChannelContent::Text(text) => {
                self.api_send_message(chat_id, &text, thread_id).await?;
            }
            ChannelContent::Image { url, caption } => {
                self.api_send_photo(chat_id, &url, caption.as_deref(), thread_id)
                    .await?;
            }
            ChannelContent::File { url, filename } => {
                self.api_send_document(chat_id, &url, &filename, thread_id)
                    .await?;
            }
            ChannelContent::FileData {
                data,
                filename,
                mime_type,
            } => {
                self.api_send_document_upload(chat_id, data, &filename, &mime_type, thread_id)
                    .await?;
            }
            ChannelContent::Voice { url, .. } => {
                self.api_send_voice(chat_id, &url, thread_id).await?;
            }
            ChannelContent::Location { lat, lon } => {
                self.api_send_location(chat_id, lat, lon, thread_id).await?;
            }
            ChannelContent::Command { name, args } => {
                let text = format!("/{name} {}", args.join(" "));
                self.api_send_message(chat_id, text.trim(), thread_id).await?;
            }
        }
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
        {
            let mut username = self.bot_username.write().await;
            *username = Some(bot_name.clone());
        }

        // Clear any existing webhook to avoid 409 Conflict during getUpdates polling.
        // This is necessary when the daemon restarts — the old polling session may
        // still be active on Telegram's side for ~30s, causing 409 errors.
        {
            let delete_url = format!(
                "{}/bot{}/deleteWebhook",
                self.api_base_url,
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
                Err(e) => {
                    let safe = sanitize_channel_error_for_log(&e.to_string(), &[self.token.as_str()]);
                    tracing::warn!("Telegram: deleteWebhook failed (non-fatal): {safe}");
                }
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
        let api_base_url = self.api_base_url.clone();
        let bot_username = self.bot_username.clone();
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
                let url = format!("{}/bot{}/getUpdates", api_base_url, token.as_str());
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
                        let safe = sanitize_channel_error_for_log(&e.to_string(), &[token.as_str()]);
                        warn!("Telegram getUpdates network error: {safe}, retrying in {backoff:?}");
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
                        let safe = sanitize_channel_error_for_log(&e.to_string(), &[token.as_str()]);
                        warn!("Telegram getUpdates parse error: {safe}");
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
                    let bot_uname = bot_username.read().await.clone();
                    let msg = match parse_telegram_update(
                        update,
                        &client,
                        token.as_str(),
                        &allowed_users,
                        &allowed_chats,
                        max_image_bytes,
                        &staging_dir,
                        &api_base_url,
                        bot_uname.as_deref(),
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
        self.send_content(user, content, None).await
    }

    async fn send_in_thread(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
        thread_id: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let tid = thread_id.parse::<i64>().ok();
        self.send_content(user, content, tid).await
    }

    async fn send_typing(&self, user: &ChannelUser) -> Result<(), Box<dyn std::error::Error>> {
        let chat_id: i64 = user
            .platform_id
            .parse()
            .map_err(|_| format!("Invalid Telegram chat_id: {}", user.platform_id))?;
        self.api_send_typing(chat_id, None).await
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
    thread_id: Option<String>,
    metadata: HashMap<String, serde_json::Value>,
}

/// Parse a Telegram update into metadata and optionally resolve image attachments.
#[allow(clippy::too_many_arguments)]
async fn parse_telegram_update(
    update: &serde_json::Value,
    client: &reqwest::Client,
    token: &str,
    allowed_users: &[String],
    allowed_chats: &[String],
    max_image_bytes: u64,
    staging_dir: &Path,
    api_base_url: &str,
    bot_username: Option<&str>,
) -> Option<ChannelMessage> {
    let parsed =
        parse_telegram_update_metadata(update, allowed_users, allowed_chats, bot_username)?;
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
                api_base_url,
            )
            .await
            {
                Ok(attachment) => attachments.push(attachment),
                Err(err) => {
                    let safe = sanitize_channel_error_for_log(&err.to_string(), &[token]);
                    warn!("Telegram photo download failed: {safe}");
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
                    api_base_url,
                )
                .await
                {
                    Ok(attachment) => attachments.push(attachment),
                    Err(err) => {
                        let safe = sanitize_channel_error_for_log(&err.to_string(), &[token]);
                        warn!("Telegram document download failed: {safe}");
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
        thread_id: parsed.thread_id,
        attachments,
        metadata: parsed.metadata,
    })
}

/// Parse a Telegram update JSON into a message envelope, or `None` if filtered/unparseable.
fn parse_telegram_update_metadata(
    update: &serde_json::Value,
    allowed_users: &[String],
    allowed_chats: &[String],
    bot_username: Option<&str>,
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
    let chat_id_str = chat_id.to_string();
    if !allowed_chats.is_empty() && !allowed_chats.iter().any(|id| id == &chat_id_str) {
        debug!("Telegram: ignoring message from unlisted chat {chat_id}");
        return None;
    }

    let from = message.get("from");
    let sender_chat = message.get("sender_chat");
    if let Some(user_id) = from.and_then(|from| from["id"].as_i64()) {
        let user_id_str = user_id.to_string();
        if !allowed_users.is_empty() && !allowed_users.iter().any(|id| id == &user_id_str) {
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
    } else if has_image_attachment {
        if text.is_empty() {
            ChannelContent::Text("[User sent a photo]".to_string())
        } else {
            ChannelContent::Text(text.clone())
        }
    } else if let Some(document) = message.get("document") {
        let filename = document["file_name"]
            .as_str()
            .unwrap_or("document")
            .to_string();
        ChannelContent::Text(format!("[User sent a file ({filename})]"))
    } else if let Some(voice) = message.get("voice") {
        let duration = voice["duration"].as_u64().unwrap_or(0);
        ChannelContent::Text(format!("[User sent a voice message ({duration}s)]"))
    } else if let Some(location) = message.get("location") {
        let lat = location["latitude"].as_f64().unwrap_or(0.0);
        let lon = location["longitude"].as_f64().unwrap_or(0.0);
        ChannelContent::Location { lat, lon }
    } else if !text.is_empty() {
        ChannelContent::Text(text.to_string())
    } else {
        return None;
    };

    let content = if let Some(reply_msg) = message.get("reply_to_message") {
        let reply_text = reply_msg["text"]
            .as_str()
            .or_else(|| reply_msg["caption"].as_str());
        let reply_sender = reply_msg["from"]["first_name"].as_str();

        if let Some(quoted_text) = reply_text {
            let sender_label = reply_sender.unwrap_or("Unknown");
            let prefix = format!("[Replying to {sender_label}: {quoted_text}]\n\n");
            match content {
                ChannelContent::Text(t) => ChannelContent::Text(format!("{prefix}{t}")),
                ChannelContent::Command { name, args } => {
                    let mut new_args = vec![format!("{prefix}{}", args.join(" "))];
                    new_args.retain(|arg| !arg.trim().is_empty());
                    ChannelContent::Command {
                        name,
                        args: new_args,
                    }
                }
                other => other,
            }
        } else {
            content
        }
    } else {
        content
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
    if let Some(reply_msg) = message.get("reply_to_message") {
        if let Some(reply_id) = reply_msg["message_id"].as_i64() {
            metadata.insert("reply_to_message_id".to_string(), serde_json::json!(reply_id));
        }
    }
    if is_group {
        if let Some(bot_uname) = bot_username {
            if check_mention_entities(message, bot_uname) {
                metadata.insert("was_mentioned".to_string(), serde_json::json!(true));
            }
        }
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
        thread_id: message["message_thread_id"]
            .as_i64()
            .map(|thread_id| thread_id.to_string()),
        metadata,
    })
}

fn select_telegram_photo(
    photo_sizes: &[serde_json::Value],
    max_image_bytes: u64,
) -> Option<&serde_json::Value> {
    photo_sizes.iter().rev().find(|photo| {
        photo["file_size"]
            .as_u64()
            .map(|size| size <= max_image_bytes)
            .unwrap_or(true)
    })
}

#[allow(clippy::too_many_arguments)]
async fn download_attachment_from_file_id(
    client: &reqwest::Client,
    token: &str,
    file_id: &str,
    mime_type: &str,
    declared_size: u64,
    filename: Option<&str>,
    staging_dir: &Path,
    api_base_url: &str,
) -> Result<InboundAttachment, String> {
    let file_path = fetch_telegram_file_path(client, token, file_id, api_base_url).await?;
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

    let url = format!("{api_base_url}/file/bot{token}/{file_path}");
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
    api_base_url: &str,
) -> Result<String, String> {
    let url = format!("{api_base_url}/bot{token}/getFile");
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

fn check_mention_entities(message: &serde_json::Value, bot_username: &str) -> bool {
    let bot_mention = format!("@{}", bot_username.to_lowercase());
    for entities_key in &["entities", "caption_entities"] {
        if let Some(entities) = message[*entities_key].as_array() {
            let text = if *entities_key == "entities" {
                message["text"].as_str().unwrap_or("")
            } else {
                message["caption"].as_str().unwrap_or("")
            };
            for entity in entities {
                if entity["type"].as_str() != Some("mention") {
                    continue;
                }
                let offset = entity["offset"].as_i64().unwrap_or(0) as usize;
                let length = entity["length"].as_i64().unwrap_or(0) as usize;
                if offset + length <= text.len()
                    && text[offset..offset + length].to_lowercase() == bot_mention
                {
                    return true;
                }
            }
        }
    }
    false
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

#[derive(Clone)]
struct TelegramOpenTag {
    name: String,
    raw: String,
    close: String,
}

enum TelegramHtmlToken<'a> {
    Text(&'a str),
    Tag {
        raw: &'a str,
        name: &'a str,
        is_closing: bool,
        is_self_closing: bool,
    },
}

fn split_telegram_html_chunks(text: &str, max_len: usize) -> Vec<String> {
    let sanitized = sanitize_telegram_html(text);
    if sanitized.is_empty() {
        return vec![String::new()];
    }
    if max_len == 0 {
        return vec![sanitized];
    }
    if sanitized.len() <= max_len {
        return vec![sanitized];
    }

    let tokens = tokenize_telegram_html(&sanitized);
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut open_tags: Vec<TelegramOpenTag> = Vec::new();
    let mut pending_reopen = String::new();

    for token in tokens {
        match token {
            TelegramHtmlToken::Text(segment) => {
                if current.is_empty() && !pending_reopen.is_empty() {
                    current.push_str(&pending_reopen);
                    pending_reopen.clear();
                }
                let mut remaining = segment;
                while !remaining.is_empty() {
                    let available = max_len.saturating_sub(current.len() + closing_tags_len(&open_tags));
                    if available == 0 {
                        flush_telegram_chunk(&mut chunks, &mut current, &open_tags, &mut pending_reopen);
                        continue;
                    }
                    if remaining.len() <= available {
                        current.push_str(remaining);
                        break;
                    }
                    let safe_end = openfang_types::truncate_str(remaining, available).len();
                    let split_at = remaining[..safe_end].rfind('\n').filter(|idx| *idx > 0).unwrap_or(safe_end);
                    current.push_str(&remaining[..split_at]);
                    flush_telegram_chunk(&mut chunks, &mut current, &open_tags, &mut pending_reopen);
                    remaining = remaining[split_at..]
                        .strip_prefix("\r\n")
                        .or_else(|| remaining[split_at..].strip_prefix('\n'))
                        .unwrap_or(&remaining[split_at..]);
                    if current.is_empty() && !pending_reopen.is_empty() && !remaining.is_empty() {
                        current.push_str(&pending_reopen);
                        pending_reopen.clear();
                    }
                }
            }
            TelegramHtmlToken::Tag {
                raw,
                name,
                is_closing,
                is_self_closing,
            } => {
                if is_closing && current.is_empty() && !pending_reopen.is_empty() {
                    if let Some(pos) = open_tags
                        .iter()
                        .rposition(|tag| tag.name.eq_ignore_ascii_case(name))
                    {
                        open_tags.truncate(pos);
                        pending_reopen = open_tags.iter().map(|tag| tag.raw.as_str()).collect();
                        continue;
                    }
                }
                if current.is_empty() && !pending_reopen.is_empty() {
                    current.push_str(&pending_reopen);
                    pending_reopen.clear();
                }
                let extra_closing_len = if is_closing || is_self_closing {
                    0
                } else {
                    name.len() + 3
                };
                if !current.is_empty()
                    && current.len() + raw.len() + closing_tags_len(&open_tags) + extra_closing_len > max_len
                {
                    flush_telegram_chunk(&mut chunks, &mut current, &open_tags, &mut pending_reopen);
                    if current.is_empty() && !pending_reopen.is_empty() && !is_closing {
                        current.push_str(&pending_reopen);
                        pending_reopen.clear();
                    }
                }

                current.push_str(raw);
                if is_self_closing {
                    continue;
                }
                if is_closing {
                    if let Some(pos) = open_tags
                        .iter()
                        .rposition(|tag| tag.name.eq_ignore_ascii_case(name))
                    {
                        open_tags.truncate(pos);
                    }
                } else {
                    open_tags.push(TelegramOpenTag {
                        name: name.to_ascii_lowercase(),
                        raw: raw.to_string(),
                        close: format!("</{name}>"),
                    });
                }
            }
        }
    }

    if !current.is_empty() {
        if is_closing_only_chunk(&current) {
            return chunks;
        }
        let mut final_chunk = current;
        for tag in open_tags.iter().rev() {
            final_chunk.push_str(&tag.close);
        }
        chunks.push(final_chunk);
    }

    chunks
}

fn tokenize_telegram_html(text: &str) -> Vec<TelegramHtmlToken<'_>> {
    let mut tokens = Vec::new();
    let mut cursor = 0;

    while let Some(start) = text[cursor..].find('<') {
        let tag_start = cursor + start;
        if tag_start > cursor {
            tokens.push(TelegramHtmlToken::Text(&text[cursor..tag_start]));
        }
        if let Some(end_offset) = text[tag_start..].find('>') {
            let tag_end = tag_start + end_offset + 1;
            let raw = &text[tag_start..tag_end];
            if let Some((name, is_closing, is_self_closing)) = parse_telegram_tag(raw) {
                tokens.push(TelegramHtmlToken::Tag {
                    raw,
                    name,
                    is_closing,
                    is_self_closing,
                });
            } else {
                tokens.push(TelegramHtmlToken::Text(raw));
            }
            cursor = tag_end;
        } else {
            tokens.push(TelegramHtmlToken::Text(&text[tag_start..]));
            cursor = text.len();
            break;
        }
    }

    if cursor < text.len() {
        tokens.push(TelegramHtmlToken::Text(&text[cursor..]));
    }

    tokens
}

fn parse_telegram_tag(raw: &str) -> Option<(&str, bool, bool)> {
    let tag = raw.strip_prefix('<')?.strip_suffix('>')?;
    let is_closing = tag.trim_start().starts_with('/');
    let trimmed = tag.trim();
    let name_part = if is_closing {
        trimmed.strip_prefix('/')?.trim_start()
    } else {
        trimmed
    };
    let name = name_part
        .split(|c: char| c.is_whitespace() || c == '/')
        .next()
        .filter(|value| !value.is_empty())?;
    let is_self_closing = !is_closing && trimmed.ends_with('/');
    Some((name, is_closing, is_self_closing))
}

fn closing_tags_len(open_tags: &[TelegramOpenTag]) -> usize {
    open_tags.iter().map(|tag| tag.close.len()).sum()
}

fn flush_telegram_chunk(
    chunks: &mut Vec<String>,
    current: &mut String,
    open_tags: &[TelegramOpenTag],
    pending_reopen: &mut String,
) {
    if current.is_empty() {
        return;
    }

    let mut chunk = std::mem::take(current);
    for tag in open_tags.iter().rev() {
        chunk.push_str(&tag.close);
    }
    chunks.push(chunk);
    pending_reopen.clear();
    for tag in open_tags {
        pending_reopen.push_str(&tag.raw);
    }
}

fn is_closing_only_chunk(chunk: &str) -> bool {
    tokenize_telegram_html(chunk).into_iter().all(|token| match token {
        TelegramHtmlToken::Tag { is_closing, .. } => is_closing,
        TelegramHtmlToken::Text(text) => text.trim().is_empty(),
    })
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

        let msg = parse_telegram_update_metadata(&update, &[], &[], None).unwrap();
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

        let msg = parse_telegram_update_metadata(&update, &[], &[], None).unwrap();
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
        let msg = parse_telegram_update_metadata(&update, &[], &[], None);
        assert!(msg.is_some());

        // Non-matching allowed_users = filter out
        let msg = parse_telegram_update_metadata(
            &update,
            &["111".to_string(), "222".to_string()],
            &[],
            None,
        );
        assert!(msg.is_none());

        // Matching allowed_users = allow
        let msg = parse_telegram_update_metadata(&update, &["999".to_string()], &[], None);
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

        let msg = parse_telegram_update_metadata(&update, &[], &[], None).unwrap();
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

        let msg = parse_telegram_update_metadata(&update, &[], &[], None).unwrap();
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

        let msg =
            parse_telegram_update_metadata(&update, &[], &["-1001234567890".to_string()], None)
                .unwrap();
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

    #[test]
    fn test_split_telegram_html_chunks_balances_tags() {
        let input = "<b>1234567890\nabcdefghij</b>";
        let chunks = split_telegram_html_chunks(input, 18);
        assert_eq!(
            chunks,
            vec![
                "<b>1234567890</b>".to_string(),
                "<b>abcdefghij</b>".to_string()
            ]
        );
        assert!(chunks.iter().all(|chunk| chunk.len() <= 18));
    }

    #[test]
    fn test_split_telegram_html_chunks_escapes_unknown_tags() {
        let chunks = split_telegram_html_chunks("hello <thinking>world</thinking>", 4096);
        assert_eq!(chunks, vec!["hello &lt;thinking&gt;world&lt;/thinking&gt;".to_string()]);
    }

    #[test]
    fn test_split_telegram_html_chunks_preserves_code_block_wrappers() {
        let input = "<pre><code>line1\nline2\nline3</code></pre>";
        let chunks = split_telegram_html_chunks(input, 30);
        assert_eq!(
            chunks,
            vec![
                "<pre><code>line1</code></pre>".to_string(),
                "<pre><code>line2</code></pre>".to_string(),
                "<pre><code>line3</code></pre>".to_string(),
            ]
        );
        assert!(chunks.iter().all(|chunk| chunk.len() <= 30));
    }

    #[test]
    fn test_split_telegram_html_chunks_preserves_table_as_preformatted_lines() {
        let input = "<pre><code>| A | B |\n| 1 | 2 |\n| 3 | 4 |</code></pre>";
        let chunks = split_telegram_html_chunks(input, 34);
        assert_eq!(
            chunks,
            vec![
                "<pre><code>| A | B |</code></pre>".to_string(),
                "<pre><code>| 1 | 2 |</code></pre>".to_string(),
                "<pre><code>| 3 | 4 |</code></pre>".to_string(),
            ]
        );
        assert!(chunks.iter().all(|chunk| chunk.len() <= 34));
    }

    #[test]
    fn test_split_telegram_html_chunks_zero_limit_returns_unsplit() {
        let chunks = split_telegram_html_chunks("hello", 0);
        assert_eq!(chunks, vec!["hello".to_string()]);
    }

}
