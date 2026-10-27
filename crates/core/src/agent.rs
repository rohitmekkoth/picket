//! Agent registration rules: stable ids, normalized handles and bounded message snippets.

use thiserror::Error;

use crate::event::{AgentMode, AgentRegistration};

/// The design's maximum stored snippet; message text itself is never stored.
pub const SNIPPET_CHARS: usize = 120;
const MAX_HANDLES: usize = 8;

#[derive(Debug, Error, Eq, PartialEq)]
pub enum AgentError {
    #[error("agent id must be 1-32 lower-case letters, digits or hyphens")]
    Id,
    #[error("agent name must be 1-64 printable characters")]
    Name,
    #[error("enter a phone number with country code, or an email address")]
    Handle,
    #[error("a Messages agent needs one to eight handles")]
    Handles,
}

/// Ids appear in coverage capability names, so they stay short and plain.
pub fn valid_id(id: &str) -> bool {
    (1..=32).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !id.starts_with('-')
}

/// Normalizes a user-entered or Messages handle. Phone numbers become E.164 using
/// `region` (an ISO 3166 code) when no country code is given; emails are lower-cased.
/// The same function normalizes both sides, so matching never compares raw formats.
pub fn normalize_handle(input: &str, region: Option<&str>) -> Result<String, AgentError> {
    let input = input.trim();
    if input.is_empty() || input.len() > 254 || input.chars().any(char::is_control) {
        return Err(AgentError::Handle);
    }
    if let Some((local, domain)) = input.split_once('@') {
        let valid = !local.is_empty()
            && domain.contains('.')
            && !domain.starts_with('.')
            && !domain.ends_with('.')
            && !input.chars().any(char::is_whitespace);
        return valid
            .then(|| input.to_lowercase())
            .ok_or(AgentError::Handle);
    }
    let region = region
        .map(|value| value.to_ascii_uppercase().parse())
        .transpose()
        .map_err(|_| AgentError::Handle)?;
    let number = phonenumber::parse(region, input).map_err(|_| AgentError::Handle)?;
    if !phonenumber::is_valid(&number) {
        return Err(AgentError::Handle);
    }
    Ok(number.format().mode(phonenumber::Mode::E164).to_string())
}

pub fn validate(agent: &AgentRegistration) -> Result<(), AgentError> {
    if !valid_id(&agent.id) {
        return Err(AgentError::Id);
    }
    if agent.name.trim().is_empty()
        || agent.name.chars().count() > 64
        || agent.name.chars().any(char::is_control)
    {
        return Err(AgentError::Name);
    }
    if agent.region.as_ref().is_some_and(|region| {
        region.len() != 2 || !region.bytes().all(|byte| byte.is_ascii_uppercase())
    }) {
        return Err(AgentError::Handle);
    }
    let needs_handles = agent.mode == AgentMode::Messages;
    if agent.handles.len() > MAX_HANDLES || (needs_handles && agent.handles.is_empty()) {
        return Err(AgentError::Handles);
    }
    for handle in &agent.handles {
        // Stored handles must already be in normal form.
        if normalize_handle(handle, None).as_deref() != Ok(handle.as_str()) {
            return Err(AgentError::Handle);
        }
    }
    Ok(())
}

/// A stable id from a display name: lower-case letters and digits joined by hyphens.
pub fn id_from_name(name: &str) -> Result<String, AgentError> {
    let id = name
        .to_lowercase()
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let id: String = id.chars().take(32).collect();
    let id = id.trim_end_matches('-').to_owned();
    valid_id(&id).then_some(id).ok_or(AgentError::Name)
}

/// The first characters of a message, on one line, for display only.
pub fn snippet(text: &str) -> String {
    text.split_whitespace()
        .flat_map(|word| std::iter::once(' ').chain(word.chars()))
        .skip(1)
        .take(SNIPPET_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_normalize_to_one_form() {
        assert_eq!(
            normalize_handle("(415) 555-0132", Some("us")),
            Ok("+14155550132".into())
        );
        assert_eq!(
            normalize_handle("+1 415 555 0132", None),
            Ok("+14155550132".into())
        );
        assert_eq!(
            normalize_handle(" Agent@Example.TEST ", None),
            Ok("agent@example.test".into())
        );
        assert_eq!(
            normalize_handle("415 555 0132", None),
            Err(AgentError::Handle),
            "a national number without a region is ambiguous"
        );
        assert_eq!(
            normalize_handle("not a handle", None),
            Err(AgentError::Handle)
        );
        assert_eq!(normalize_handle("a@b", None), Err(AgentError::Handle));
    }

    #[test]
    fn registration_requires_valid_id_name_and_normalized_handles() {
        let mut agent = AgentRegistration {
            id: "instinct".into(),
            name: "Instinct".into(),
            mode: AgentMode::Messages,
            handles: vec!["+14155550132".into()],
            connected_since: None,
            enabled: true,
            region: Some("US".into()),
        };
        assert_eq!(validate(&agent), Ok(()));
        agent.handles = vec!["(415) 555-0132".into()];
        assert_eq!(validate(&agent), Err(AgentError::Handle));
        agent.handles.clear();
        assert_eq!(validate(&agent), Err(AgentError::Handles));
        agent.mode = AgentMode::EffectsOnly;
        assert_eq!(validate(&agent), Ok(()));
        agent.id = "Bad Id".into();
        assert_eq!(validate(&agent), Err(AgentError::Id));
    }

    #[test]
    fn ids_come_from_names() {
        assert_eq!(id_from_name("Instinct"), Ok("instinct".into()));
        assert_eq!(id_from_name("  My Agent #2 "), Ok("my-agent-2".into()));
        assert_eq!(id_from_name("!!!"), Err(AgentError::Name));
    }

    #[test]
    fn snippets_are_single_line_and_bounded() {
        assert_eq!(
            snippet("  Book\n a table \t for two "),
            "Book a table for two"
        );
        assert_eq!(snippet(&"x".repeat(500)).chars().count(), SNIPPET_CHARS);
    }
}
