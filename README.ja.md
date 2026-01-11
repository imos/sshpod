# sshpod

`sshpod` は、手元の OpenSSH クライアントから Kubernetes Pod に SSH できるようにするツールです。`~/.ssh/config` の ProxyCommand を書き換え、`ssh pod--...context--....sshpod` の接続時に `kubectl` 経由で Pod 内に `sshd` を起動・転送します。

## インストール

### 方法1: 自動
```bash
curl -fsSL https://raw.githubusercontent.com/imos/sshpod/main/install.sh | sh -s -- --yes
```

### 方法2: 手動
- リリースから `sshpod` をダウンロードして PATH に置く。
- `~/.ssh/config` に次を追記（インストーラが自動で書き込みます）:
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

## 使い方
```bash
ssh root@deployment--<deployment>.namespace--<namespace>.context--<context>.sshpod
```
- `<context>` は必須。
- `<namespace>` は省略可能（省略時はコンテキストのデフォルト namespace を使用）。
- 複数コンテナ Pod では `container--<container>.` を先頭に付与。
- リソース種別: `pod--...` / `deployment--...` / `job--...`。
- SSH ユーザはコンテナ内の実ユーザと一致している必要があります。

## 要件
- ローカル: 対象クラスタに到達できる `kubectl`、OpenSSH クライアント (`ssh`/`scp`/`sftp`)。
- Pod 側: `amd64` または `arm64`、`tar` が利用可能、`/tmp` が書き込み可。`xz` が無くても sshpod が gzip/プレーン転送にフォールバックします。

## 開発メモ

ProxyCommand ブロックを ~/.ssh/config に書き込み、sshpod を ~/.local にインストールします。

```bash
make install
```
