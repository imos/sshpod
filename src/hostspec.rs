use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostSpec {
    pub pod: String,
    pub namespace_hint: String,
    pub container: Option<String>,
}

#[derive(Debug, Error)]
pub enum HostSpecError {
    #[error("hostname must end with .sshpod")]
    MissingSuffix,
    #[error("hostname must be <pod>.<namespace>.sshpod or <container>.<pod>.<namespace>.sshpod")]
    InvalidFormat,
}

pub fn parse(host: &str) -> Result<HostSpec, HostSpecError> {
    let trimmed = host.trim_end_matches('.');
    let without_suffix = trimmed
        .strip_suffix(".sshpod")
        .ok_or(HostSpecError::MissingSuffix)?;

    let parts = without_suffix.split('.').collect::<Vec<_>>();
    let (pod, namespace_hint, container) = match parts.as_slice() {
        [pod, ns] if !pod.is_empty() && !ns.is_empty() => (*pod, *ns, None),
        [container, pod, ns] if !container.is_empty() && !pod.is_empty() && !ns.is_empty() => {
            (*pod, *ns, Some((*container).to_string()))
        }
        _ => return Err(HostSpecError::InvalidFormat),
    };

    Ok(HostSpec {
        pod: pod.to_string(),
        namespace_hint: namespace_hint.to_string(),
        container,
    })
}
