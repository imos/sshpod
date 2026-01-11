# sshpod

`sshpod` lets you SSH into a Kubernetes Pod from your regular OpenSSH client. It rewrites `~/.ssh/config` to use a ProxyCommand so that `ssh pod--???.context--???.sshpod` starts a pod-local `sshd` via `kubectl` and forwards your session.

## Install

### Option 1: automatic
```bash
curl -fsSL https://raw.githubusercontent.com/imos/sshpod/main/install.sh | sh -s -- --yes
```

### Option 2: manual
- Download `sshpod` from the release page and put it on your PATH.
- Add this to `~/.ssh/config` (the installer writes it for you):
```sshconfig
Host *.sshpod
  ProxyCommand sshpod proxy --host %h --user %r --port %p
  StrictHostKeyChecking no
  UserKnownHostsFile /dev/null
  GlobalKnownHostsFile /dev/null
  CheckHostIP no
  IdentityFile ~/.cache/sshpod/id_ed25519
  IdentitiesOnly yes
  BatchMode yes
  ForwardAgent yes
```

## Usage
```bash
ssh root@deployment--<deployment>.namespace--<namespace>.context--<context>.sshpod
```
- `context` is required.
- `namespace` is optional; when omitted, the default namespace of the context is used.
- `container--<container>.` prefix is needed only for multi-container Pods.
- Resource types: `pod--...`, `deployment--...`, `job--...`.
- The SSH user must match the user inside the container.

## Requirements
- Local: `kubectl` configured for the target cluster; OpenSSH client (`ssh`, `scp`, `sftp`).
- Pod side: `amd64` or `arm64`, `tar` available, `/tmp` writable. `xz` is not required; sshpod will fall back to gzip/plain if needed.

## Developer notes

Update ~/.ssh/config with the ProxyCommand block, and build/install binary and bundles under ~/.local:

```bash
make install
```
