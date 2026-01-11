use crate::kubectl;
use anyhow::{bail, Context, Result};

pub async fn try_acquire_lock(
    context: Option<&str>,
    namespace: &str,
    pod: &str,
    container: &str,
    base: &str,
) {
    let lock_cmd = format!("umask 077; mkdir \"{}/lock\"", base);
    let _ = kubectl::exec_capture_optional(
        context,
        namespace,
        pod,
        container,
        &["sh", "-c", &lock_cmd],
    )
    .await;
}

pub async fn assert_login_user_allowed(
    context: Option<&str>,
    namespace: &str,
    pod: &str,
    container: &str,
    login_user: &str,
) -> Result<()> {
    let uid = kubectl::exec_capture(context, namespace, pod, container, &["id", "-u"])
        .await
        .context("failed to read remote uid")?;
    if uid.trim() == "0" {
        return Ok(());
    }
    let remote_user = kubectl::exec_capture(context, namespace, pod, container, &["id", "-un"])
        .await
        .context("failed to read remote user")?;
    if remote_user.trim() != login_user {
        bail!(
            "This Pod runs as non-root. Use the container user for login (requested: {}, required: {}).",
            login_user,
            remote_user.trim()
        );
    }
    Ok(())
}

pub async fn ensure_sshd_running(
    context: Option<&str>,
    namespace: &str,
    pod: &str,
    container: &str,
    base: &str,
    login_user: &str,
    pubkey_line: &str,
) -> Result<u16> {
    let script = START_SSHD_SCRIPT.as_bytes();
    let output = kubectl::exec_with_input(
        context,
        namespace,
        pod,
        container,
        &["sh", "-s", "--", base, login_user, pubkey_line],
        script,
    )
    .await
    .with_context(|| format!("failed to start sshd under {}", base))?;

    let port: u16 = output
        .trim()
        .parse()
        .with_context(|| format!("unexpected sshd port output: {}", output))?;
    Ok(port)
}

const START_SSHD_SCRIPT: &str = r#"#!/bin/sh
set -eu

BASE="$1"
LOGIN_USER="$2"
PUBKEY_LINE="$3"
SSHD="$BASE/bundle/sshd"
SSHKEYGEN="$BASE/bundle/ssh-keygen"
ENV_FILE="$BASE/environment"

umask 077
mkdir -p "$BASE" "$BASE/hostkeys" "$BASE/logs"
chmod 700 "$BASE" "$BASE/hostkeys" "$BASE/logs"
BASE_PARENT="$(dirname "$BASE")"
TOP_DIR="$(dirname "$BASE_PARENT")"
chmod 711 "$TOP_DIR" "$BASE_PARENT"

if [ ! -f "$BASE/authorized_keys" ]; then
  : > "$BASE/authorized_keys"
fi
grep -qxF "$PUBKEY_LINE" "$BASE/authorized_keys" || printf '%s\n' "$PUBKEY_LINE" >> "$BASE/authorized_keys"
chmod 600 "$BASE/authorized_keys"
if [ -n "$LOGIN_USER" ]; then
  chown "$LOGIN_USER":"$LOGIN_USER" "$BASE" "$BASE/authorized_keys" 2>/dev/null || true
fi

mkdir -p /tmp/empty
chmod 755 /tmp/empty

if [ ! -f "$BASE/hostkeys/ssh_host_ed25519_key" ]; then
  "$SSHKEYGEN" -t ed25519 -f "$BASE/hostkeys/ssh_host_ed25519_key" -N '' >/dev/null
fi
chmod 600 "$BASE/hostkeys/"*

if [ -f "$BASE/sshd.pid" ] && kill -0 "$(cat "$BASE/sshd.pid")" 2>/dev/null && [ -f "$BASE/sshd.port" ]; then
  cat "$BASE/sshd.port"
  exit 0
fi

rand_port() {
  val="$(od -An -N2 -tu2 /dev/urandom | tr -d ' ')"
  echo $((20000 + (val % 45000)))
}

