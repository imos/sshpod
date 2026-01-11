use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostSpec {
    pub target: Target,
    pub namespace: Option<String>,
    pub context: String,
    pub container: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Pod(String),
    Deployment(String),
    Job(String),
}

#[derive(Debug, Error)]
pub enum HostSpecError {
    #[error("hostname must end with .sshpod")]
    MissingSuffix,
    #[error(
        "hostname must be pod--<pod>[.namespace--<namespace>].context--<context>.sshpod (container--<container>. prefix optional) or deployment--/job-- variants"
    )]
    InvalidFormat,
}

pub fn parse(host: &str) -> Result<HostSpec, HostSpecError> {
    let trimmed = host.trim_end_matches('.');
    let without_suffix = trimmed
        .strip_suffix(".sshpod")
        .ok_or(HostSpecError::MissingSuffix)?;

    let parts = without_suffix.split('.').collect::<Vec<_>>();
    let mut idx = 0;
    let mut container = None;
    if let Some(token) = parts.get(idx) {
        if let Some(rest) = token.strip_prefix("container--") {
            if rest.is_empty() {
                return Err(HostSpecError::InvalidFormat);
            }
            container = Some(rest.to_string());
            idx += 1;
        }
    }

    let target_token = parts.get(idx).ok_or(HostSpecError::InvalidFormat)?;
    idx += 1;
    let mut namespace = None;
    if let Some(ns_token) = parts.get(idx) {
        if let Some(rest) = ns_token.strip_prefix("namespace--") {
            namespace = if rest.is_empty() {
                return Err(HostSpecError::InvalidFormat);
            } else {
                Some(rest.to_string())
            };
            idx += 1;
        }
    }
    let context_token = parts.get(idx).ok_or(HostSpecError::InvalidFormat)?;
    idx += 1;
    if idx != parts.len() {
        return Err(HostSpecError::InvalidFormat);
    }

    let context = context_token
        .strip_prefix("context--")
        .ok_or(HostSpecError::InvalidFormat)?;
    if context.is_empty() {
        return Err(HostSpecError::InvalidFormat);
    }

    Ok(HostSpec {
        target: parse_target(target_token)?,
        namespace,
        context: context.to_string(),
        container,
    })
}

fn parse_target(token: &str) -> Result<Target, HostSpecError> {
    if token.is_empty() {
        return Err(HostSpecError::InvalidFormat);
    }
    if let Some(rest) = token.strip_prefix("pod--") {
        if rest.is_empty() {
            return Err(HostSpecError::InvalidFormat);
        }
        return Ok(Target::Pod(rest.to_string()));
    }
    if let Some(rest) = token.strip_prefix("deployment--") {
        if rest.is_empty() {
            return Err(HostSpecError::InvalidFormat);
        }
        return Ok(Target::Deployment(rest.to_string()));
    }
    if let Some(rest) = token.strip_prefix("job--") {
        if rest.is_empty() {
            return Err(HostSpecError::InvalidFormat);
        }
        return Ok(Target::Job(rest.to_string()));
    }
    Ok(Target::Pod(token.to_string()))
}
