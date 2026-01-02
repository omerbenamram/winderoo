//! Wi‑Fi captive portal helpers (host-testable).
//!
//! The ESP32 runtime uses a tiny HTTP server while in AP mode to collect
//! Wi‑Fi credentials. This module holds the parsing and HTML so it can be
//! unit-tested on a host.

/// Parse the captive-portal POST body into `(ssid, password)`.
///
/// Supports both:
/// - `application/x-www-form-urlencoded` (from the built-in HTML form)
/// - JSON `{"ssid":"...","password":"..."}`
pub fn parse_wifi_payload(body: &str) -> Option<(String, String)> {
    let body = body.trim();
    if body.is_empty() {
        return None;
    }

    if body.starts_with('{') {
        // JSON (best-effort)
        let value: serde_json::Value = serde_json::from_str(body).ok()?;
        let ssid = value.get("ssid")?.as_str()?.to_string();
        let password = value
            .get("password")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        return Some((ssid, password));
    }

    // Form body: ssid=...&password=...
    let mut ssid: Option<String> = None;
    let mut password: Option<String> = None;
    for pair in body.split('&') {
        let mut it = pair.splitn(2, '=');
        let key = it.next().unwrap_or("");
        let value = it.next().unwrap_or("");
        let decoded = url_decode(value);
        match key {
            "ssid" => ssid = Some(decoded),
            "password" => password = Some(decoded),
            _ => {}
        }
    }

    ssid.map(|s| (s, password.unwrap_or_default()))
}

fn url_decode(value: &str) -> String {
    // Minimal x-www-form-urlencoded decoder: '+' => space, %XX hex decoding.
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '+' => out.push(' '),
            '%' => {
                let a = chars.next();
                let b = chars.next();
                if let (Some(a), Some(b)) = (a, b) {
                    if let Ok(byte) = u8::from_str_radix(&format!("{a}{b}"), 16) {
                        out.push(byte as char);
                        continue;
                    }
                }
                // Bad escape: keep '%' and whatever we consumed.
                out.push('%');
                if let Some(a) = a {
                    out.push(a);
                }
                if let Some(b) = b {
                    out.push(b);
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// The captive portal HTML page served in AP mode.
pub fn config_portal_page() -> &'static str {
    r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width,initial-scale=1" />
  <title>Winderoo WiFi Setup</title>
  <style>
    body { font-family: system-ui, -apple-system, sans-serif; margin: 2rem; }
    label { display: block; margin-top: 1rem; }
    input { width: 100%; padding: .6rem; font-size: 1rem; }
    button { margin-top: 1.2rem; padding: .8rem 1rem; font-size: 1rem; }
    .hint { color: #666; font-size: .9rem; }
  </style>
</head>
<body>
  <h2>Winderoo WiFi Setup</h2>
  <p class="hint">Enter your WiFi credentials to connect this device.</p>
  <form method="post" action="/wifi">
    <label>SSID
      <input name="ssid" autocomplete="off" required />
    </label>
    <label>Password
      <input name="password" type="password" autocomplete="off" />
    </label>
    <button type="submit">Save &amp; Restart</button>
  </form>
</body>
</html>"#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_form_body() {
        let parsed = parse_wifi_payload("ssid=MyNet&password=secret").unwrap();
        assert_eq!(parsed.0, "MyNet");
        assert_eq!(parsed.1, "secret");
    }

    #[test]
    fn parse_form_body_decodes_plus_and_percent() {
        let parsed = parse_wifi_payload("ssid=My+Net%21&password=").unwrap();
        assert_eq!(parsed.0, "My Net!");
        assert_eq!(parsed.1, "");
    }

    #[test]
    fn parse_json_body() {
        let parsed = parse_wifi_payload(r#"{"ssid":"Net","password":"pw"}"#).unwrap();
        assert_eq!(parsed.0, "Net");
        assert_eq!(parsed.1, "pw");
    }
}