REMOTE_PATH="${PATH:-/usr/bin:/bin}"
ENV_EXPORTS="$(env | awk -F= '/^KUBERNETES_/ {print $1}')"
USER_HOME="$(getent passwd "$LOGIN_USER" 2>/dev/null | awk -F: '{print $6}')"

i=0
while [ $i -lt 30 ]; do
  i=$((i+1))
  PORT="$(rand_port)"

  if [ -f "$BASE/bundle/sshd_config.in" ]; then
    sed -e "s|__BASE__|$BASE|g" -e "s|__PORT__|$PORT|g" "$BASE/bundle/sshd_config.in" > "$BASE/sshd_config"
  else
    cat > "$BASE/sshd_config" <<EOF
ListenAddress 127.0.0.1
Port $PORT
HostKey $BASE/hostkeys/ssh_host_ed25519_key
PidFile $BASE/sshd.pid
AuthorizedKeysFile $BASE/authorized_keys
PubkeyAuthentication yes
StrictModes no
PasswordAuthentication no
KbdInteractiveAuthentication no
ChallengeResponseAuthentication no
PermitEmptyPasswords no
AllowAgentForwarding yes
AllowTcpForwarding yes
X11Forwarding no
Subsystem sftp internal-sftp
LogLevel VERBOSE
PermitUserEnvironment yes
EOF
  fi

  printf 'SetEnv PATH=%s\n' "$REMOTE_PATH" >> "$BASE/sshd_config"
  for key in $ENV_EXPORTS; do
    val="$(printenv "$key" || true)"
    printf 'SetEnv %s=%s\n' "$key" "$val" >> "$BASE/sshd_config"
  done
  if [ -n "${KUBECONFIG:-}" ]; then
    printf 'SetEnv KUBECONFIG=%s\n' "$KUBECONFIG" >> "$BASE/sshd_config"
  fi
  if [ -n "$USER_HOME" ] && [ -d "$USER_HOME" ]; then
    mkdir -p "$USER_HOME/.ssh"
    {
      printf 'PATH=%s\n' "$REMOTE_PATH"
      for key in $ENV_EXPORTS; do
        val="$(printenv "$key" || true)"
        printf '%s=%s\n' "$key" "$val"
      done
      if [ -n "${KUBECONFIG:-}" ]; then
        printf 'KUBECONFIG=%s\n' "$KUBECONFIG"
      fi
    } > "$USER_HOME/.ssh/environment"
    chmod 700 "$USER_HOME/.ssh"
    chmod 600 "$USER_HOME/.ssh/environment"
    if [ -n "$LOGIN_USER" ]; then
      chown "$LOGIN_USER":"$LOGIN_USER" "$USER_HOME/.ssh" "$USER_HOME/.ssh/environment" 2>/dev/null || true
    fi
  fi

  {
    printf 'PATH=%s\n' "$REMOTE_PATH"
    for key in $ENV_EXPORTS; do
      val="$(printenv "$key" || true)"
      printf '%s=%s\n' "$key" "$val"
    done
    if [ -n "${KUBECONFIG:-}" ]; then
      printf 'KUBECONFIG=%s\n' "$KUBECONFIG"
    fi
  } > "$ENV_FILE"
  chmod 600 "$ENV_FILE"
  if [ -n "$LOGIN_USER" ]; then
    chown "$LOGIN_USER":"$LOGIN_USER" "$ENV_FILE" 2>/dev/null || true
  fi

  chmod 600 "$BASE/sshd_config"
  rm -f "$BASE/sshd.pid"
  "$SSHD" -f "$BASE/sshd_config" -E "$BASE/logs/sshd.log" </dev/null >/dev/null 2>&1 || true
  j=0
  while [ $j -lt 10 ]; do
    if [ -f "$BASE/sshd.pid" ] && kill -0 "$(cat "$BASE/sshd.pid")" 2>/dev/null; then
      echo "$PORT" > "$BASE/sshd.port"
      chmod 600 "$BASE/sshd.pid" "$BASE/sshd.port"
      echo "$PORT"
      exit 0
    fi
    j=$((j+1))
    sleep 1
  done
done

echo "sshd did not start" >&2
exit 1
"#;
