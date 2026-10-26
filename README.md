# yesy

yesy は、ローカルファイルの名前とテキストの内容を検索する Rust 製のツールです。SQLite に索引を保存し、ブラウザから検索できます。外部の検索エンジンは使いません。

Linux と WSL on Windows に対応します。WSL では検索結果に Linux パスと Windows の UNC パスを併記します。

## セットアップ

Rust のツールチェーンを用意し、リポジトリのルートで次のコマンドを実行します。

```console
cargo install --path .
```

`~/.config/yesy.toml` に検索対象のディレクトリを設定します。

```toml
directories = [
  "~/Documents",
  "~/projects",
]

ignore = [
  ".git/",
  "node_modules/",
  "target/",
  "*.log",
]
```

相対パスは設定ファイルのあるディレクトリからのパスとして扱います。`directories` には少なくとも 1 つのディレクトリが必要です。

## 使い方

索引を作り直すには `update` を実行します。

```console
yesy update
```

Web サーバを起動するには `start` を実行します。データベースがなければ、起動時に索引を作成します。

```console
yesy start
```

デフォルトでは `http://127.0.0.1:8080` で待ち受けます。アドレスとポートは変更できます。

```console
yesy start --listen 0.0.0.0 --port 8080
```

`0.0.0.0` を指定すると、同じネットワーク上のほかの端末からも検索画面へアクセスできます。検索対象の情報を公開しないよう、利用するネットワークを確認してください。

サーバを止めるには Ctrl-C を押します。

## 検索と索引

- ファイル名と、UTF-8 テキストとして読めるファイルの内容を検索します。
- バイナリファイルはファイル名だけを検索できます。
- 10 MiB を超えるファイルは内容を索引せず、ファイル名だけを登録します。
- `update` は索引を再構築するため、前回以降に削除したファイルも結果から消えます。
- データベースは `~/.local/share/yesy/yesy.db` です。XDG のデータディレクトリを設定している場合は、その下の `yesy/yesy.db` です。

`ignore` では空行と `#` で始まるコメントを使えます。次の gitignore に近い記法に対応します。

| 記法 | 意味 |
| --- | --- |
| `*.log` | 全ディレクトリで名前が一致するファイルを無視する |
| `cache/` | `cache` ディレクトリ以下を無視する |
| `/notes/private.txt` | 検索対象ディレクトリの直下にあるファイルだけを無視する |
| `!important.log` | 直前までの規則で無視した `important.log` を再び対象にする |

`*`、`?`、`**` のワイルドカードを使えます。規則は設定した順に評価します。

## WSL のパス

WSL では、たとえば `/home/user/note.txt` の検索結果に `\\wsl.localhost\Ubuntu\home\user\note.txt` を併記します。Windows のブラウザでは、このパスが `file:` URL のリンクになります。ブラウザやセキュリティ設定によっては、HTTP ページから `file:` URL を開く操作が制限される場合があります。

## 開発

```console
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```
