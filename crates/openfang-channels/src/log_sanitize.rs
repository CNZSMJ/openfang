/// Shared sanitization helpers for channel/log error messages.
///
/// Replaces known secrets and common token patterns so transport errors can be
/// logged safely without leaking credentials.
pub(crate) fn sanitize_channel_error_for_log(raw: &str, secrets: &[&str]) -> String {
    let mut sanitized = raw.to_string();
    for secret in secrets {
        if !secret.is_empty() {
            sanitized = sanitized.replace(secret, "<redacted>");
        }
    }
    let redacted_bot_path = redact_telegram_bot_token_in_urls(&sanitized);
    let redacted_access = redact_query_param_value(&redacted_bot_path, "access_token");
    let redacted_bot_token = redact_query_param_value(&redacted_access, "bot_token");
    redact_query_param_value(&redacted_bot_token, "token")
}

fn redact_telegram_bot_token_in_urls(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(pos) = rest.find("/bot") {
        out.push_str(&rest[..pos]);
        let after_bot = &rest[pos + 4..];
        let Some(end) = after_bot.find('/') else {
            out.push_str(&rest[pos..]);
            return out;
        };

        let token_candidate = &after_bot[..end];
        if !token_candidate.is_empty() && token_candidate.contains(':') {
            out.push_str("/bot<redacted>");
        } else {
            out.push_str("/bot");
            out.push_str(token_candidate);
        }
        rest = &after_bot[end..];
    }

    out.push_str(rest);
    out
}

fn redact_query_param_value(input: &str, key: &str) -> String {
    let pattern = format!("{key}=");
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0usize;

    while let Some(rel_pos) = input[cursor..].find(&pattern) {
        let start = cursor + rel_pos;
        let is_boundary = start == 0
            || matches!(
                input.as_bytes()[start - 1] as char,
                '?' | '&' | ' ' | '(' | '[' | '{'
            );
        if !is_boundary {
            // Keep scanning after this non-matching occurrence.
            out.push_str(&input[cursor..start + 1]);
            cursor = start + 1;
            continue;
        }

        let value_start = start + pattern.len();
        out.push_str(&input[cursor..value_start]);

        let value_and_tail = &input[value_start..];
        let end = value_and_tail
            .find(|c: char| {
                matches!(
                    c,
                    '&' | ' ' | '"' | '\'' | ')' | ']' | '}' | '\n' | '\r' | '\t'
                )
            })
            .unwrap_or(value_and_tail.len());

        if end > 0 {
            out.push_str("<redacted>");
        }
        cursor = value_start + end;
    }

    out.push_str(&input[cursor..]);
    out
}

#[cfg(test)]
mod tests {
    use super::sanitize_channel_error_for_log;

    #[test]
    fn test_sanitize_channel_error_redacts_telegram_bot_token() {
        let raw = "error sending request for url (https://api.telegram.org/bot8618010257:AAHqzdhoa8TVy_th2dwYuhBjN1rwfL5FGuc/sendMessage)";
        let sanitized = sanitize_channel_error_for_log(raw, &[]);
        assert!(sanitized.contains("/bot<redacted>/sendMessage"));
        assert!(!sanitized.contains("8618010257:AAHqzdhoa8TVy_th2dwYuhBjN1rwfL5FGuc"));
    }

    #[test]
    fn test_sanitize_channel_error_keeps_non_secret_bot_path() {
        let raw = "https://api.telegram.org/file/bot123/photos/file_42";
        let sanitized = sanitize_channel_error_for_log(raw, &[]);
        assert_eq!(sanitized, raw);
    }

    #[test]
    fn test_sanitize_channel_error_redacts_token_query_values() {
        let raw = "https://example.com/hook?access_token=abc123&token=xyz&bot_token=qwe";
        let sanitized = sanitize_channel_error_for_log(raw, &[]);
        assert_eq!(
            sanitized,
            "https://example.com/hook?access_token=<redacted>&token=<redacted>&bot_token=<redacted>"
        );
    }

    #[test]
    fn test_sanitize_channel_error_does_not_match_embedded_key_names() {
        let raw = "https://example.com/path?notoken=keep&my_access_token=keep";
        let sanitized = sanitize_channel_error_for_log(raw, &[]);
        assert_eq!(sanitized, raw);
    }

    #[test]
    fn test_sanitize_channel_error_redacts_explicit_secret_literals() {
        let raw = "request failed with bearer abc.secret.xyz";
        let sanitized = sanitize_channel_error_for_log(raw, &["abc.secret.xyz"]);
        assert_eq!(sanitized, "request failed with bearer <redacted>");
    }
}
