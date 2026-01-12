use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostSpec {
    pub context: Option<String>,
    pub namespace: Option<String>,
    pub target: Target,
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
        "hostname must include one of pod--/deployment--/job-- (container-- optional, namespace-- optional, context-- optional), ending with .sshpod"
    )]
    InvalidFormat,
}

pub fn parse(host: &str) -> Result<HostSpec, HostSpecError> {
    let trimmed = host.trim_end_matches('.');
    let without_suffix = trimmed
        .strip_suffix(".sshpod")
        .ok_or(HostSpecError::MissingSuffix)?;

    let mut container = None;
    let mut namespace = None;
    let mut context = None;
    let mut target = None;

    for token in without_suffix.split('.').filter(|s| !s.is_empty()) {
        if let Some(rest) = token.strip_prefix("container--") {
            if rest.is_empty() || container.is_some() {
                return Err(HostSpecError::InvalidFormat);
            }
            container = Some(rest.to_string());
            continue;
        }
        if let Some(rest) = token.strip_prefix("namespace--") {
            if rest.is_empty() || namespace.is_some() {
                return Err(HostSpecError::InvalidFormat);
            }
            namespace = Some(rest.to_string());
            continue;
        }
        if let Some(rest) = token.strip_prefix("context--") {
            if rest.is_empty() || context.is_some() {
                return Err(HostSpecError::InvalidFormat);
            }
            context = Some(rest.to_string());
            continue;
        }
        if target.is_none() {
            target = Some(parse_target(token)?);
            continue;
        }
        return Err(HostSpecError::InvalidFormat);
    }

    let target = target.ok_or(HostSpecError::InvalidFormat)?;

    Ok(HostSpec {
        target,
        namespace,
        context,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pod_with_namespace_and_context() {
        let spec =
            parse("pod--app.namespace--ns.context--ctx.sshpod").expect("should parse successfully");
        assert_eq!(spec.target, Target::Pod("app".into()));
        assert_eq!(spec.namespace.as_deref(), Some("ns"));
        assert_eq!(spec.context.as_deref(), Some("ctx"));
        assert!(spec.container.is_none());
    }

    #[test]
    fn parse_pod_with_context_only_namespace_defaults() {
        let spec = parse("pod--app.context--ctx.sshpod").expect("should parse successfully");
        assert_eq!(spec.target, Target::Pod("app".into()));
        assert!(spec.namespace.is_none());
        assert_eq!(spec.context.as_deref(), Some("ctx"));
    }

    #[test]
    fn parse_deployment_with_container_prefix() {
        let spec = parse("container--web.deployment--api.namespace--ns.context--ctx.sshpod")
            .expect("should parse successfully");
        assert_eq!(spec.target, Target::Deployment("api".into()));
        assert_eq!(spec.container.as_deref(), Some("web"));
        assert_eq!(spec.namespace.as_deref(), Some("ns"));
        assert_eq!(spec.context.as_deref(), Some("ctx"));
    }

    #[test]
    fn reject_missing_context() {
        let err = parse("pod--app.namespace--ns.sshpod").unwrap_err();
        assert!(matches!(err, HostSpecError::InvalidFormat));
    }

    #[test]
    fn reject_missing_suffix() {
        let err = parse("pod--app.context--ctx").unwrap_err();
        assert!(matches!(err, HostSpecError::MissingSuffix));
    }
}
