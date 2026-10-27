# yesy

yesy は、ローカルファイルの名前とテキストの内容を検索する Rust 製のツールです。SQLite に索引を保存し、ブラウザから検索できます。外部の検索エンジンは使いません。

Linux と WSL on Windows に対応します。WSL では検索結果に Linux パスと Windows の UNC パスを併記します。

## セットアップ

Rust のツールチェーンを用意し、次のコマンドを実行します。

```console
cargo install --git https://github.com/cympfh/yesy
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

コマンドラインで検索するには `search` を実行します。複数の検索語を指定すると、すべての検索語を含むファイルを検索します。結果のパスを1行1件で表示します。データベースがなければ、検索前に索引を作成します。

```console
yesy search キーワード
yesy search キーワード1 キーワード2
```

Web サーバを起動するには `start` を実行します。データベースがなければ、起動時に索引を作成します。

```console
yesy start
```

起動前に索引を更新するには `--update` を指定します。

```console
yesy start --update
```

デフォルトでは `http://0.0.0.0:8080` で待ち受けます。アドレスとポートは変更できます。

```console
yesy start --listen 127.0.0.1 --port 8080
```

`0.0.0.0` では、同じネットワーク上のほかの端末からも検索画面へアクセスできます。検索対象の情報を公開しないよう、利用するネットワークを確認してください。

リクエストごとに、HTTPメソッド、URL、ステータスコード、処理時間を標準エラーへ出力します。サーバを止めるには Ctrl-C を押します。

## 検索と索引

- ファイル名と、UTF-8 テキストとして読めるファイルの内容を検索します。
- Web 画面では、半角または全角の空白で検索語を区切ると AND 検索になります。空白は複数続けて入力できます。
- バイナリファイルはファイル名だけを検索できます。
- 10 MiB を超えるファイルは内容を索引せず、ファイル名だけを登録します。
- `update` は進捗と残り時間の目安を表示します。変更のあったファイルだけを索引へ反映し、前回以降に削除したファイルも結果から消します。
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

WSL では、たとえば `/home/user/note.txt` の検索結果に `\\wsl.localhost\Ubuntu\home\user\note.txt` を併記します。`/mnt/c`、`/mnt/d`、`/mnt/e`、`/mnt/f` 以下のファイルは、UNC パスよりも `C:\`、`D:\`、`E:\`、`F:\` から始まる Windows のドライブパスを優先します。

各パスの右にあるクリップボードアイコンをクリックすると、そのパスをコピーできます。Windows のパスは、エクスプローラーのアドレスバーに貼り付けて開けます。Windows のブラウザでは、HTTP ページから `file:` URL を直接開く操作が制限されるためです。

Windows パスがある検索結果では、`Open with` から NeeViewer または Zed でファイルを開けます。Windows 側で `NeeView.exe` または `zed.exe` を実行できる状態にしてください。

## 開発

```console
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```
