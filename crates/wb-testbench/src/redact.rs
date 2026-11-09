//! The rules of scripts/lib/Redact.psm1, for reports this tool writes into docs/evidence: the
//! user profile path, user name and computer name are replaced, and NAME=value secrets lose
//! their value.

fn replace_word(text: &str, word: &str, with: &str) -> String {
    if word.len() < 2 {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    // ASCII lowering keeps byte offsets identical between `lower` and `text`.
    let lower = text.to_ascii_lowercase();
    let w = word.to_ascii_lowercase();
    let mut i = 0;
    while let Some(pos) = lower[i..].find(&w) {
        let start = i + pos;
        let end = start + w.len();
        let before_ok = start == 0
            || !lower[..start]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_');
        let after_ok = end >= lower.len()
            || !lower[end..]
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric() || c == '_');
        out.push_str(&text[i..start]);
        if before_ok && after_ok {
            out.push_str(with);
        } else {
            out.push_str(&text[start..end]);
        }
        i = end;
    }
    out.push_str(&text[i..]);
    out
}

fn redact_secrets(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (n, line) in text.split('\n').enumerate() {
        if n > 0 {
            out.push('\n');
        }
        let mut rest = line;
        while let Some(eq) = rest.find('=') {
            let name_start = rest[..eq]
                .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .map_or(0, |p| p + 1);
            let name = &rest[name_start..eq].to_ascii_uppercase();
            out.push_str(&rest[..=eq]);
            let after = &rest[eq + 1..];
            if ["KEY", "TOKEN", "SECRET", "PASSWORD"]
                .iter()
                .any(|s| name.ends_with(s))
            {
                let end = after.find(char::is_whitespace).unwrap_or(after.len());
                out.push_str("<redacted>");
                rest = &after[end..];
            } else {
                rest = after;
            }
        }
        out.push_str(rest);
    }
    out
}

/// Applies the evidence redaction rules with the given identity values.
pub fn redact_with(
    text: &str,
    profile: Option<&str>,
    user: Option<&str>,
    computer: Option<&str>,
) -> String {
    let mut s = text.to_string();
    if let Some(p) = profile.filter(|p| p.len() > 3) {
        s = s.replace(p, "%USERPROFILE%");
        // JSON escapes backslashes.
        s = s.replace(&p.replace('\\', "\\\\"), "%USERPROFILE%");
    }
    if let Some(u) = user {
        s = replace_word(&s, u, "%USERNAME%");
    }
    if let Some(c) = computer {
        s = replace_word(&s, c, "%COMPUTERNAME%");
    }
    redact_secrets(&s)
}

/// Redacts with this machine's USERPROFILE, USERNAME and COMPUTERNAME.
pub fn redact(text: &str) -> String {
    let p = std::env::var("USERPROFILE").ok();
    let u = std::env::var("USERNAME").ok();
    let c = std::env::var("COMPUTERNAME").ok();
    redact_with(text, p.as_deref(), u.as_deref(), c.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules() {
        let t = "C:\\Users\\asha\\x and \"C:\\\\Users\\\\asha\\\\y\" asha ashar LEGION-1 API_KEY=abc OTHER=1";
        let r = redact_with(t, Some("C:\\Users\\asha"), Some("asha"), Some("LEGION-1"));
        assert_eq!(
            r,
            "%USERPROFILE%\\x and \"%USERPROFILE%\\\\y\" %USERNAME% ashar %COMPUTERNAME% API_KEY=<redacted> OTHER=1"
        );
    }
}
