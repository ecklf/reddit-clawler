use reqwest::cookie::Jar;
use serde::Deserialize;
use std::{
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserCookie {
    domain: String,
    name: String,
    value: String,
    path: Option<String>,
    host_only: Option<bool>,
    http_only: Option<bool>,
    secure: Option<bool>,
    expiration_date: Option<f64>,
    session: Option<bool>,
}

/// Loads a browser-export JSON or Netscape-format text cookie file into a jar.
pub fn load_cookie_file(path: impl AsRef<Path>) -> Result<Jar, anyhow::Error> {
    let path = path.as_ref();
    let contents = fs::read_to_string(path)
        .map_err(anyhow::Error::from)
        .map_err(|error| {
            anyhow::anyhow!("Failed to open cookie file {}: {error}", path.display())
        })?;
    let jar = Jar::default();
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

    match path.extension().and_then(|extension| extension.to_str()) {
        Some("json") => load_json_cookies(&jar, &contents, path, now)?,
        Some("txt") => load_netscape_cookies(&jar, &contents, path, now)?,
        _ => anyhow::bail!(
            "Cookie file {} must use a .json or .txt extension",
            path.display()
        ),
    }

    Ok(jar)
}

fn load_json_cookies(
    jar: &Jar,
    contents: &str,
    path: &Path,
    now: u64,
) -> Result<(), anyhow::Error> {
    let cookies = serde_json::from_str::<Vec<BrowserCookie>>(contents).map_err(|error| {
        anyhow::anyhow!(
            "Failed to parse JSON cookie file {}: {error}",
            path.display()
        )
    })?;

    for (index, cookie) in cookies.into_iter().enumerate() {
        let expiration = cookie.expiration_date.map(|value| value as u64);
        if expiration.is_some_and(|expires| expires <= now) {
            continue;
        }

        add_cookie(
            jar,
            &cookie.domain,
            !cookie.host_only.unwrap_or(false),
            &cookie.name,
            &cookie.value,
            cookie.path.as_deref().unwrap_or("/"),
            cookie.secure.unwrap_or(false),
            cookie.http_only.unwrap_or(false),
            if cookie.session.unwrap_or(false) {
                None
            } else {
                expiration.map(|expires| expires.saturating_sub(now))
            },
        )
        .map_err(|error| {
            anyhow::anyhow!(
                "Invalid browser cookie entry in {} at index {}: {error}",
                path.display(),
                index + 1
            )
        })?;
    }

    Ok(())
}

fn load_netscape_cookies(
    jar: &Jar,
    contents: &str,
    path: &Path,
    now: u64,
) -> Result<(), anyhow::Error> {
    for (line_number, line) in contents.lines().enumerate() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() || (line.starts_with('#') && !line.starts_with("#HttpOnly_")) {
            continue;
        }

        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() != 7 {
            anyhow::bail!(
                "Invalid Netscape cookie entry in {} at line {}",
                path.display(),
                line_number + 1
            );
        }

        let (domain, http_only) = fields[0]
            .strip_prefix("#HttpOnly_")
            .map(|domain| (domain, true))
            .unwrap_or((fields[0], false));
        let include_subdomains = fields[1].eq_ignore_ascii_case("true");
        let secure = fields[3].eq_ignore_ascii_case("true");
        let expires = fields[4].parse::<u64>().map_err(|_| {
            anyhow::anyhow!(
                "Invalid cookie expiry in {} at line {}",
                path.display(),
                line_number + 1
            )
        })?;
        let name = fields[5];
        let value = fields[6];

        if domain.is_empty()
            || name.is_empty()
            || name.contains(';')
            || name.contains('\r')
            || name.contains('\n')
        {
            anyhow::bail!(
                "Invalid cookie entry in {} at line {}",
                path.display(),
                line_number + 1
            );
        }
        if expires != 0 && expires <= now {
            continue;
        }

        add_cookie(
            &jar,
            domain,
            include_subdomains,
            name,
            value,
            fields[2],
            secure,
            http_only,
            (expires != 0).then(|| expires.saturating_sub(now)),
        )
        .map_err(|error| {
            anyhow::anyhow!(
                "Invalid Netscape cookie entry in {} at line {}: {error}",
                path.display(),
                line_number + 1
            )
        })?;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn add_cookie(
    jar: &Jar,
    domain: &str,
    include_subdomains: bool,
    name: &str,
    value: &str,
    path: &str,
    secure: bool,
    http_only: bool,
    max_age: Option<u64>,
) -> Result<(), anyhow::Error> {
    if domain.is_empty()
        || name.is_empty()
        || name.contains(';')
        || name.contains('\r')
        || name.contains('\n')
    {
        anyhow::bail!("invalid cookie domain or name");
    }

    let host = domain.trim_start_matches('.');
    let url = reqwest::Url::parse(&format!("https://{host}/"))?;
    let mut cookie = format!("{name}={value}; Path={path}");
    if include_subdomains {
        cookie.push_str(&format!("; Domain={host}"));
    }
    if secure {
        cookie.push_str("; Secure");
    }
    if http_only {
        cookie.push_str("; HttpOnly");
    }
    if let Some(max_age) = max_age {
        cookie.push_str(&format!("; Max-Age={max_age}"));
    }

    jar.add_cookie_str(&cookie, &url);
    Ok(())
}
